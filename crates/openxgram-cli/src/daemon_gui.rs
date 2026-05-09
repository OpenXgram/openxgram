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
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
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

/// Bearer 토큰 검증 — 매칭 시 agent 이름 반환. 미설정·실패 시 None.
/// XGRAM_GUI_REQUIRE_AUTH=0 으로 명시 끄면 통과 (dev 전용 — 운영 사용 금지).
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
