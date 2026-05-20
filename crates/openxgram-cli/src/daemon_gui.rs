//! `xgram daemon` 의 GUI HTTP API 서버 (`/v1/gui/*`).
//!
//! Tauri 데스크톱 앱(혹은 다른 클라이언트)이 원격에서 daemon 데이터를
//! 조회·조작하기 위한 REST 표면.
//!
//! 인증: `Authorization: Bearer <token>` — `mcp_tokens` 테이블 검증.
//! 동일 머신 loopback 도 토큰 강제 (실수로 외부 노출됐을 때의 방어선).
//!
//! Phase 2a-skeleton: `/v1/gui/status` 한 라우트만. 패턴 검증.
//! 후속 PR 에서 peers / channel / memory / payment 등 확장.
//!
//! 절대 규칙:
//! - silent fallback 금지: 토큰 검증 실패 시 401, 미설정 시 503 명시.
//! - localhost 외 bind 시 토큰 강제 (env override 없음).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use openxgram_core::paths::{db_path, manifest_path};
use openxgram_db::{Db, DbConfig};
use openxgram_manifest::InstallManifest;
use openxgram_payment::DailyLimitStore;
use openxgram_peer::{PeerRole, PeerStore};
use openxgram_vault::VaultStore;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct GuiServerState {
    data_dir: Arc<PathBuf>,
    /// daemon 가 한 DB 핸들을 long-lived 유지. 핸들러 호출 시 lock.
    db: Arc<Mutex<Db>>,
}

// (axum 의 layer middleware 가 URI 를 mutate 해도 router matching 은 재실행
// 되지 않는 알려진 동작 — `/api/*` → `/v1/*` rewrite 는 frontend 측에서 처리.
// rc.26 부터 client.ts/auth.ts 가 직접 `/v1/*` 호출.)

#[derive(Debug, Serialize)]
pub struct StatusDto {
    pub initialized: bool,
    pub alias: Option<String>,
    pub address: Option<String>,
    pub data_dir: String,
}

