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
use std::sync::atomic::{AtomicI64, Ordering};
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
    /// 같은 inbound→응답 묶음을 cross-node 로 동기하기 위한 hint.
    /// 서명에는 포함되지 않음 (전송 메타데이터). None 이면 수신측이 새 conversation 시작.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// rc.193 — sender 자동 등록 hint (수신측이 unknown sender 자동 peer upsert).
    /// `sender_alias` = sender 의 alias (수신측 peers table 의 alias).
    /// `sender_transport_url` = sender 머신의 외부 reach 가능 URL (예: http://100.101.237.9:47300).
    /// `sender_pubkey_hex` = sender 의 compressed secp256k1 pubkey (서명 검증 용).
    /// 메타데이터 — 서명 검증 대상 아님. 거짓 가능성 있지만 자동 등록은 best-effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_transport_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_pubkey_hex: Option<String>,
    /// rc.199 — 받는 측 tmux push 매핑 hint.
    /// 송신 측이 peer table 의 alias 로 send 시 그 alias 자체를 동봉.
    /// 받는 측 process_inbound 가 envelope.to pubkey 매핑 실패 시 이 hint 로 tmux session resolve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_alias: Option<String>,
    /// rc.219 — envelope 종류. None / "message" = 일반 메시지 (default).
    /// "ack" = receiver → sender 응답. ACK 시 ack_for_ulid + ack_status 필수.
    /// process_inbound 가 envelope_type="ack" 받으면 inbox 저장 X + outbound_queue.ack_at UPDATE.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope_type: Option<String>,
    /// rc.219 — ACK envelope 의 원본 msg_ulid (sender outbound_queue 의 row 매칭).
    /// envelope_type="ack" 일 때 필수.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack_for_ulid: Option<String>,
    /// rc.219 — ACK 의 처리 결과 status:
    ///   "inbox_stored" — DB insert 만 성공 (tmux 매칭 실패).
    ///   "tmux_injected" — tmux 매칭 + inject 성공 (inbox_stored 도 포함).
    ///   "both" — 모두 성공 (== tmux_injected, 호환).
    ///   "fail" — DB insert 실패.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack_status: Option<String>,
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
    /// rc.238 — inbound_processor 가 매 tick 마다 update 하는 마지막 tick 시각 (unix epoch secs).
    /// health endpoint 가 `last_inbound_tick_secs_ago` 로 노출 → 외부 watchdog 가 stuck 감지.
    /// 0 = 아직 한 번도 tick 안 함.
    last_inbound_tick: Arc<AtomicI64>,
    join: JoinHandle<()>,
}

impl ServerHandle {
    /// 지금까지 수신한 모든 envelope (clone). 큐에서 제거 안 함.
    pub fn received(&self) -> Vec<Envelope> {
        self.received.lock().expect("poisoned").clone()
    }

    /// rc.238 — inbound_processor 가 매 tick 마다 호출. 현재 unix epoch secs 기록.
    /// health endpoint 가 이를 읽어 `last_inbound_tick_secs_ago` 계산.
    pub fn mark_inbound_tick(&self) {
        let now = chrono::Utc::now().timestamp();
        self.last_inbound_tick.store(now, Ordering::Relaxed);
    }

