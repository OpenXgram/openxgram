//! xgram notify — Discord (webhook 송신 / Gateway 수신) · Telegram bot 양방향.
//!
//! - notify discord/telegram     : 텍스트 송신 (webhook / sendMessage).
//! - notify telegram-listen      : Telegram long-polling 수신.
//! - notify discord-listen       : Discord Gateway WebSocket 수신 (다중 에이전트 채팅방).
//!
//! `--store-session` 옵션으로 받은 메시지를 OpenXgram L0 messages 테이블에 저장
//! (이후 회상·reflection 대상). 두 listen 모두 같은 `StoreCtx` 를 공유한다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use openxgram_adapter::{
    Adapter, ChannelMcpClient, DiscordGatewayClient, DiscordIncomingMessage, DiscordWebhookAdapter,
    TelegramBotAdapter, TelegramUpdate,
};
use openxgram_core::env::require_password;
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keypair, Keystore};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};

const DISCORD_URL_ENV: &str = "DISCORD_WEBHOOK_URL";
const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
const TELEGRAM_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
const TELEGRAM_CHAT_ENV: &str = "TELEGRAM_CHAT_ID";
/// 테스트·self-host 환경에서 Telegram API base 를 교체할 때 사용.
const TELEGRAM_API_BASE_ENV: &str = "TELEGRAM_API_BASE";
/// Starian Channel MCP HTTP gateway base URL (예: http://localhost:7100).
pub const CHANNEL_MCP_URL_ENV: &str = "OPENXGRAM_CHANNEL_MCP_URL";
/// 선택 — channel-mcp gateway 가 bearer 토큰을 요구하는 경우.
pub const CHANNEL_MCP_TOKEN_ENV: &str = "OPENXGRAM_CHANNEL_MCP_TOKEN";

#[derive(Debug, Clone)]
pub enum NotifyAction {
    Discord {
        webhook_url: Option<String>,
        text: String,
    },
    Telegram {
        bot_token: Option<String>,
        chat_id: Option<String>,
        text: String,
    },
    /// Telegram long-polling 으로 받기. 받은 텍스트를 stdout 으로 출력하고,
    /// `store_session_title` 이 있으면 해당 session 에 L0 message 로 저장.
    TelegramListen {
        bot_token: Option<String>,
        /// 지정 시 그 chat 으로 들어온 메시지만 통과.
        chat_id_filter: Option<i64>,
        /// 지정 시 OpenXgram L0 message 로 저장. session title 로 사용.
        store_session_title: Option<String>,
        /// L0 저장 시 사용할 data_dir (None → 기본 ~/.openxgram).
        data_dir: Option<PathBuf>,
        /// 한 번만 polling 후 종료 (테스트·debug 용). 기본 false (Ctrl+C 까지 loop).
        once: bool,
    },
    /// Discord Gateway 봇 — 채널/DM 수신 (WebSocket).
    DiscordListen {
        bot_token: Option<String>,
        /// 특정 channel 만 받기 (없으면 모든 channel + DM).
        channel_id: Option<u64>,
        /// 받은 메시지를 L0 messages 로 저장. session title (없으면 자동 생성).
        store_session: Option<String>,
        /// 데이터 디렉토리 (store_session 사용 시 필요).
        data_dir: Option<PathBuf>,
        /// 사람 친화 출력 (false 면 한 줄 JSON).
        pretty: bool,
    },
    /// Starian Channel MCP HTTP gateway 호출 — 다중 에이전트 메시지 라우팅 허브.
    ///
    /// 세 가지 모드 (배타):
    /// - `ChannelMode::Platform`     : `send_to_platform(platform, channel_id, text, reply_to?)`
    /// - `ChannelMode::Peer`         : `send_message(to_role, summary, msg_type)`
    /// - `ChannelMode::ListAdapters` : `list_adapters()` 결과를 stdout 으로 출력
    Channel {
        /// gateway base URL (생략 시 OPENXGRAM_CHANNEL_MCP_URL 환경변수 — 미설정 시 raise)
        mcp_url: Option<String>,
        /// 선택 bearer 토큰 (생략 시 OPENXGRAM_CHANNEL_MCP_TOKEN)
        auth_token: Option<String>,
        mode: ChannelMode,
    },
}