/// Tauri 의 `PeerDto` 와 동일 모양 — 클라이언트 측 양쪽 호환.
#[derive(Debug, Serialize)]
pub struct PeerDto {
    pub id: String,
    pub alias: String,
    pub address: String,
    pub public_key_hex: String,
    pub role: String,
    pub created_at: String,
    pub last_seen: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct ChannelAdapterStatus {
    pub platform: String,
    pub configured: bool,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct ChannelStatusDto {
    pub adapters: Vec<ChannelAdapterStatus>,
    pub peer_count: usize,
    pub schedule_pending: usize,
}

#[derive(Debug, Serialize)]
struct ErrorDto {
    error: String,
}

#[derive(Debug, Deserialize)]
pub struct PeerAddBody {
    pub alias: String,
    pub address: String,
    pub public_key_hex: String,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PendingDto {
    pub id: String,
    pub key: String,
    pub agent: String,
    pub action: String,
    pub status: String,
    pub requested_at: String,
}

#[derive(Debug, Deserialize)]
pub struct DenyBody {
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DailyLimitBody {
    pub micro_usdc: i64,
}

#[derive(Debug, Serialize, Default)]
pub struct NotifyStatusDto {
    pub telegram_configured: bool,
    pub discord_configured: bool,
    pub discord_webhook_configured: bool,
}

#[derive(Debug, Serialize)]
pub struct ScheduleDto {
    pub id: String,
    pub target_kind: String,
    pub target: String,
    pub payload: String,
    pub msg_type: String,
    pub schedule_kind: String,
    pub schedule_value: String,
    pub status: String,
    pub created_at_kst: i64,
    pub next_due_at_kst: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct ScheduleStatsDto {
    pub pending: usize,
    pub sent: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Debug, Deserialize)]
pub struct ScheduleCreateBody {
    pub target_kind: String,
    pub target: String,
    pub payload: String,
    pub msg_type: Option<String>,
    pub schedule_kind: String,
    pub schedule_value: String,
}

#[derive(Debug, Serialize)]
pub struct ChainDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at_kst: i64,
    pub enabled: bool,
    pub step_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ChainStepDto {
    pub step_order: i64,
    pub target_kind: String,
    pub target: String,
    pub payload: String,
    pub delay_secs: i64,
    pub condition_kind: Option<String>,
    pub condition_value: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChainDetailDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at_kst: i64,
    pub enabled: bool,
    pub steps: Vec<ChainStepDto>,
}

/// GUI HTTP 서버 가동 — 별도 axum 인스턴스, transport(47300) 와 분리된 포트.
pub async fn spawn_gui_server(data_dir: PathBuf, bind_addr: SocketAddr) -> Result<()> {
    let db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .context("daemon-gui DB open 실패")?;

    let state = GuiServerState {
        data_dir: Arc::new(data_dir),
        db: Arc::new(Mutex::new(db)),
    };

    let app = Router::new()
        .route("/v1/gui/health", get(gui_health))
        .route("/v1/gui/status", get(gui_status))
        .route("/v1/gui/initialized", get(gui_initialized))
        .route("/v1/gui/peers", get(gui_peers).post(gui_peer_add))
        // 메신저 v1.3 §3.2 — 머신×세션 통합 detector (M-1).
        .route("/v1/gui/sessions", get(gui_sessions))
        .route("/v1/gui/sessions/{identifier}/screen", get(gui_session_screen))
        .route("/v1/gui/machine", get(gui_machine_info))
        // 메신저 카드 v1.3 Step 0 — 메시지 송수신.
        .route("/v1/gui/messages", get(gui_messages_recent))
        .route("/v1/gui/peers/{alias}/send", post(gui_peer_send))
        .route("/v1/gui/channel/status", get(gui_channel_status))
        .route("/v1/gui/vault/pending", get(gui_vault_pending_list))
        .route(
            "/v1/gui/vault/pending/{id}/approve",
            post(gui_vault_pending_approve),
        )
        .route(
            "/v1/gui/vault/pending/{id}/deny",
            post(gui_vault_pending_deny),
        )
        .route(
            "/v1/gui/payment/daily-limit",
            get(gui_payment_get_limit).put(gui_payment_set_limit),
        )
        .route("/v1/gui/notify/status", get(gui_notify_status))
        // Discord 마법사 — token 검증 → 봇이 가입한 guild 목록 → 저장+테스트.
        .route(
            "/v1/gui/notify/discord/validate",
            post(gui_notify_discord_validate),
        )
        .route(
            "/v1/gui/notify/discord/guilds",
            post(gui_notify_discord_guilds),
        )
        .route("/v1/gui/notify/discord/save", post(gui_notify_discord_save))
        // Telegram 마법사 — token 검증 → chat_id 자동 감지 → 저장+테스트.
        .route(
            "/v1/gui/notify/telegram/validate",
            post(gui_notify_telegram_validate),
        )
        .route(
            "/v1/gui/notify/telegram/detect_chat",
            post(gui_notify_telegram_detect_chat),
        )
        .route(
            "/v1/gui/notify/telegram/save",
            post(gui_notify_telegram_save),
        )
        .route(
            "/v1/gui/schedule",
            get(gui_schedule_list).post(gui_schedule_create),
        )
        .route("/v1/gui/schedule/stats", get(gui_schedule_stats))
        .route("/v1/gui/chain", get(gui_chain_list))
        .route(
            "/v1/gui/chain/{name}",
            get(gui_chain_show).delete(gui_chain_delete),
        )
        .route("/v1/gui/schedule/{id}/cancel", post(gui_schedule_cancel))
        .route("/v1/agent/inject", post(agent_inject))
        // 단일 사용자 잠금 (PRD §1) — XGRAM_KEYSTORE_PASSWORD 와 비교, session_token 발급.
        // register/users 테이블·JWT 모두 폐기 (multi-user X — 사이드카는 1 사람용).
        .route("/v1/auth/unlock", post(auth_unlock))
        .route("/v1/auth/check", get(auth_check))
        // Web GUI 정적 자산 — xgram 바이너리에 임베드 (PRD-OpenXgram v1.3 §4.8).
        // nginx 외부 호스팅 불필요. 외부 노출은 Tailscale Funnel 또는 reverse proxy 위임.
        .route("/gui", get(crate::ui_assets::gui_root))
        .route("/gui/", get(crate::ui_assets::gui_root))
        .route("/gui/{*path}", get(crate::ui_assets::gui_asset_path))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("daemon-gui bind 실패: {bind_addr}"))?;
    let bound = listener.local_addr()?;
    tracing::info!(addr = %bound, "GUI HTTP API server bound");
    println!("  ✓ GUI HTTP API bound: http://{bound}");

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "daemon-gui server stopped");
        }
    });

    Ok(())
}