    /// rc.238 — 공유 AtomicI64 핸들 반환 (별도 task 에서 mark 하고 싶을 때).
    pub fn inbound_tick_handle(&self) -> Arc<AtomicI64> {
        self.last_inbound_tick.clone()
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

/// cross-machine peer-sync (gossip) 용 reachable peer 요약 DTO.
///
/// `daemon_peer_sync::RemotePeer` 와 **필드명·JSON 형태가 정확히 일치**해야 한다
/// (그쪽이 `GET /v1/peers/reachable` 응답을 Deserialize 받음). transport 크레이트는
/// openxgram-db / openxgram-peer 에 의존하지 않으므로(저수준), 이 타입은 transport 안에
/// 독립 정의하고 daemon 이 provider closure 로 데이터를 주입한다(의존성 순환 방지).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReachablePeerDto {
    pub alias: String,
    pub public_key_hex: String,
    /// 0x… ECDSA 주소 — 식별 키.
    pub eth_address: String,
    /// http://<reachable-ip>:<port> — localhost 면 핸들러에서 제외.
    pub address: String,
    /// gui_address(transport+2) — cross-machine 터미널 proxy 용.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_address: Option<String>,
    /// "primary" / "secondary" / "worker".
    pub role: String,
    /// rc.369 — 정본 신원 편집(이름) cross-machine 전파용. 홈 머신이 권위.
    /// 편집 전파(P2 identity_update)는 편집된 에이전트 자신에게만 가므로, 다른 머신의
    /// 로스터 행에는 반영되지 않던 갭을 메운다 — 홈 머신이 gossip 으로 display_name 을 광고하고
    /// merge 가 홈-홈드 peer 행에 한해 갱신한다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// reachable peer 목록을 돌려주는 provider closure (이미 localhost 제외된 목록 가정).
/// daemon 이 자기 DB 를 읽어 주입한다. None 이면 빈 목록 응답(미주입 환경).
pub type ReachablePeerProvider = Arc<dyn Fn() -> Vec<ReachablePeerDto> + Send + Sync>;

#[derive(Clone)]
struct AppState {
    received: Arc<Mutex<Vec<Envelope>>>,
    started_at: std::time::Instant,
    metrics: Option<MetricsProvider>,
    replay: Arc<replay::ReplayCache>,
    rate_limiter: Arc<rate_limit::RateLimiter>,
    /// rc.238 — inbound_processor 의 마지막 tick (unix epoch secs). 0 = 미tick.
    last_inbound_tick: Arc<AtomicI64>,
    /// peer-sync gossip provider — None 이면 `/v1/peers/reachable` 가 빈 배열 반환.
    peer_provider: Option<ReachablePeerProvider>,
}

pub async fn spawn_server(bind_addr: SocketAddr) -> Result<ServerHandle> {
    spawn_server_inner(
        bind_addr,
        None,
        Arc::new(rate_limit::RateLimiter::default()),
        None,
    )
    .await
}

pub async fn spawn_server_with_metrics(
    bind_addr: SocketAddr,
    metrics: Option<MetricsProvider>,
) -> Result<ServerHandle> {
    spawn_server_inner(
        bind_addr,
        metrics,
        Arc::new(rate_limit::RateLimiter::default()),
        None,
    )
    .await
}

/// metrics + peer-sync provider 동시 주입. daemon 이 `/v1/peers/reachable` 활성화 시 사용.
/// `peer_provider` 는 이미 localhost 제외된 reachable peer 목록을 돌려준다(핸들러도 방어 필터).
pub async fn spawn_server_with_peer_provider(
    bind_addr: SocketAddr,
    metrics: Option<MetricsProvider>,
    peer_provider: Option<ReachablePeerProvider>,
) -> Result<ServerHandle> {
    spawn_server_inner(
        bind_addr,
        metrics,
        Arc::new(rate_limit::RateLimiter::default()),
        peer_provider,
    )
    .await
}

/// Explicit per-minute rate limit — bypasses `XGRAM_RATE_LIMIT_PER_MIN` env var.
/// Use this in tests so parallel cases don't race on shared process env.
pub async fn spawn_server_with_rate_limit(
    bind_addr: SocketAddr,
    per_minute: u32,
) -> Result<ServerHandle> {
    spawn_server_inner(
        bind_addr,
        None,
        Arc::new(rate_limit::RateLimiter::new(per_minute)),
        None,
    )
    .await
}

async fn spawn_server_inner(
    bind_addr: SocketAddr,
    metrics: Option<MetricsProvider>,
    rate_limiter: Arc<rate_limit::RateLimiter>,
    peer_provider: Option<ReachablePeerProvider>,
) -> Result<ServerHandle> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let last_inbound_tick = Arc::new(AtomicI64::new(0));
    let state = AppState {
        received: received.clone(),
        started_at: std::time::Instant::now(),
        metrics,
        replay: Arc::new(replay::ReplayCache::default()),
        rate_limiter,
        last_inbound_tick: last_inbound_tick.clone(),
        peer_provider,
    };

    let app = Router::new()
        .route("/v1/health", get(health_check))
        .route("/v1/message", post(receive_message))
        .route("/v1/metrics", get(metrics_endpoint))
        .route("/v1/peers/reachable", get(reachable_peers_endpoint))
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
        last_inbound_tick,
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
    /// rc.238 — inbound_processor 의 마지막 tick 이후 경과 (초).
    /// inbound_processor 가 stuck 이면 이 값이 계속 증가 → 외부 watchdog 가 120초+ 면 restart.
    /// None = daemon 가 inbound_processor 와 연결 안 됨 (아직 첫 tick 전 또는 미연동).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_inbound_tick_secs_ago: Option<u64>,
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
    // rc.238 — inbound_processor 마지막 tick 으로부터 경과 (초). 0 = 미tick (아직 None).
    let last_tick = state.last_inbound_tick.load(Ordering::Relaxed);
    let last_inbound_tick_secs_ago = if last_tick > 0 {
        let now = chrono::Utc::now().timestamp();
        Some((now - last_tick).max(0) as u64)
    } else {
        None
    };
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds,
        received_count,
        tailscale_state,
        tailscale_ipv4,
        last_inbound_tick_secs_ago,
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

/// `GET /v1/peers/reachable` — cross-machine peer-sync(gossip) 주소록 힌트.
/// read-only 주소록만 제공하므로 health/metrics 처럼 무인증 GET (설계노트 A).
/// provider 미주입(None) 환경이면 빈 배열 `[]`. provider 결과도 localhost 면 방어적 제외.
async fn reachable_peers_endpoint(State(state): State<AppState>) -> Json<Vec<ReachablePeerDto>> {
    let peers = match &state.peer_provider {
        Some(provider) => provider()
            .into_iter()
            .filter(|p| !crate::tailscale::is_unreachable_address(&p.address))
            .collect(),
        None => Vec::new(),
    };
    Json(peers)
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

#[cfg(test)]
mod peers_reachable_tests {
    use super::*;

    fn dto(alias: &str, addr: &str) -> ReachablePeerDto {
        ReachablePeerDto {
            alias: alias.to_string(),
            public_key_hex: "02abc".to_string(),
            eth_address: format!("0x{alias}"),
            address: addr.to_string(),
            gui_address: None,
            role: "worker".to_string(),
            display_name: None,
        }
    }

    /// 핸들러가 localhost/unreachable 주소를 방어적으로 제외하고 reachable 만 노출하는지.
    #[tokio::test]
    async fn endpoint_filters_localhost_and_returns_reachable() {
        let provider: ReachablePeerProvider = Arc::new(|| {
            vec![
                dto("local", "http://127.0.0.1:47300"),
                dto("zero", "http://0.0.0.0:47300"),
                dto("good", "http://100.101.237.9:47300"),
            ]
        });
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind");
        let server = spawn_server_with_peer_provider(bind, None, Some(provider))
            .await
            .expect("spawn");
        let url = format!("http://{}/v1/peers/reachable", server.bound_addr);
        let body: Vec<ReachablePeerDto> = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .expect("send")
            .json()
            .await
            .expect("json");
        assert_eq!(body.len(), 1, "localhost/0.0.0.0 제외, reachable 1 개만");
        assert_eq!(body[0].alias, "good");
        server.shutdown();
    }

    /// provider 미주입(None) 환경이면 빈 배열을 반환해야 한다.
    #[tokio::test]
    async fn endpoint_empty_without_provider() {
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind");
        let server = spawn_server(bind).await.expect("spawn");
        let url = format!("http://{}/v1/peers/reachable", server.bound_addr);
        let body: Vec<ReachablePeerDto> = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .expect("send")
            .json()
            .await
            .expect("json");
        assert!(body.is_empty(), "provider None → []");
        server.shutdown();
    }
}