/// `xgram notify channel` 의 세 가지 호출 모드.
#[derive(Debug, Clone)]
pub enum ChannelMode {
    /// `send_to_platform` 도구 호출 (discord/telegram/slack/kakaotalk/webhook).
    Platform {
        platform: String,
        channel_id: String,
        text: String,
        reply_to: Option<String>,
    },
    /// `send_message` 도구 호출 — 역할명 라우팅.
    Peer {
        to_role: String,
        summary: String,
        msg_type: String,
    },
    /// `list_adapters` 도구 호출.
    ListAdapters,
}

pub async fn run_notify(action: NotifyAction) -> Result<()> {
    match action {
        NotifyAction::Discord { webhook_url, text } => {
            let url = resolve(webhook_url, DISCORD_URL_ENV, "--webhook-url")?;
            DiscordWebhookAdapter::new(url).send_text(&text).await?;
            println!("✓ Discord 전송 완료 ({} chars)", text.chars().count());
        }
        NotifyAction::Telegram {
            bot_token,
            chat_id,
            text,
        } => {
            let token = resolve(bot_token, TELEGRAM_TOKEN_ENV, "--bot-token")?;
            let chat = resolve(chat_id, TELEGRAM_CHAT_ENV, "--chat-id")?;
            adapter_with_base(TelegramBotAdapter::new(token, chat))
                .send_text(&text)
                .await?;
            println!("✓ Telegram 전송 완료 ({} chars)", text.chars().count());
        }
        NotifyAction::TelegramListen {
            bot_token,
            chat_id_filter,
            store_session_title,
            data_dir,
            once,
        } => {
            let token = resolve(bot_token, TELEGRAM_TOKEN_ENV, "--bot-token")?;
            let adapter = adapter_with_base(TelegramBotAdapter::new(token, ""));
            run_telegram_listen(
                adapter,
                chat_id_filter,
                store_session_title.as_deref(),
                data_dir.as_deref(),
                once,
            )
            .await?;
        }
        NotifyAction::DiscordListen {
            bot_token,
            channel_id,
            store_session,
            data_dir,
            pretty,
        } => {
            run_discord_listen(bot_token, channel_id, store_session, data_dir, pretty).await?;
        }
        NotifyAction::Channel {
            mcp_url,
            auth_token,
            mode,
        } => {
            run_channel(mcp_url, auth_token, mode).await?;
        }
    }
    Ok(())
}

// ── Channel MCP ──────────────────────────────────────────────────────────