/// Bearer 토큰 검증 — session_token (웹 GUI) 또는 mcp-token (CLI/agent).
/// PRD §1: 1 사람 = 1 메인 daemon. multi-user X.
/// XGRAM_GUI_REQUIRE_AUTH=0 으로 명시 끄면 통과 (dev 전용).
async fn require_auth(
    state: &GuiServerState,
    headers: &HeaderMap,
) -> Result<Option<String>, StatusCode> {
    if std::env::var("XGRAM_GUI_REQUIRE_AUTH").as_deref() == Ok("0") {
        return Ok(None);
    }
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // 1) session_token (웹 GUI unlock) — 길이 64자 hex.
    if crate::auth::verify_session_token(token) {
        return Ok(Some("self".to_string()));
    }
    // 2) mcp-token (CLI/agent Bearer) fallback.
    let mut db = state.db.lock().await;
    match crate::mcp_tokens::verify_token(&mut db, token) {
        Ok(Some(agent)) => Ok(Some(agent)),
        Ok(None) => Err(StatusCode::UNAUTHORIZED),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// `GET /v1/gui/health` — 무인증 health check (load balancer / probe 용).
async fn gui_health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

/// `GET /v1/gui/status` — manifest 요약 + initialized 여부.
async fn gui_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<StatusDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(|s| {
        (
            s,
            Json(ErrorDto {
                error: "unauthorized — provide Authorization: Bearer <token>".into(),
            }),
        )
    })?;

    let mp = manifest_path(state.data_dir.as_ref());
    let dto = match InstallManifest::read(&mp) {
        Ok(m) => StatusDto {
            initialized: true,
            alias: Some(m.machine.alias),
            address: m.registered_keys.first().map(|k| k.address.clone()),
            data_dir: state.data_dir.display().to_string(),
        },
        Err(_) => StatusDto {
            initialized: false,
            alias: None,
            address: None,
            data_dir: state.data_dir.display().to_string(),
        },
    };
    Ok(Json(dto))
}

/// `GET /v1/gui/initialized` — manifest 존재 여부 (boolean).
async fn gui_initialized(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<bool>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mp = manifest_path(state.data_dir.as_ref());
    Ok(Json(mp.exists()))
}

/// `GET /v1/gui/peers` — 등록된 peer 전체 목록.
async fn gui_peers(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PeerDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut store = PeerStore::new(&mut db);
    let rows = store.list().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto {
                error: format!("peer list: {e}"),
            }),
        )
    })?;
    let dtos: Vec<PeerDto> = rows
        .into_iter()
        .map(|p| PeerDto {
            id: p.id,
            alias: p.alias,
            address: p.address,
            public_key_hex: p.public_key_hex,
            role: p.role.as_str().to_string(),
            created_at: p.created_at.to_rfc3339(),
            last_seen: p.last_seen.map(|t| t.to_rfc3339()),
        })
        .collect();
    Ok(Json(dtos))
}

/// `GET /v1/gui/sessions` — 머신×세션 통합 detector (UI-MESSENGER-SPEC v1.3 §3.2 M-1).
/// tmux + Claude Code projects 통합. xgram session 은 후속.
async fn gui_sessions(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_sessions::SessionsDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_sessions::collect_sessions()))
}

/// `GET /v1/gui/sessions/{identifier}/screen` — 세션 라이브 출력 (UI-MESSENGER-SPEC §4.3 S5).
/// tmux: capture-pane -e (ANSI). claude_project: .jsonl tail (포맷됨).
async fn gui_session_screen(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
) -> Result<Json<crate::daemon_gui_sessions::SessionScreenDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_sessions::capture_session(&identifier)))
}

/// `GET /v1/gui/machine` — 이 머신의 4-tuple machine part (UI-MESSENGER-SPEC L2).
async fn gui_machine_info(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_sessions::MachineInfo>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_sessions::detect_machine()))
}

/// `GET /v1/gui/channel/status` — notify.toml + DB 카운트 (peers, schedule pending).
async fn gui_channel_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<ChannelStatusDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let notify =
        crate::notify_setup::NotifyConfig::load(Some(state.data_dir.as_ref())).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorDto {
                    error: format!("NotifyConfig load: {e}"),
                }),
            )
        })?;
    let mut adapters = Vec::new();
    adapters.push(ChannelAdapterStatus {
        platform: "telegram".into(),
        configured: notify.telegram.is_some(),
        note: notify
            .telegram
            .as_ref()
            .map(|t| format!("chat_id={}", t.chat_id)),
    });
    adapters.push(ChannelAdapterStatus {
        platform: "discord".into(),
        configured: notify.discord.is_some(),
        note: notify.discord.as_ref().map(|d| {
            let mut parts = Vec::new();
            if let Some(c) = &d.channel_id {
                parts.push(format!("channel={c}"));
            }
            if d.webhook_url.is_some() {
                parts.push("webhook".into());
            }
            if parts.is_empty() {
                "(token only)".into()
            } else {
                parts.join(" + ")
            }
        }),
    });

    let mut db = state.db.lock().await;
    let peer_count = PeerStore::new(&mut db).list().map(|v| v.len()).unwrap_or(0);
    let schedule_pending = openxgram_orchestration::ScheduledStore::new(db.conn())
        .list(Some(openxgram_orchestration::ScheduledStatus::Pending))
        .map(|v| v.len())
        .unwrap_or(0);

    Ok(Json(ChannelStatusDto {
        adapters,
        peer_count,
        schedule_pending,
    }))
}

