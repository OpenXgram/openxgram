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
    Adapter, DiscordGatewayClient, DiscordIncomingMessage, DiscordWebhookAdapter,
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
        let updates = match adapter.poll_updates(offset, Some(if once { 1 } else { 25 })).await {
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
    signing_key: Keypair,
}

impl StoreCtx {
    fn open(data_dir: &Path, session_title: &str) -> Result<Self> {
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

        // session ensure_by_title — 없으면 생성. home_machine 은 hostname.
        let home_machine = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
        let session = SessionStore::new(&mut db).ensure_by_title(session_title, &home_machine)?;

        // 마스터 키 로드 (서명용). XGRAM_KEYSTORE_PASSWORD 필수.
        let password = require_password()?;
        let ks = FsKeystore::new(keystore_dir(data_dir));
        let kp = ks
            .load(MASTER_KEY_NAME, &password)
            .context("master 키 로드 실패 — keystore 패스워드 확인")?;

        println!(
            "✓ store-session 모드 — session={} ({}), 메시지를 L0 으로 저장합니다.",
            session.id, session_title
        );
        Ok(Self {
            db,
            session_id: session.id,
            signing_key: kp,
        })
    }

    fn append(&mut self, sender: &str, body: &str) -> Result<()> {
        let signature = hex::encode(self.signing_key.sign(body.as_bytes()));
        let embedder = default_embedder()?;
        let msg = MessageStore::new(&mut self.db, embedder.as_ref()).insert(
            &self.session_id,
            sender,
            body,
            &signature,
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