async fn run_channel(
    mcp_url: Option<String>,
    auth_token: Option<String>,
    mode: ChannelMode,
) -> Result<()> {
    let url = resolve(mcp_url, CHANNEL_MCP_URL_ENV, "--mcp-url")?;
    let token = auth_token.or_else(|| {
        std::env::var(CHANNEL_MCP_TOKEN_ENV)
            .ok()
            .filter(|s| !s.is_empty())
    });
    let client = ChannelMcpClient::new(url, token);

    match mode {
        ChannelMode::Platform {
            platform,
            channel_id,
            text,
            reply_to,
        } => {
            let r = client
                .send_to_platform(&platform, &channel_id, &text, reply_to.as_deref())
                .await?;
            if !r.success {
                bail!(
                    "channel-mcp send_to_platform 실패: {}",
                    r.error.unwrap_or_else(|| "(no error message)".into())
                );
            }
            println!(
                "✓ channel-mcp send_to_platform({platform}, {channel_id}) 완료{}",
                r.message_id
                    .map(|id| format!(" — id={id}"))
                    .unwrap_or_default()
            );
        }
        ChannelMode::Peer {
            to_role,
            summary,
            msg_type,
        } => {
            let r = client.send_message(&to_role, &summary, &msg_type).await?;
            if !r.success {
                bail!(
                    "channel-mcp send_message 실패: {}",
                    r.error.unwrap_or_else(|| "(no error message)".into())
                );
            }
            println!(
                "✓ channel-mcp send_message(to={to_role}, type={msg_type}) 완료{}",
                r.message_id
                    .map(|id| format!(" — id={id}"))
                    .unwrap_or_default()
            );
        }
        ChannelMode::ListAdapters => {
            let list = client.list_adapters().await?;
            if list.is_empty() {
                println!("(등록된 어댑터 없음)");
            } else {
                println!("등록된 channel-mcp 어댑터 ({}):", list.len());
                for a in &list {
                    let conn = if a.connected { "✓" } else { "✗" };
                    let ch = a
                        .channel_id
                        .as_deref()
                        .map(|c| format!(" channel={c}"))
                        .unwrap_or_default();
                    let note = a
                        .note
                        .as_deref()
                        .map(|n| format!(" — {n}"))
                        .unwrap_or_default();
                    println!("  {conn} {}{ch}{note}", a.platform);
                }
            }
        }
    }
    Ok(())
}

/// `TELEGRAM_API_BASE` 가 설정되면 (테스트·mock) 어댑터 base 교체.
fn adapter_with_base(a: TelegramBotAdapter) -> TelegramBotAdapter {
    if let Ok(base) = std::env::var(TELEGRAM_API_BASE_ENV) {
        if !base.is_empty() {
            return a.with_api_base(base);
        }
    }
    a
}

fn resolve(arg: Option<String>, env: &str, flag: &str) -> Result<String> {
    arg.or_else(|| std::env::var(env).ok())
        .ok_or_else(|| anyhow!("{flag} 또는 환경변수 {env} 가 필요합니다"))
}

// ── Telegram listen ──────────────────────────────────────────────────────

async fn run_telegram_listen(
    adapter: TelegramBotAdapter,
    chat_id_filter: Option<i64>,
    store_session_title: Option<&str>,
    data_dir: Option<&Path>,
    once: bool,
) -> Result<()> {
    println!(
        "✓ Telegram listen 시작 (chat_id_filter={:?}, store_session={:?}, once={})",
        chat_id_filter, store_session_title, once
    );

    let mut store_ctx = if let Some(title) = store_session_title {
        let dir = resolve_data_dir(data_dir)?;
        Some(StoreCtx::open(&dir, title)?)
    } else {
        None
    };

    let stop = Arc::new(AtomicBool::new(false));
    let _signal_handle = {
        let stop = stop.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                stop.store(true, Ordering::SeqCst);
                eprintln!("\n[telegram-listen] Ctrl+C 감지 — 종료 중...");
            }
        })
    };

    let mut offset: i64 = 0;
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let updates = match adapter
            .poll_updates(offset, Some(if once { 1 } else { 25 }))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[telegram-listen] poll 오류: {e} — 5초 후 재시도");
                if once {
                    return Err(e.into());
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        for u in &updates {
            offset = offset.max(u.update_id + 1);
            if let Some(filter) = chat_id_filter {
                if u.chat_id != filter {
                    continue;
                }
            }
            handle_telegram_update(u, store_ctx.as_mut())?;
        }

        if once {
            break;
        }
    }
    println!("✓ Telegram listen 종료 (마지막 offset={})", offset);
    Ok(())
}

fn handle_telegram_update(u: &TelegramUpdate, store: Option<&mut StoreCtx>) -> Result<()> {
    let sender = u.sender_username.as_deref().unwrap_or("(anonymous)");
    println!(
        "[{}] tg chat={} from=@{} update_id={}: {}",
        kst_now().to_rfc3339(),
        u.chat_id,
        sender,
        u.update_id,
        u.text,
    );
    if let Some(ctx) = store {
        let sender_label = format!(
            "telegram:{}",
            u.sender_username.as_deref().unwrap_or("anonymous")
        );
        ctx.append(&sender_label, &u.text)?;
    }
    Ok(())
}