/// `POST /v1/gui/peers` — 새 peer 등록.
/// pubkey → keccak/EIP-55 eth_address 자동 도출 (PR #138 패턴 재사용).
async fn gui_peer_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<PeerAddBody>,
) -> Result<Json<PeerDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.alias.trim().is_empty()
        || body.address.trim().is_empty()
        || body.public_key_hex.trim().is_empty()
    {
        return Err(bad_request("alias/address/public_key_hex 필수"));
    }
    let eth_addr = crate::peer::eth_address_from_pubkey_hex(&body.public_key_hex)
        .map_err(|e| bad_request(&format!("public_key 파싱: {e}")))?;
    let mut db = state.db.lock().await;
    let p = PeerStore::new(&mut db)
        .add_with_eth(
            &body.alias,
            &body.public_key_hex,
            &body.address,
            Some(&eth_addr),
            PeerRole::Worker,
            body.notes.as_deref(),
        )
        .map_err(|e| internal(&format!("peer add: {e}")))?;
    Ok(Json(PeerDto {
        id: p.id,
        alias: p.alias,
        address: p.address,
        public_key_hex: p.public_key_hex,
        role: p.role.as_str().to_string(),
        created_at: p.created_at.to_rfc3339(),
        last_seen: p.last_seen.map(|t| t.to_rfc3339()),
    }))
}

/// `GET /v1/gui/vault/pending` — vault 의 pending 승인 요청 목록.
async fn gui_vault_pending_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PendingDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let rows = VaultStore::new(&mut db)
        .list_pending()
        .map_err(|e| internal(&format!("list_pending: {e}")))?;
    Ok(Json(
        rows.into_iter()
            .map(|p| PendingDto {
                id: p.id,
                key: p.key,
                agent: p.agent,
                action: p.action.as_str().to_string(),
                status: p.status.as_str().to_string(),
                requested_at: p.requested_at.to_rfc3339(),
            })
            .collect(),
    ))
}

