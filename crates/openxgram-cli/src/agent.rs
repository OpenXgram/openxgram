//! 메인 에이전트 런타임 — Phase 1 v1.
//!
//! 담당:
//! - daemon 이 inbox-* 세션에 저장한 inbound 메시지를 폴링해서 처리.
//! - 처리 = 1) 콘솔 로그, 2) Discord webhook outbound (옵션), 3) (다음) 서브에이전트 호출.
//! - watermark 는 `<data_dir>/agent-state.json` 에 (session_id, last_seen_ts) 로 저장.
//!
//! v1 범위:
//! - inbox 폴링 + 로그 + Discord forward.
//! - 서브에이전트 호출 라우팅 / 응답 작성 / xgram peer_send 회신은 다음 iteration.
//!
//! 다음 iteration 후보:
//! - Discord inbound (master 가 채널에 친 메시지 → daemon inbox 로 주입)
//! - Starian Channel send_message 호출 — 서브에이전트 실행
//! - 응답 자동 작성 + xgram peer_send 회신

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};
use openxgram_orchestration::{
    kst_now_epoch, ScheduleKind, ScheduledStore, TargetKind,
};
use serde::{Deserialize, Serialize};

use crate::response::Generator;

const STATE_FILE: &str = "agent-state.json";

#[derive(Debug, Clone)]
pub struct AgentOpts {
    pub data_dir: PathBuf,
    pub poll_interval_secs: u64,
    /// Discord webhook URL — outbound forward (옵션).
    pub discord_webhook_url: Option<String>,
    /// Discord bot token — inbound polling (옵션). XGRAM_DISCORD_BOT_TOKEN env.
    pub discord_bot_token: Option<String>,
    /// Discord channel ID — inbound polling target (옵션). XGRAM_DISCORD_CHANNEL_ID env.
    pub discord_channel_id: Option<String>,
    /// Anthropic API key — 활성 시 LLM 응답 (옵션). XGRAM_ANTHROPIC_API_KEY env.
    pub anthropic_api_key: Option<String>,
    /// 자기 alias — system prompt 에 사용 (예: "Starian")
    pub agent_alias: Option<String>,
    /// Telegram bot token (옵션). XGRAM_TELEGRAM_BOT_TOKEN env.
    pub telegram_bot_token: Option<String>,
    /// Telegram chat id (옵션 — 응답 회신 대상). XGRAM_TELEGRAM_CHAT_ID env.
    pub telegram_chat_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AgentState {
    /// session_id → 마지막으로 처리한 message timestamp (RFC3339)
    watermarks: HashMap<String, String>,
    /// Discord channel id → 마지막 message id (snowflake)
    #[serde(default)]
    discord_cursors: HashMap<String, String>,
    /// Telegram getUpdates offset (다음 update_id)
    #[serde(default)]
    telegram_offset: i64,
    /// HITL outbox-to-human 세션의 마지막 forward timestamp
    #[serde(default)]
    hitl_outbox_watermark: String,
}

impl AgentState {
    fn load(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let s = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&s).unwrap_or_default())
    }

    fn save(&self, path: &std::path::Path) -> Result<()> {
        let s = serde_json::to_string_pretty(self)?;
        std::fs::write(path, s)?;
        Ok(())
    }
}