// ── Discord listen ───────────────────────────────────────────────────────

async fn run_discord_listen(
    bot_token: Option<String>,
    channel_id: Option<u64>,
    store_session: Option<String>,
    data_dir: Option<PathBuf>,
    pretty: bool,
) -> Result<()> {
    let token = resolve(bot_token, DISCORD_BOT_TOKEN_ENV, "--bot-token")?;

    let mut store_ctx = if let Some(title) = &store_session {
        let dir = resolve_data_dir(data_dir.as_deref())?;
        Some(StoreCtx::open(&dir, title)?)
    } else {
        None
    };

    let client = DiscordGatewayClient::new(token);
    let stream: std::pin::Pin<Box<dyn futures_util::Stream<Item = DiscordIncomingMessage> + Send>> =
        match channel_id {
            Some(cid) => Box::pin(client.listen_channel(cid).await?),
            None => Box::pin(client.connect().await?),
        };

    eprintln!(
        "✓ Discord Gateway 연결됨 — 메시지 수신 대기 중 (Ctrl+C 종료){}",
        store_ctx
            .as_ref()
            .map(|c| format!(" · L0 store=session/{}", c.session_id))
            .unwrap_or_default()
    );

    let mut stream = stream;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                eprintln!("\n✓ Ctrl+C — 종료");
                return Ok(());
            }
            next = stream.next() => {
                match next {
                    Some(msg) => {
                        emit_discord(&msg, pretty);
                        if let Some(ctx) = store_ctx.as_mut() {
                            let sender_label = format!("discord:{}", msg.author_name);
                            if let Err(e) = ctx.append(&sender_label, &msg.content) {
                                tracing::warn!(error = %e, "L0 저장 실패");
                                eprintln!("⚠ L0 저장 실패: {e:#}");
                            }
                        }
                    }
                    None => {
                        eprintln!("⚠ Discord Gateway stream 종료 (서버 측 disconnect)");
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// daemon 내부에서 spawn 하는 long-running Discord inbound listener.
/// CLI 의 run_discord_listen 과 다르게 ctrl_c 안 잡음 + 항상 store 모드.
/// daemon 의 ctrl_c 가 tokio runtime 종료 시 같이 정리됨.
pub async fn run_discord_inbound_for_daemon(
    data_dir: PathBuf,
    bot_token: String,
    master_key: Option<Keypair>,
) -> Result<()> {
    let mut store = match master_key {
        Some(k) => StoreCtx::open_with_key(&data_dir, "discord-inbox", Some(k))?,
        None => StoreCtx::open_with_key(&data_dir, "discord-inbox", None)?,
    };
    let client = DiscordGatewayClient::new(bot_token.clone());
    let mut stream: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = DiscordIncomingMessage> + Send>,
    > = Box::pin(client.connect().await?);

    let portal_url = std::env::var("XGRAM_PORTAL_URL").unwrap_or_else(|_| "http://127.0.0.1:9400".into());
    let portal_token = std::env::var("XGRAM_PORTAL_TOKEN").unwrap_or_else(|_| "0205".into());
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    tracing::info!("discord inbound listener: connected, draining stream (rc.91 routing 활성)");
    while let Some(msg) = stream.next().await {
        let sender = format!("discord:{}", msg.author_name);
        // (1) L0 저장
        if let Err(e) = store.append(&sender, &msg.content) {
            tracing::warn!(error = %e, "discord inbound L0 저장 실패");
        }

        // (2) bindings 조회 + 매칭 세션에 dispatch (rc.91)
        let bindings_result: rusqlite::Result<Vec<(String, Option<String>, String)>> = {
            let conn = store.db.conn();
            let mut stmt = match conn.prepare(
                "SELECT agent_id, mention_trigger, permission \
                 FROM session_channel_bindings \
                 WHERE platform='discord' AND channel_ref = ?1 AND active = 1",
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "bindings SELECT prepare 실패");
                    continue;
                }
            };
            let channel_str = msg.channel_id.to_string();
            let rows = stmt
                .query_map([&channel_str], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?))
                })
                .and_then(|m| m.collect::<rusqlite::Result<Vec<_>>>());
            rows
        };
        let bindings = match bindings_result {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "bindings SELECT 실패");
                continue;
            }
        };

        for (agent_id, mention, perm) in bindings {
            // mention_trigger 매칭 — 비어있으면 모든 메시지, 있으면 content 에 포함된 것만
            if let Some(m) = mention.as_deref() {
                let m_trim = m.trim();
                if !m_trim.is_empty() && !msg.content.contains(m_trim) {
                    continue;
                }
            }
            if perm == "read_only" {
                continue;
            }
            // command 모드는 일단 prefix 체크 ('/'); reply 는 그대로
            if perm == "command" && !msg.content.trim_start().starts_with('/') {
                continue;
            }
            // (rc.91) 첨부 다운로드 — ~/.openxgram/inbox/discord/<msg_id>/<filename>
            let mut attach_paths: Vec<String> = Vec::new();
            if !msg.attachments.is_empty() {
                let inbox = data_dir.join("inbox/discord").join(msg.message_id.to_string());
                let _ = std::fs::create_dir_all(&inbox);
                for a in &msg.attachments {
                    let safe_name = a.filename.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-' && c != '_', "_");
                    if safe_name.is_empty() { continue; }
                    let dst = inbox.join(&safe_name);
                    match http_client.get(&a.url).send().await {
                        Ok(r) if r.status().is_success() => {
                            if let Ok(bytes) = r.bytes().await {
                                if std::fs::write(&dst, &bytes).is_ok() {
                                    attach_paths.push(dst.display().to_string());
                                    tracing::info!(file = %dst.display(), size = bytes.len(), "discord attachment 다운로드");
                                }
                            }
                        }
                        Ok(r) => tracing::warn!(status = %r.status(), url = %a.url, "attachment download HTTP 실패"),
                        Err(e) => tracing::warn!(error = %e, url = %a.url, "attachment download 네트워크 실패"),
                    }
                }
            }
            let attach_line = if attach_paths.is_empty() {
                String::new()
            } else {
                format!("\n[attachments]\n{}\n", attach_paths.join("\n"))
            };
            let injected = format!("\n[discord:{}] {}{}\n", msg.author_name, msg.content, attach_line);
            if let Err(e) =
                dispatch_to_session(&agent_id, &injected, &portal_url, &portal_token, &http_client).await
            {
                tracing::warn!(agent_id = %agent_id, error = %e, "discord → session dispatch 실패");
            } else {
                tracing::info!(agent_id = %agent_id, channel = %msg.channel_id, attachments = attach_paths.len(), "discord → session dispatched");
            }
        }
    }
    tracing::warn!("discord inbound stream 종료 (server disconnect)");
    Ok(())
}