/// `POST /v1/gui/vault/pending/:id/approve` — pending 승인.
async fn gui_vault_pending_approve(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    VaultStore::new(&mut db)
        .approve_confirmation(&id)
        .map_err(|e| internal(&format!("approve: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/gui/vault/pending/:id/deny` — pending 거부.
async fn gui_vault_pending_deny(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<DenyBody>>,
) -> Result<StatusCode, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let _ = body; // Phase 2: 거부 사유 컬럼 추가 후 기록.
    let mut db = state.db.lock().await;
    VaultStore::new(&mut db)
        .deny_confirmation(&id)
        .map_err(|e| internal(&format!("deny: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

// Tauri 측 handlers_core.rs 와 동일 키 — 단일 master/chain 단위.
const PAYMENT_LIMIT_AGENT: &str = "default";
const PAYMENT_LIMIT_CHAIN: &str = "base";

/// `GET /v1/gui/payment/daily-limit` — 현재 일일 USDC 한도 (micro USDC).
async fn gui_payment_get_limit(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<i64>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let row = DailyLimitStore::new(&mut db)
        .get(PAYMENT_LIMIT_AGENT, PAYMENT_LIMIT_CHAIN)
        .map_err(|e| internal(&format!("daily limit get: {e}")))?;
    Ok(Json(row.map(|r| r.daily_micro).unwrap_or(0)))
}

/// `PUT /v1/gui/payment/daily-limit` — 일일 USDC 한도 설정.
async fn gui_payment_set_limit(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<DailyLimitBody>,
) -> Result<StatusCode, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.micro_usdc < 0 {
        return Err(bad_request("micro_usdc 는 0 이상"));
    }
    let mut db = state.db.lock().await;
    DailyLimitStore::new(&mut db)
        .set(PAYMENT_LIMIT_AGENT, PAYMENT_LIMIT_CHAIN, body.micro_usdc)
        .map_err(|e| internal(&format!("daily limit set: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/gui/notify/status` — notify.toml 의 어댑터 설정 여부.
async fn gui_notify_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<NotifyStatusDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let cfg = crate::notify_setup::NotifyConfig::load(Some(state.data_dir.as_ref()))
        .map_err(|e| internal(&format!("NotifyConfig load: {e}")))?;
    Ok(Json(NotifyStatusDto {
        telegram_configured: cfg.telegram.is_some(),
        discord_configured: cfg.discord.is_some(),
        discord_webhook_configured: cfg
            .discord
            .as_ref()
            .and_then(|d| d.webhook_url.as_deref())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
    }))
}

// ── Messenger v1.3 Step 0 — 메시지 송수신 ────────────────────────────────

#[derive(Debug, Serialize)]
struct GuiMessageDto {
    id: String,
    session_id: String,
    sender: String,
    body: String,
    timestamp: String,
    conversation_id: String,
}

#[derive(Debug, Deserialize)]
struct GuiPeerSendBody {
    body: String,
    #[serde(default)]
    conversation_id: Option<String>,
}

/// `GET /v1/gui/messages?limit=N&sender=X` — L0 최근 메시지 (recv_messages MCP 도구의 HTTP 래퍼).
async fn gui_messages_recent(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Vec<GuiMessageDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let limit = q
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(50)
        .min(200);
    let sender_filter = q.get("sender").map(|s| s.to_lowercase());

    let mut db = state.db.lock().await;
    let embedder = openxgram_memory::default_embedder()
        .map_err(|e| internal(&format!("embedder: {e}")))?;
    let messages = openxgram_memory::MessageStore::new(&mut db, embedder.as_ref())
        .list_recent(limit * 4) // 필터 후 limit 충족 보장
        .map_err(|e| internal(&format!("list_recent: {e}")))?;

    let items: Vec<GuiMessageDto> = messages
        .into_iter()
        .filter(|m| match &sender_filter {
            Some(s) => m.sender.to_lowercase() == *s,
            None => true,
        })
        .take(limit)
        .map(|m| GuiMessageDto {
            id: m.id,
            session_id: m.session_id,
            sender: m.sender,
            body: m.body,
            timestamp: m.timestamp.to_rfc3339(),
            conversation_id: m.conversation_id,
        })
        .collect();
    Ok(Json(items))
}

/// `POST /v1/gui/peers/{alias}/send` — peer 에게 메시지 송신.
async fn gui_peer_send(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<GuiPeerSendBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // master 키 서명 위해 vault password 필요. daemon systemd unit 의 env 사용.
    let pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").map_err(|_| {
        internal("XGRAM_KEYSTORE_PASSWORD 미설정 — daemon 환경 변수 필요")
    })?;
    let data_dir = state.data_dir.as_ref().clone();
    crate::peer_send::run_peer_send_with_conv(
        &data_dir,
        &alias,
        None,
        &body.body,
        &pw,
        body.conversation_id,
    )
    .await
    .map_err(|e| internal(&format!("peer_send: {e}")))?;
    Ok(Json(serde_json::json!({"sent": true, "alias": alias})))
}

// ── Notify wizard (Discord/Telegram) HTTP endpoints ─────────────────────
// 동작: token 검증 / guild 자동조회 / 저장+테스트.
// Vault 저장은 notify.toml 만 갱신 (xgram setup discord CLI 와 동일 경로).

#[derive(Debug, Deserialize)]
struct NotifyTokenBody {
    token: String,
}

#[derive(Debug, Serialize)]
struct DiscordValidateResp {
    bot_label: String,
}

#[derive(Debug, Deserialize)]
struct DiscordSaveBody {
    token: String,
    #[serde(default)]
    guild_id: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    webhook_url: Option<String>,
    #[serde(default)]
    test_text: Option<String>,
}

#[derive(Debug, Serialize)]
struct SavedAtResp {
    saved_at: String,
}

#[derive(Debug, Serialize)]
struct TelegramValidateResp {
    bot_username: String,
}

#[derive(Debug, Deserialize)]
struct TelegramSaveBody {
    token: String,
    chat_id: String,
    #[serde(default)]
    test_text: Option<String>,
}

async fn gui_notify_discord_validate(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<NotifyTokenBody>,
) -> Result<Json<DiscordValidateResp>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let api_base = crate::notify_setup::discord_api_base();
    let bot = crate::notify_setup::discord_get_me(&api_base, &body.token)
        .await
        .map_err(|e| internal(&format!("discord validate: {e}")))?;
    let label = match (&bot.username, &bot.discriminator) {
        (Some(u), Some(d)) if d != "0" => format!("{u}#{d}"),
        (Some(u), _) => u.clone(),
        _ => "(unknown)".into(),
    };
    Ok(Json(DiscordValidateResp { bot_label: label }))
}

async fn gui_notify_discord_guilds(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<NotifyTokenBody>,
) -> Result<Json<Vec<crate::notify_setup::DiscordGuild>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let api_base = crate::notify_setup::discord_api_base();
    let guilds = crate::notify_setup::discord_list_guilds(&api_base, &body.token)
        .await
        .map_err(|e| internal(&format!("discord guilds: {e}")))?;
    Ok(Json(guilds))
}

async fn gui_notify_discord_save(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<DiscordSaveBody>,
) -> Result<Json<SavedAtResp>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut config = crate::notify_setup::NotifyConfig::load(Some(state.data_dir.as_ref()))
        .map_err(|e| internal(&format!("notify cfg load: {e}")))?;
    // guild_id 는 channel_id 가 비어 있을 때 fallback 식별자로 함께 저장 (참조용).
    let effective_channel = body
        .channel_id
        .clone()
        .or_else(|| body.guild_id.clone())
        .unwrap_or_default();
    config.discord = Some(crate::notify_setup::DiscordConfig {
        bot_token: body.token.clone(),
        channel_id: if effective_channel.is_empty() {
            None
        } else {
            Some(effective_channel)
        },
        webhook_url: body.webhook_url.clone(),
    });
    let saved_path = config
        .save(Some(state.data_dir.as_ref()))
        .map_err(|e| internal(&format!("notify cfg save: {e}")))?;

    if let (Some(url), Some(text)) = (&body.webhook_url, &body.test_text) {
        crate::notify_setup::discord_send_webhook(url, text)
            .await
            .map_err(|e| internal(&format!("discord webhook test: {e}")))?;
    }
    Ok(Json(SavedAtResp {
        saved_at: saved_path.display().to_string(),
    }))
}

async fn gui_notify_telegram_validate(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<NotifyTokenBody>,
) -> Result<Json<TelegramValidateResp>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let api_base = crate::notify_setup::telegram_api_base();
    let bot = crate::notify_setup::telegram_get_me(&api_base, &body.token)
        .await
        .map_err(|e| internal(&format!("telegram validate: {e}")))?;
    Ok(Json(TelegramValidateResp {
        bot_username: bot.username.unwrap_or_else(|| "(unknown)".into()),
    }))
}

async fn gui_notify_telegram_detect_chat(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<NotifyTokenBody>,
) -> Result<Json<Option<i64>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let api_base = crate::notify_setup::telegram_api_base();
    let chat = crate::notify_setup::telegram_detect_chat_id(&api_base, &body.token, 1)
        .await
        .map_err(|e| internal(&format!("telegram detect_chat: {e}")))?;
    Ok(Json(chat))
}

async fn gui_notify_telegram_save(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<TelegramSaveBody>,
) -> Result<Json<SavedAtResp>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut config = crate::notify_setup::NotifyConfig::load(Some(state.data_dir.as_ref()))
        .map_err(|e| internal(&format!("notify cfg load: {e}")))?;
    config.telegram = Some(crate::notify_setup::TelegramConfig {
        bot_token: body.token.clone(),
        chat_id: body.chat_id.clone(),
    });
    let saved_path = config
        .save(Some(state.data_dir.as_ref()))
        .map_err(|e| internal(&format!("notify cfg save: {e}")))?;

    if let Some(text) = &body.test_text {
        let api_base = crate::notify_setup::telegram_api_base();
        crate::notify_setup::telegram_send(&api_base, &body.token, &body.chat_id, text)
            .await
            .map_err(|e| internal(&format!("telegram test: {e}")))?;
    }
    Ok(Json(SavedAtResp {
        saved_at: saved_path.display().to_string(),
    }))
}

/// `GET /v1/gui/schedule` — 예약 메시지 전체 목록 (status 필터 없음).
async fn gui_schedule_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ScheduleDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let store = openxgram_orchestration::ScheduledStore::new(db.conn());
    let rows = store
        .list(None)
        .map_err(|e| internal(&format!("schedule list: {e}")))?;
    Ok(Json(
        rows.into_iter()
            .map(|m| ScheduleDto {
                id: m.id,
                target_kind: m.target_kind.as_str().to_string(),
                target: m.target,
                payload: m.payload,
                msg_type: m.msg_type,
                schedule_kind: m.schedule_kind.as_str().to_string(),
                schedule_value: m.schedule_value,
                status: m.status.as_str().to_string(),
                created_at_kst: m.created_at_kst,
                next_due_at_kst: m.next_due_at_kst,
                last_error: m.last_error,
            })
            .collect(),
    ))
}

