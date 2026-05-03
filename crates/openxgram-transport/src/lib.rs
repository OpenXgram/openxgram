//! openxgram-transport — 사이드카 간 HTTP 메시지 전송.
//!
//! Phase 1 first PR: localhost 송수신 baseline.
//!   - axum HTTP server (POST /v1/message)
//!   - reqwest client (send_envelope)
//!
//! 후속 PR:
//!   - Tailscale IP / mTLS
//!   - PRD §4 자동 라우팅 (localhost → Tailscale → XMTP)
//!   - 서명 검증 (현재는 transport 책임 외, 호출자/keystore 영역)

pub mod rate_limit;
pub mod replay;
pub mod tailscale;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::task::JoinHandle;

/// 사이드카 간 표준 메시지 envelope. 서명 형식·검증은 호출자가 처리한다.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Envelope {
    /// 송신자 주소 (예: secp256k1 EIP-55)
    pub from: String,
    /// 수신자 주소
    pub to: String,
    /// 임의 binary payload — hex 인코딩
    pub payload_hex: String,
    /// 송신 시각 (KST 권장)
    pub timestamp: DateTime<FixedOffset>,
    /// 송신자 서명 (hex). 검증은 수신자 측 상위 레이어
    pub signature_hex: String,
    /// replay 방지 nonce (PRD-MFA-01). UUID 또는 random hex 권장.
    /// None 이면 backward-compat — 검증 안 함 (legacy 메시지 호환).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("server error (status {status}): {body}")]
    ServerError { status: u16, body: String },
}

pub type Result<T> = std::result::Result<T, TransportError>;

/// `bind_addr` 에 axum 서버를 띄우고 수신 envelope 을 메모리 큐에 쌓는다.
/// 테스트·간이 데몬용. 실제 데몬은 후속 PR.
#[derive(Debug)]
pub struct ServerHandle {
    pub bound_addr: SocketAddr,
    received: Arc<Mutex<Vec<Envelope>>>,
    join: JoinHandle<()>,
}

impl ServerHandle {
    /// 지금까지 수신한 모든 envelope (clone). 큐에서 제거 안 함.
    pub fn received(&self) -> Vec<Envelope> {
        self.received.lock().expect("poisoned").clone()
    }

    /// 큐를 비우고 모든 envelope 반환. 처리 중 빈 큐 보장.
    pub fn drain_received(&self) -> Vec<Envelope> {
        let mut guard = self.received.lock().expect("poisoned");
        std::mem::take(&mut *guard)
    }

    /// 서버 종료 (현재는 task abort — graceful shutdown 은 후속).
    pub fn shutdown(self) {
        self.join.abort();
    }
}

/// 외부 모니터링용 metrics text provider — Prometheus exposition format 등.
pub type MetricsProvider = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Clone)]
struct AppState {
    received: Arc<Mutex<Vec<Envelope>>>,
    started_at: std::time::Instant,
    metrics: Option<MetricsProvider>,
    replay: Arc<replay::ReplayCache>,
    rate_limiter: Arc<rate_limit::RateLimiter>,
}

pub async fn spawn_server(bind_addr: SocketAddr) -> Result<ServerHandle> {
    spawn_server_with_metrics(bind_addr, None).await
}

pub async fn spawn_server_with_metrics(
    bind_addr: SocketAddr,
    metrics: Option<MetricsProvider>,
) -> Result<ServerHandle> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let state = AppState {
        received: received.clone(),
        started_at: std::time::Instant::now(),
        metrics,
        replay: Arc::new(replay::ReplayCache::default()),
        rate_limiter: Arc::new(rate_limit::RateLimiter::default()),
    };

    let app = Router::new()
        .route("/v1/health", get(health_check))
        .route("/v1/message", post(receive_message))
        .route("/v1/metrics", get(metrics_endpoint))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let bound_addr = listener.local_addr()?;

    let join = tokio::spawn(async move {
        // serve 가 에러로 종료되면 trace 만 — abort 시 정상 종료
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "transport server stopped");
        }
    });

    Ok(ServerHandle {
        bound_addr,
        received,
        join,
    })
}

#[derive(Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    /// daemon 가동 후 경과 (초)
    pub uptime_seconds: u64,
    /// 누적 수신 envelope 수
    pub received_count: usize,
    /// Tailscale BackendState (Running / NeedsLogin / Stopped 등). tailscaled 미설치 시 None.
    pub tailscale_state: Option<String>,
    /// Tailscale 노드 IPv4 (있을 때).
    pub tailscale_ipv4: Option<String>,
}

async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    let uptime_seconds = state.started_at.elapsed().as_secs();
    let received_count = state.received.lock().expect("poisoned").len();
    let (tailscale_state, tailscale_ipv4) = if crate::tailscale::is_running() {
        let st = crate::tailscale::backend_state().ok();
        let ip = crate::tailscale::local_ipv4().ok().map(|a| a.to_string());
        (st, ip)
    } else {
        (None, None)
    };
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds,
        received_count,
        tailscale_state,
        tailscale_ipv4,
    })
}

async fn receive_message(State(state): State<AppState>, Json(env): Json<Envelope>) -> StatusCode {
    // 1. rate limit (PRD-2.0.4)
    if state.rate_limiter.check_and_record(&env.from).is_err() {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    // 2. replay 방지 (PRD-MFA-01) — nonce 가 있는 경우만
    if let Some(nonce) = &env.nonce {
        if !state.replay.check_and_insert(&env.from, nonce) {
            return StatusCode::CONFLICT;
        }
    }
    // 3. timestamp window — 90초 이상 오래된 메시지 reject
    let now = chrono::Utc::now();
    let env_utc = env.timestamp.with_timezone(&chrono::Utc);
    let drift = (now - env_utc).num_seconds().abs();
    if drift > replay::DEFAULT_WINDOW_SECS as i64 {
        return StatusCode::REQUEST_TIMEOUT;
    }
    state.received.lock().expect("poisoned").push(env);
    StatusCode::OK
}

/// Prometheus exposition format. metrics provider 없으면 daemon 내장 baseline 만.
async fn metrics_endpoint(State(state): State<AppState>) -> (StatusCode, String) {
    let uptime = state.started_at.elapsed().as_secs();
    let received = state.received.lock().expect("poisoned").len();
    let mut body = format!(
        "# HELP openxgram_uptime_seconds daemon uptime\n\
         # TYPE openxgram_uptime_seconds gauge\n\
         openxgram_uptime_seconds {uptime}\n\
         # HELP openxgram_received_total inbound envelope 누적 수신 수\n\
         # TYPE openxgram_received_total counter\n\
         openxgram_received_total {received}\n",
    );
    if let Some(p) = &state.metrics {
        body.push_str(&p());
    }
    (StatusCode::OK, body)
}

/// `base_url` 의 `/v1/message` 로 envelope POST. 4xx/5xx 시 raise.
pub async fn send_envelope(base_url: &str, envelope: &Envelope) -> Result<()> {
    let url = format!("{}/v1/message", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(envelope)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(TransportError::ServerError {
            status: status.as_u16(),
            body,
        });
    }
    Ok(())
}