/// 메인 에이전트 런타임 진입점.
pub async fn run_agent(opts: AgentOpts) -> Result<()> {
    let dir = opts.data_dir.clone();
    let state_path = dir.join(STATE_FILE);
    let mut state = AgentState::load(&state_path)?;

    // 첫 가동 시 기본 self-trigger cron (매일 09:00 KST 'morning-briefing') 등록 — idempotent.
    if let Err(e) = ensure_default_self_cron(&dir) {
        eprintln!("[agent][warn] 기본 self cron 등록 실패: {e}");
    }

    eprintln!("xgram agent — Phase 1 v1");
    eprintln!("  data_dir         : {}", dir.display());
    eprintln!(
        "  discord webhook  : {}",
        if opts.discord_webhook_url.is_some() {
            "configured (outbound forward)"
        } else {
            "(not set)"
        }
    );
    eprintln!(
        "  discord inbound  : {}",
        if opts.discord_bot_token.is_some() && opts.discord_channel_id.is_some() {
            "configured (channel poll)"
        } else {
            "(not set)"
        }
    );
    eprintln!("  poll_interval    : {}s", opts.poll_interval_secs);
    eprintln!();
    eprintln!("[agent] inbox 폴링 시작 — Ctrl+C 로 중단");

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("reqwest client 생성")?;

    let generator = Generator::from_anthropic_key(opts.anthropic_api_key.as_deref());
    eprintln!("  generator        : {}", generator.label());

    let interval = Duration::from_secs(opts.poll_interval_secs.max(1));
    loop {
        let mut changed = false;

        // Discord inbound: 새 채널 메시지 → DB 직접 저장 (inbox-from-discord:{user_id} 세션)
        if let (Some(token), Some(chan)) = (
            opts.discord_bot_token.as_deref(),
            opts.discord_channel_id.as_deref(),
        ) {
            match poll_discord_inbound(&dir, &mut state, token, chan, &http).await {
                Ok(n) if n > 0 => changed = true,
                Ok(_) => {}
                Err(e) => eprintln!("[agent][warn] discord inbound 실패: {e}"),
            }
        }

        // Telegram inbound: getUpdates → inbox-from-telegram:{chat_id} 세션
        if let Some(token) = opts.telegram_bot_token.as_deref() {
            match poll_telegram_inbound(&dir, &mut state, token, &http).await {
                Ok(n) if n > 0 => changed = true,
                Ok(_) => {}
                Err(e) => eprintln!("[agent][warn] telegram inbound 실패: {e}"),
            }
        }

        // Self-trigger: 도래한 SelfTrigger 예약을 inbox-from-self:{target} 세션으로 inject.
        if let Err(e) = poll_self_trigger(&dir).await {
            eprintln!("[agent][warn] self-trigger 폴 실패: {e}");
        }

        // HITL: outbox-to-human 의 새 메시지를 Discord/Telegram 으로 push (사람 호출).
        match forward_hitl_outbox(
            &dir,
            &mut state,
            opts.discord_webhook_url.as_deref(),
            opts.telegram_bot_token.as_deref(),
            opts.telegram_chat_id.as_deref(),
            &http,
        )
        .await
        {
            Ok(n) if n > 0 => changed = true,
            Ok(_) => {}
            Err(e) => eprintln!("[agent][warn] HITL forward 실패: {e}"),
        }

        // inbox → outbound forward + 응답 생성 (Generator: Anthropic / Echo)
        match poll_once(
            &dir,
            &mut state,
            opts.discord_webhook_url.as_deref(),
            opts.telegram_bot_token.as_deref(),
            opts.telegram_chat_id.as_deref(),
            &generator,
            opts.agent_alias.as_deref().unwrap_or("Starian"),
            &http,
        )
        .await
        {
            Ok(n) if n > 0 => changed = true,
            Ok(_) => {}
            Err(e) => eprintln!("[agent][warn] poll 실패: {e}"),
        }

        if changed {
            if let Err(e) = state.save(&state_path) {
                eprintln!("[agent][warn] state 저장 실패: {e}");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Discord 채널 폴링 — 신규 메시지를 inbox-from-discord:{user_id} 세션에 저장.
async fn poll_discord_inbound(
    data_dir: &std::path::Path,
    state: &mut AgentState,
    bot_token: &str,
    channel_id: &str,
    http: &reqwest::Client,
) -> Result<usize> {
    let cursor = state.discord_cursors.get(channel_id).cloned();
    let mut url = format!("https://discord.com/api/v10/channels/{channel_id}/messages?limit=50");
    if let Some(after) = cursor.as_deref() {
        url.push_str(&format!("&after={after}"));
    }

    let resp = http
        .get(&url)
        .header("Authorization", format!("Bot {bot_token}"))
        .send()
        .await
        .context("discord messages GET")?;
    if !resp.status().is_success() {
        anyhow::bail!("discord API HTTP {}", resp.status());
    }
    let mut messages: Vec<DiscordMessage> = resp.json().await.context("discord messages JSON")?;
    if messages.is_empty() {
        return Ok(0);
    }
    // Discord 응답: 최신 → 옛날 순. 처리 위해 시간순 (옛날 → 최신) 으로 reverse.
    messages.reverse();

    let mut db = Db::open(DbConfig {
        path: data_dir.join("xgram.db"),
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate()?;
    let embedder = default_embedder()?;

    let mut count = 0usize;
    let mut last_id = cursor.unwrap_or_default();
    for m in messages {
        if m.author.bot.unwrap_or(false) {
            // bot 메시지 (자기 자신·다른 봇) 무시 — 루프 방지.
            last_id = m.id.clone();
            continue;
        }
        let sender = format!("discord:{}", m.author.id);
        let session_title = format!("inbox-from-{}", sender);
        let session = openxgram_memory::SessionStore::new(&mut db)
            .ensure_by_title(&session_title, "inbound")
            .with_context(|| format!("session ensure: {session_title}"))?;
        openxgram_memory::MessageStore::new(&mut db, embedder.as_ref())
            .insert(&session.id, &sender, &m.content, "discord", None)
            .context("discord message insert")?;
        last_id = m.id;
        count += 1;
        eprintln!(
            "[agent][discord] {} → inbox: {}",
            m.author.username,
            m.content.lines().next().unwrap_or("")
        );
    }
    if !last_id.is_empty() {
        state.discord_cursors.insert(channel_id.into(), last_id);
    }
    Ok(count)
}

#[derive(Debug, Deserialize)]
struct DiscordMessage {
    id: String,
    content: String,
    author: DiscordAuthor,
}

#[derive(Debug, Deserialize)]
struct DiscordAuthor {
    id: String,
    username: String,
    bot: Option<bool>,
}

/// Telegram getUpdates 폴링 — 신규 메시지를 inbox-from-telegram:{chat_id} 세션에 저장.
async fn poll_telegram_inbound(
    data_dir: &std::path::Path,
    state: &mut AgentState,
    bot_token: &str,
    http: &reqwest::Client,
) -> Result<usize> {
    let url = format!(
        "https://api.telegram.org/bot{bot_token}/getUpdates?offset={}&timeout=0",
        state.telegram_offset
    );
    let resp = http.get(&url).send().await.context("telegram getUpdates")?;
    if !resp.status().is_success() {
        anyhow::bail!("telegram API HTTP {}", resp.status());
    }
    let parsed: TelegramUpdates = resp.json().await.context("telegram JSON parse")?;
    if !parsed.ok || parsed.result.is_empty() {
        return Ok(0);
    }

    let mut db = Db::open(DbConfig {
        path: data_dir.join("xgram.db"),
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate()?;
    let embedder = default_embedder()?;

    let mut count = 0usize;
    let mut max_id = state.telegram_offset - 1;
    for u in parsed.result {
        if u.update_id > max_id {
            max_id = u.update_id;
        }
        let Some(msg) = u.message else { continue };
        let Some(text) = msg.text else { continue };
        if msg.from.is_bot.unwrap_or(false) {
            continue;
        }
        let sender = format!("telegram:{}", msg.chat.id);
        let session_title = format!("inbox-from-{}", sender);
        let session = openxgram_memory::SessionStore::new(&mut db)
            .ensure_by_title(&session_title, "inbound")?;
        openxgram_memory::MessageStore::new(&mut db, embedder.as_ref()).insert(
            &session.id,
            &sender,
            &text,
            "telegram",
            None,
        )?;
        count += 1;
        eprintln!(
            "[agent][telegram] {} → inbox: {}",
            msg.from.username.as_deref().unwrap_or("?"),
            text.lines().next().unwrap_or("")
        );
    }
    state.telegram_offset = max_id + 1;
    Ok(count)
}

#[derive(Debug, Deserialize)]
struct TelegramUpdates {
    ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessageT>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessageT {
    chat: TelegramChat,
    from: TelegramUser,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    is_bot: Option<bool>,
    username: Option<String>,
}

#[derive(Serialize)]
struct TelegramSendBody<'a> {
    chat_id: &'a str,
    text: &'a str,
}

async fn post_to_telegram(
    http: &reqwest::Client,
    bot_token: &str,
    chat_id: &str,
    text: &str,
) -> Result<()> {
    // Telegram 메시지 길이 제한 4096자.
    let truncated: String = text.chars().take(4000).collect();
    let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
    let resp = http
        .post(&url)
        .json(&TelegramSendBody {
            chat_id,
            text: &truncated,
        })
        .send()
        .await
        .context("telegram sendMessage")?;
    if !resp.status().is_success() {
        anyhow::bail!("telegram sendMessage HTTP {}", resp.status());
    }
    Ok(())
}

/// 한 번의 폴링 — inbox-* 세션의 신규 메시지를 처리. 처리한 개수 반환.
async fn poll_once(
    data_dir: &std::path::Path,
    state: &mut AgentState,
    discord_url: Option<&str>,
    telegram_token: Option<&str>,
    telegram_chat: Option<&str>,
    generator: &Generator,
    agent_alias: &str,
    http: &reqwest::Client,
) -> Result<usize> {
    let mut db = Db::open(DbConfig {
        path: data_dir.join("xgram.db"),
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    let embedder = default_embedder().context("embedder init 실패")?;

    let inbox_sessions: Vec<_> = SessionStore::new(&mut db)
        .list()
        .context("session list 실패")?
        .into_iter()
        .filter(|s| s.title.starts_with("inbox-from-"))
        .collect();

    let mut processed = 0usize;
    for session in inbox_sessions {
        let watermark = state
            .watermarks
            .get(&session.id)
            .cloned()
            .unwrap_or_default();
        let messages = {
            let mut store = MessageStore::new(&mut db, embedder.as_ref());
            store
                .list_for_session(&session.id)
                .with_context(|| format!("messages list_for_session({})", session.id))?
        };

        let mut last_ts = watermark.clone();
        for m in messages {
            let ts = m.timestamp.to_rfc3339();
            if !watermark.is_empty() && ts <= watermark {
                continue;
            }

            eprintln!(
                "[agent][inbox] {} ({}): {}",
                session.title,
                m.sender,
                m.body.lines().next().unwrap_or("")
            );

            // c-마무리 — friend-accept handshake 자동 처리.
            // 일반 응답 흐름 들어가기 전에 magic prefix 체크 → peer 자동 등록 후 메시지 종료.
            if let Some(acc) = crate::invite::parse_accept_message(&m.body) {
                use openxgram_peer::{PeerRole, PeerStore};
                let mut store = PeerStore::new(&mut db);
                let already = store.get_by_alias(&acc.alias).unwrap_or(None).is_some();
                if !already {
                    if let Err(e) = store.add(
                        &acc.alias,
                        &acc.pubkey_hex,
                        &acc.address,
                        PeerRole::Worker,
                        Some("via friend handshake"),
                    ) {
                        eprintln!("[agent][friend] auto peer add 실패 ({}): {e}", acc.alias);
                    } else {
                        eprintln!("[agent][friend] handshake → 자동 peer 등록: {} ({})", acc.alias, acc.address);
                    }
                }
                last_ts = ts;
                processed += 1;
                continue;
            }

            // 응답 라우팅 대상: Discord / Telegram / Self / Peer.
            let from_discord = m.sender.starts_with("discord:");
            let from_telegram = m.sender.starts_with("telegram:");
            let from_self = m.sender.starts_with("self:");
            let from_peer = m.sender.starts_with("peer:");
            if from_discord || from_telegram || from_self || from_peer {
                // 1.7.3.4 — 같은 conversation 의 이전 메시지 (시간순) 를 LLM 컨텍스트로 동봉.
                let history = MessageStore::new(&mut db, embedder.as_ref())
                    .list_for_conversation(&m.conversation_id)
                    .unwrap_or_default();
                let out = generator
                    .generate(http, agent_alias, &m.body, &history)
                    .await
                    .context("response generator")?;
                let response = out.body;
                let signature = out.signature;
                // 회신 라우팅: Discord/Telegram 은 원래 채널로, self 는 (옵션) Discord 로 관전 push.
                if from_discord {
                    if let Some(url) = discord_url {
                        let payload = format!("🤖 **{agent_alias}**: {response}");
                        if let Err(e) = post_to_discord(http, url, &payload).await {
                            eprintln!("[agent][warn] Discord 회신 실패: {e}");
                        }
                    }
                } else if from_telegram {
                    if let (Some(t), Some(c)) = (telegram_token, telegram_chat) {
                        let payload = format!("🤖 {agent_alias}: {response}");
                        if let Err(e) = post_to_telegram(http, t, c, &payload).await {
                            eprintln!("[agent][warn] Telegram 회신 실패: {e}");
                        }
                    }
                } else if from_self {
                    // master 가 보고 있을 수 있는 Discord 채널이 있다면 self-trigger 결과를 함께 push.
                    if let Some(url) = discord_url {
                        let payload =
                            format!("🤖 **{agent_alias}** [self/{}]: {response}", m.sender);
                        if let Err(e) = post_to_discord(http, url, &payload).await {
                            eprintln!("[agent][warn] Discord self-forward 실패: {e}");
                        }
                    }
                } else if from_peer {
                    // 1.9.1.3 — 다른 xgram 노드(peer) 가 보낸 메시지에 대한 회신은 peer_send.
                    // sender 형식 `peer:{alias}` — alias 추출 후 conversation_id 와 함께 전달.
                    let peer_alias = m.sender.trim_start_matches("peer:").to_string();
                    if let Ok(password) = std::env::var("XGRAM_KEYSTORE_PASSWORD") {
                        if let Err(e) = crate::peer_send::run_peer_send_with_conv(
                            data_dir,
                            &peer_alias,
                            None,
                            &response,
                            &password,
                            Some(m.conversation_id.clone()),
                        )
                        .await
                        {
                            eprintln!("[agent][warn] peer 회신 실패 ({peer_alias}): {e}");
                        }
                    } else {
                        eprintln!(
                            "[agent][peer] {} 에게 회신 (XGRAM_KEYSTORE_PASSWORD 미설정 — outbox 만 기록)",
                            peer_alias
                        );
                    }
                }
                // outbox 메모리 기록 — inbound 의 conversation_id 를 재사용해서 같은 대화로 묶음.
                let outbox_title = format!("outbox-to-{}", m.sender);
                let outbox = openxgram_memory::SessionStore::new(&mut db)
                    .ensure_by_title(&outbox_title, "outbound")
                    .context("outbox session ensure")?;
                let mut store2 = MessageStore::new(&mut db, embedder.as_ref());
                store2
                    .insert(
                        &outbox.id,
                        agent_alias,
                        &response,
                        signature,
                        Some(&m.conversation_id),
                    )
                    .context("outbox message insert")?;
            } else if let Some(url) = discord_url {
                let body = format!("**{}** ({}): {}", session.title, m.sender, m.body);
                if let Err(e) = post_to_discord(http, url, &body).await {
                    eprintln!("[agent][warn] Discord 전송 실패: {e}");
                }
            }

            last_ts = ts;
            processed += 1;
        }

        if last_ts != watermark {
            state.watermarks.insert(session.id, last_ts);
        }
    }

    Ok(processed)
}

/// 도래한 SelfTrigger 예약을 inbox-from-self:{target} 세션으로 inject.
/// cron 예약은 mark_sent 가 자동으로 next_due 를 다음 발화 시각으로 재계산.
pub async fn poll_self_trigger(data_dir: &std::path::Path) -> Result<usize> {
    let mut db = Db::open(DbConfig {
        path: data_dir.join("xgram.db"),
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate()?;

    let due = {
        let store = ScheduledStore::new(db.conn());
        store
            .list_due(kst_now_epoch())
            .context("scheduled list_due")?
    };
    let due: Vec<_> = due
        .into_iter()
        .filter(|m| matches!(m.target_kind, TargetKind::SelfTrigger))
        .collect();
    if due.is_empty() {
        return Ok(0);
    }

    let embedder = default_embedder()?;
    let mut count = 0usize;
    for sched in due {
        let sender = format!("self:{}", sched.target);
        let session_title = format!("inbox-from-{sender}");
        let session = SessionStore::new(&mut db)
            .ensure_by_title(&session_title, "self-trigger")
            .with_context(|| format!("self session ensure: {session_title}"))?;
        let insert_res = MessageStore::new(&mut db, embedder.as_ref()).insert(
            &session.id,
            &sender,
            &sched.payload,
            "self-trigger",
            None,
        );
        let store = ScheduledStore::new(db.conn());
        match insert_res {
            Ok(_) => {
                if let Err(e) = store.mark_sent(&sched.id) {
                    eprintln!("[agent][warn] mark_sent 실패 ({}): {e}", sched.id);
                }
                count += 1;
                eprintln!(
                    "[agent][self] {} → inbox: {}",
                    sched.target,
                    sched.payload.lines().next().unwrap_or("")
                );
            }
            Err(e) => {
                let _ = store.mark_failed(&sched.id, &format!("inject 실패: {e}"));
                eprintln!("[agent][warn] self-trigger inject 실패 ({}): {e}", sched.id);
            }
        }
    }
    Ok(count)
}

/// HITL outbox-to-human 세션의 신규 메시지 (magic prefix `xgram-human-input-required-v1`) 를
/// Discord/Telegram 으로 push. watermark 로 중복 방지.
async fn forward_hitl_outbox(
    data_dir: &std::path::Path,
    state: &mut AgentState,
    discord_url: Option<&str>,
    telegram_token: Option<&str>,
    telegram_chat: Option<&str>,
    http: &reqwest::Client,
) -> Result<usize> {
    if discord_url.is_none() && telegram_token.is_none() {
        return Ok(0); // 채널 없으면 forward 의미 없음
    }
    let mut db = Db::open(DbConfig {
        path: data_dir.join("xgram.db"),
        ..Default::default()
    })
    .context("DB open")?;
    db.migrate()?;
    let embedder = default_embedder()?;
    let session = match SessionStore::new(&mut db)
        .list()?
        .into_iter()
        .find(|s| s.title == crate::hitl::HUMAN_OUTBOX_SESSION)
    {
        Some(s) => s,
        None => return Ok(0),
    };
    let messages = MessageStore::new(&mut db, embedder.as_ref())
        .list_for_session(&session.id)?;
    let mut last_ts = state.hitl_outbox_watermark.clone();
    let mut count = 0usize;
    for m in messages {
        let ts = m.timestamp.to_rfc3339();
        if !state.hitl_outbox_watermark.is_empty() && ts <= state.hitl_outbox_watermark {
            continue;
        }
        let req = match crate::hitl::parse_human_request(&m.body) {
            Some(r) => r,
            None => {
                last_ts = ts;
                continue;
            }
        };
        let mut payload = format!("🙋 **사람 입력 필요** [{}]\n{}", req.id, req.question);
        if !req.options.is_empty() {
            payload.push_str("\n옵션:");
            for o in &req.options {
                payload.push_str(&format!("\n- {o}"));
            }
        }
        payload.push_str(&format!(
            "\n\n응답: `xgram human respond {} <답>`",
            req.id
        ));
        if let Some(url) = discord_url {
            if let Err(e) = post_to_discord(http, url, &payload).await {
                eprintln!("[agent][warn] HITL Discord push 실패: {e}");
            }
        }
        if let (Some(t), Some(c)) = (telegram_token, telegram_chat) {
            if let Err(e) = post_to_telegram(http, t, c, &payload).await {
                eprintln!("[agent][warn] HITL Telegram push 실패: {e}");
            }
        }
        last_ts = ts;
        count += 1;
    }
    if last_ts != state.hitl_outbox_watermark {
        state.hitl_outbox_watermark = last_ts;
    }
    Ok(count)
}

/// 기본 self-trigger cron — 매일 09:00 KST `morning-briefing` (idempotent).
/// 같은 (kind=SelfTrigger, target='morning-briefing') entry 가 이미 있으면 skip.
pub fn ensure_default_self_cron(data_dir: &std::path::Path) -> Result<()> {
    let mut db = Db::open(DbConfig {
        path: data_dir.join("xgram.db"),
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate()?;
    let store = ScheduledStore::new(db.conn());
    let existing = store.list(None).context("scheduled list")?;
    if existing
        .iter()
        .any(|m| matches!(m.target_kind, TargetKind::SelfTrigger) && m.target == "morning-briefing")
    {
        return Ok(());
    }
    store
        .insert(
            TargetKind::SelfTrigger,
            "morning-briefing",
            "오늘 작업을 정리해주세요. 어제까지 진행 상황과 오늘 우선순위를 짧게 요약해 보고합니다.",
            "info",
            ScheduleKind::Cron,
            "0 9 * * *",
        )
        .context("default self cron INSERT")?;
    eprintln!("[agent][self] 기본 cron 등록: 매일 09:00 KST 'morning-briefing'");
    Ok(())
}

#[derive(Serialize)]
struct DiscordWebhookBody<'a> {
    content: &'a str,
}

async fn post_to_discord(http: &reqwest::Client, url: &str, content: &str) -> Result<()> {
    // Discord 메시지 길이 제한 (2000자) — 초과 시 잘라서 전송.
    let truncated: String = content.chars().take(1900).collect();
    let resp = http
        .post(url)
        .json(&DiscordWebhookBody {
            content: &truncated,
        })
        .send()
        .await
        .context("Discord webhook POST")?;
    if !resp.status().is_success() {
        anyhow::bail!("Discord webhook 비정상 응답: HTTP {}", resp.status());
    }
    Ok(())
}
