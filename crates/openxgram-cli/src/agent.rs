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
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AgentState {
    /// session_id → 마지막으로 처리한 message timestamp (RFC3339)
    watermarks: HashMap<String, String>,
    /// Discord channel id → 마지막 message id (snowflake)
    #[serde(default)]
    discord_cursors: HashMap<String, String>,
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

        // inbox → outbound forward
        match poll_once(&dir, &mut state, opts.discord_webhook_url.as_deref(), &http).await {
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
            .insert(&session.id, &sender, &m.content, "discord")
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

/// 한 번의 폴링 — inbox-* 세션의 신규 메시지를 처리. 처리한 개수 반환.
async fn poll_once(
    data_dir: &std::path::Path,
    state: &mut AgentState,
    discord_url: Option<&str>,
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
        let mut store = MessageStore::new(&mut db, embedder.as_ref());
        let messages = store
            .list_for_session(&session.id)
            .with_context(|| format!("messages list_for_session({})", session.id))?;

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

            // 발신자가 Discord 인 메시지는 다시 Discord 로 forward 하지 않는다 (피드백 루프 방지).
            let skip_discord_forward = m.sender.starts_with("discord:");
            if let Some(url) = discord_url {
                if !skip_discord_forward {
                    let body = format!("**{}** ({}): {}", session.title, m.sender, m.body);
                    if let Err(e) = post_to_discord(http, url, &body).await {
                        eprintln!("[agent][warn] Discord 전송 실패: {e}");
                    }
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