/// `GET /v1/gui/schedule/stats` — pending/sent/failed/cancelled 카운트.
async fn gui_schedule_stats(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<ScheduleStatsDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let store = openxgram_orchestration::ScheduledStore::new(db.conn());
    let mut stats = ScheduleStatsDto::default();
    for status in [
        openxgram_orchestration::ScheduledStatus::Pending,
        openxgram_orchestration::ScheduledStatus::Sent,
        openxgram_orchestration::ScheduledStatus::Failed,
        openxgram_orchestration::ScheduledStatus::Cancelled,
    ] {
        let n = store.list(Some(status)).map(|v| v.len()).unwrap_or(0);
        match status {
            openxgram_orchestration::ScheduledStatus::Pending => stats.pending = n,
            openxgram_orchestration::ScheduledStatus::Sent => stats.sent = n,
            openxgram_orchestration::ScheduledStatus::Failed => stats.failed = n,
            openxgram_orchestration::ScheduledStatus::Cancelled => stats.cancelled = n,
        }
    }
    Ok(Json(stats))
}

/// `GET /v1/gui/chain` — 등록된 chain 목록 (각 step_count 포함).
async fn gui_chain_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ChainDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let store = openxgram_orchestration::ChainStore::new(db.conn());
    let chains = store
        .list()
        .map_err(|e| internal(&format!("chain list: {e}")))?;
    let mut out = Vec::with_capacity(chains.len());
    for c in chains {
        let steps = store
            .list_steps(&c.id)
            .map_err(|e| internal(&format!("chain list_steps: {e}")))?;
        out.push(ChainDto {
            id: c.id,
            name: c.name,
            description: c.description,
            created_at_kst: c.created_at_kst,
            enabled: c.enabled,
            step_count: steps.len(),
        });
    }
    Ok(Json(out))
}

