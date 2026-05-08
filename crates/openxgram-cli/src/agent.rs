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
    /// Discord webhook URL — 옵션. 미지정 시 forward 안 함.
    pub discord_webhook_url: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AgentState {
    /// session_id → 마지막으로 처리한 message timestamp (RFC3339)
    watermarks: HashMap<String, String>,
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
            "configured"
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
        match poll_once(&dir, &mut state, opts.discord_webhook_url.as_deref(), &http).await {
            Ok(n) if n > 0 => {
                if let Err(e) = state.save(&state_path) {
                    eprintln!("[agent][warn] state 저장 실패: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("[agent][warn] poll 실패: {e}"),
        }
        tokio::time::sleep(interval).await;
    }
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

            if let Some(url) = discord_url {
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