/// identifier (`portal:<session>:<idx>` 또는 `aoe:<session>:...`) 의 세션에 텍스트 주입.
/// portal-new 의 `/api/tmux/send` 호출. peer:* 는 별도 fan-out (TODO).
async fn dispatch_to_session(
    identifier: &str,
    text: &str,
    portal_url: &str,
    portal_token: &str,
    http: &reqwest::Client,
) -> Result<()> {
    let (session, idx) = if let Some(rest) = identifier.strip_prefix("portal:") {
        let mut parts = rest.splitn(2, ':');
        let first = parts.next().unwrap_or("");
        let rest2 = parts.next().unwrap_or("");
        if first.parse::<u32>().is_ok() && !rest2.contains(':') {
            bail!("dispatch: 옛 portal:<idx>:<id> 형식 — agent_id 재바인딩 필요");
        }
        let idx = rest2
            .split(':')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        (first.to_string(), idx)
    } else if let Some(rest) = identifier.strip_prefix("aoe:") {
        let s = rest.split(':').next().unwrap_or("");
        if s.is_empty() {
            bail!("dispatch: aoe identifier 에 tmux_session 없음");
        }
        (s.to_string(), 0u32)
    } else if identifier.starts_with("peer:") {
        bail!("dispatch: peer:* binding 은 미구현 (Phase 2)");
    } else {
        bail!("dispatch: 알 수 없는 identifier prefix: {}", identifier);
    };
    let url = format!("{}/api/tmux/send?token={}", portal_url.trim_end_matches('/'), portal_token);
    let resp = http
        .post(&url)
        .json(&serde_json::json!({
            "session": session,
            "window": idx,
            "text": text,
            "enter": true,
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("portal /api/tmux/send HTTP {}: {}", status, body);
    }
    Ok(())
}

fn emit_discord(msg: &DiscordIncomingMessage, pretty: bool) {
    if pretty {
        println!(
            "[{}] #{} <{}> {}",
            msg.timestamp_kst.format("%Y-%m-%d %H:%M:%S%:z"),
            msg.channel_id,
            msg.author_name,
            msg.content
        );
    } else {
        let line = serde_json::json!({
            "message_id": msg.message_id,
            "channel_id": msg.channel_id,
            "guild_id": msg.guild_id,
            "author_id": msg.author_id,
            "author_name": msg.author_name,
            "content": msg.content,
            "timestamp_kst": msg.timestamp_kst.to_rfc3339(),
        });
        println!("{line}");
    }
}

// ── 공용 store context ────────────────────────────────────────────────────

struct StoreCtx {
    db: Db,
    session_id: String,
    /// None 이면 unsigned 모드 — signature = "external" placeholder (agent_inject 패턴).
    signing_key: Option<Keypair>,
}

impl StoreCtx {
    fn open(data_dir: &Path, session_title: &str) -> Result<Self> {
        let password = require_password()?;
        let ks = FsKeystore::new(keystore_dir(data_dir));
        let kp = ks
            .load(MASTER_KEY_NAME, &password)
            .context("master 키 로드 실패 — keystore 패스워드 확인")?;
        Self::open_with_key(data_dir, session_title, Some(kp))
    }

    fn open_with_key(data_dir: &Path, session_title: &str, signing_key: Option<Keypair>) -> Result<Self> {
        let path = db_path(data_dir);
        if !path.exists() {
            bail!(
                "DB 파일 미존재 ({}). `xgram init` 먼저 실행.",
                path.display()
            );
        }
        let mut db = Db::open(DbConfig {
            path: path.clone(),
            ..Default::default()
        })
        .with_context(|| format!("DB open 실패: {}", path.display()))?;
        let home_machine = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
        let session = SessionStore::new(&mut db).ensure_by_title(session_title, &home_machine)?;
        let mode = if signing_key.is_some() { "signed" } else { "unsigned (external)" };
        println!(
            "✓ store-session 모드 — session={} ({}, {mode}), 메시지를 L0 으로 저장합니다.",
            session.id, session_title
        );
        Ok(Self {
            db,
            session_id: session.id,
            signing_key,
        })
    }

    fn append(&mut self, sender: &str, body: &str) -> Result<()> {
        let signature = match &self.signing_key {
            Some(k) => hex::encode(k.sign(body.as_bytes())),
            None => "external".to_string(),
        };
        let embedder = default_embedder()?;
        let msg = MessageStore::new(&mut self.db, embedder.as_ref()).insert(
            &self.session_id,
            sender,
            body,
            &signature,
            None,
        )?;
        println!("  → L0 저장 (id={})", msg.id);
        Ok(())
    }
}

fn resolve_data_dir(opt: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = opt {
        return Ok(p.to_path_buf());
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow!("HOME 환경변수 없음 — --data-dir 명시 필요"))?;
    Ok(PathBuf::from(home).join(".openxgram"))
}