/// `GET /v1/gui/chain/:name` — chain 상세 (steps 포함).
async fn gui_chain_show(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<ChainDetailDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let store = openxgram_orchestration::ChainStore::new(db.conn());
    let (chain, steps) = store
        .get_by_name(&name)
        .map_err(|e| internal(&format!("chain get_by_name: {e}")))?;
    Ok(Json(ChainDetailDto {
        id: chain.id,
        name: chain.name,
        description: chain.description,
        created_at_kst: chain.created_at_kst,
        enabled: chain.enabled,
        steps: steps
            .into_iter()
            .map(|s| ChainStepDto {
                step_order: s.step_order,
                target_kind: s.target_kind.as_str().to_string(),
                target: s.target,
                payload: s.payload,
                delay_secs: s.delay_secs,
                condition_kind: s.condition_kind.map(|c| c.as_str().to_string()),
                condition_value: s.condition_value,
            })
            .collect(),
    }))
}

/// `POST /v1/gui/schedule` — 새 예약 등록. 반환: 새 schedule id.
async fn gui_schedule_create(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<ScheduleCreateBody>,
) -> Result<Json<String>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.target.trim().is_empty()
        || body.payload.trim().is_empty()
        || body.schedule_value.trim().is_empty()
    {
        return Err(bad_request("target/payload/schedule_value 필수"));
    }
    let tk = match body.target_kind.as_str() {
        "role" => openxgram_orchestration::TargetKind::Role,
        "platform" => openxgram_orchestration::TargetKind::Platform,
        "self" => openxgram_orchestration::TargetKind::SelfTrigger,
        other => {
            return Err(bad_request(&format!(
                "target_kind '{other}' 허용 안 됨 (role|platform|self)"
            )))
        }
    };
    let sk = match body.schedule_kind.as_str() {
        "once" => openxgram_orchestration::ScheduleKind::Once,
        "cron" => openxgram_orchestration::ScheduleKind::Cron,
        other => {
            return Err(bad_request(&format!(
                "schedule_kind '{other}' 허용 안 됨 (once|cron)"
            )))
        }
    };
    let mt = body.msg_type.unwrap_or_else(|| "info".into());
    let mut db = state.db.lock().await;
    let id = openxgram_orchestration::ScheduledStore::new(db.conn())
        .insert(
            tk,
            &body.target,
            &body.payload,
            &mt,
            sk,
            &body.schedule_value,
        )
        .map_err(|e| internal(&format!("schedule insert: {e}")))?;
    Ok(Json(id))
}

