//! 메인 에이전트 런타임 — Phase 1 스켈레톤 (v0).
//!
//! 자율 루프: inbound 메시지 폴링 → 처리 (현재는 echo) → 메모리 기록.
//! 다음 iteration: 채널 어댑터(Discord/Telegram) 통합, 서브에이전트 호출 라우팅.
//!
//! 메모리 정정된 아키텍처 (`project_architecture.md`) 의 1단계 구현.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct AgentOpts {
    pub data_dir: Option<PathBuf>,
    pub daemon_url: String,
    pub daemon_token: Option<String>,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
struct DaemonHealth {
    #[allow(dead_code)]
    status: String,
}

/// 메인 에이전트 런타임 진입점.
///
/// v0: daemon health 확인 + 폴링 루프 (무한). Ctrl+C 로 중단.
/// 다음 단계: /v1/peer/inbox 류 엔드포인트 폴링하여 inbound envelope 받음.
pub async fn run_agent(opts: AgentOpts) -> Result<()> {
    eprintln!("xgram agent — Phase 1 스켈레톤 v0");
    eprintln!("  daemon_url       : {}", opts.daemon_url);
    eprintln!("  poll_interval    : {}s", opts.poll_interval_secs);
    eprintln!();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("reqwest client 생성")?;

    let health_url = format!("{}/v1/gui/health", opts.daemon_url.trim_end_matches('/'));

    let mut req = client.get(&health_url);
    if let Some(t) = opts.daemon_token.as_deref() {
        req = req.bearer_auth(t);
    }
    let resp = req
        .send()
        .await
        .with_context(|| format!("daemon health 호출 실패 ({health_url})"))?;
    if !resp.status().is_success() {
        anyhow::bail!("daemon health 비정상 응답: HTTP {}", resp.status());
    }
    let _h: DaemonHealth = resp.json().await.context("health JSON parse")?;
    eprintln!("✓ daemon 연결 OK ({health_url})");
    eprintln!();
    eprintln!("[agent] 폴링 루프 시작 — Ctrl+C 로 중단");

    let interval = Duration::from_secs(opts.poll_interval_secs.max(1));
    let mut tick = 0u64;
    loop {
        tokio::time::sleep(interval).await;
        tick += 1;
        eprintln!("[agent] tick #{tick} — (inbox 폴링은 다음 iteration)");
    }
}