/// `POST /v1/gui/schedule/:id/cancel` — 예약 취소.
async fn gui_schedule_cancel(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    openxgram_orchestration::ScheduledStore::new(db.conn())
        .cancel(&id)
        .map_err(|e| internal(&format!("schedule cancel: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/gui/chain/:name` — chain 삭제 (steps cascade).
async fn gui_chain_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    openxgram_orchestration::ChainStore::new(db.conn())
        .delete_by_name(&name)
        .map_err(|e| internal(&format!("chain delete: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

fn bad_request(msg: &str) -> (StatusCode, Json<ErrorDto>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorDto { error: msg.into() }),
    )
}

fn internal(msg: &str) -> (StatusCode, Json<ErrorDto>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorDto { error: msg.into() }),
    )
}

fn unauthorized(s: StatusCode) -> (StatusCode, Json<ErrorDto>) {
    (
        s,
        Json(ErrorDto {
            error: "unauthorized — provide Authorization: Bearer <token>".into(),
        }),
    )
}

// ── 사용자 인증 (POST /v1/auth/{register, login, logout} + GET /v1/auth/me) ──

#[derive(Debug, Deserialize)]
pub struct AuthRegisterBody {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AuthLoginBody {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthIssuedDto {
    pub user_id: String,
    pub email: String,
    pub alias: Option<String>,
    pub role: String,
    pub jwt_token: String,
}

#[derive(Debug, Serialize)]
pub struct AuthMeDto {
    pub user_id: String,
    pub email: String,
    pub alias: Option<String>,
    pub role: String,
    pub machine_alias: Option<String>,
}

/// `POST /v1/auth/unlock` — keystore 비밀번호 검증 후 session_token 발급.
/// PRD §1: 1 사람 = 1 메인 daemon. multi-user X, register X.
async fn auth_unlock(
    Json(body): Json<crate::auth::UnlockRequest>,
) -> Result<Json<crate::auth::UnlockResponse>, (StatusCode, Json<ErrorDto>)> {
    if !crate::auth::verify_password(&body.password) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorDto {
                error: "비밀번호가 틀렸습니다".into(),
            }),
        ));
    }
    Ok(Json(crate::auth::UnlockResponse {
        session_token: crate::auth::session_token().to_string(),
    }))
}

/// `GET /v1/auth/check` — session_token 유효성 확인. Bearer 필수.
async fn auth_check(headers: HeaderMap) -> Result<Json<serde_json::Value>, StatusCode> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if crate::auth::verify_session_token(token) {
        Ok(Json(serde_json::json!({"ok": true})))
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[derive(Debug, Deserialize)]
pub struct AgentInjectBody {
    /// 발신자 식별자 — `discord:<userid>`, `telegram:<chatid>`, `cli:<alias>` 등.
    pub sender: String,
    pub body: String,
    /// 옵션 — 기존 대화에 이어 붙일 conversation_id. 미지정 시 새 conversation 생성.
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentInjectResponse {
    pub message_id: String,
    pub session_id: String,
    pub conversation_id: String,
}

/// `POST /v1/agent/inject` — 외부 채널 (Discord/Telegram/...) 또는 self-trigger 메시지를 daemon inbox 로 주입.
///
/// 서명 검증을 거치지 않는다 (외부 소스 unsigned). 대신 mcp_token Bearer 로 외부 호출 권한 통제.
/// 저장 흐름:
/// - session: `inbox-from-{sender}` ensure
/// - L0 message: sender, body, signature="external"
async fn agent_inject(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AgentInjectBody>,
) -> Result<Json<AgentInjectResponse>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    if body.sender.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: "sender 비어있음".into(),
            }),
        ));
    }
    if body.body.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: "body 비어있음".into(),
            }),
        ));
    }

    let mut db = state.db.lock().await;
    let embedder = openxgram_memory::default_embedder().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto {
                error: format!("embedder init: {e}"),
            }),
        )
    })?;

    let session_title = format!("inbox-from-{}", body.sender);
    let session = openxgram_memory::SessionStore::new(&mut db)
        .ensure_by_title(&session_title, "inbound")
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorDto {
                    error: format!("session ensure: {e}"),
                }),
            )
        })?;

    let msg = openxgram_memory::MessageStore::new(&mut db, embedder.as_ref())
        .insert(
            &session.id,
            &body.sender,
            &body.body,
            "external",
            body.conversation_id.as_deref(),
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorDto {
                    error: format!("message insert: {e}"),
                }),
            )
        })?;

    Ok(Json(AgentInjectResponse {
        message_id: msg.id,
        session_id: session.id,
        conversation_id: msg.conversation_id,
    }))
}
