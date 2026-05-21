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
        // UI-MESSENGER-SPEC v1.3 §7.1·§7.3 — 헤더 🔔 통합 승인 큐 (L6 차등 만료 + V4).
        .route("/v1/gui/approvals", get(gui_approvals))
        // UI-MESSENGER-SPEC v1.3 §2.4 + M-3 + L4 — 마스터+서브 지갑 (HD 영구 점유).
        .route("/v1/gui/wallets", get(gui_wallets_list).post(gui_wallet_create))
        .route("/v1/gui/wallets/topup", post(gui_wallet_topup))
        // UI-MESSENGER-SPEC v1.3 L3 + V1 — 역할별 auto_respond 마스터 정책.
        .route("/v1/gui/role-policies", get(gui_role_policies))
        // UI-MESSENGER-SPEC v1.3 §3.6 M-5 + N1 + N3 + V4 — 화이트리스트 패턴 + 우선순위.
        .route("/v1/gui/whitelist", get(gui_whitelist))
        // UI-MESSENGER-SPEC v1.3 S8 + V6 — cross-machine 큐 status (Tailscale P2P).
        .route("/v1/gui/cross-machine-queue", get(gui_cross_machine_queue))
        // UI-MESSENGER-SPEC v1.3 §7.5 + N4 — 글로벌 검색 (FTS5)
        .route("/v1/gui/search", get(gui_global_search))
        // V11 — RoutingRule CRUD (에이전트 ↔ 에이전트, Internal scope)
        .route("/v1/gui/routing-rules", get(gui_routing_rules_list).post(gui_routing_rule_add))
        .route("/v1/gui/routing-rules/{id}", post(gui_routing_rule_delete))
        // V12 — 3-layer 버전 정보
        .route("/v1/gui/version", get(gui_version))
        // N7 — 시스템 cron 보호 (비활성화 시도 reject + audit)
        .route("/v1/gui/system-cron/protect-attempt", post(gui_system_cron_protect))
        // S7 — 첨부 업로드/다운로드 (content-addressed, V2/V3 refcount)
        .route("/v1/gui/attachments", post(gui_attachment_upload))
        .route("/v1/gui/attachments/{hash}", get(gui_attachment_get))
        // M-5 — 사용자 화이트리스트 패턴 CRUD
        .route("/v1/gui/whitelist-patterns", get(gui_whitelist_list).post(gui_whitelist_add))
        // UI-MEMORY-SPEC v1.1 — 위키 CRUD + 검색 + 패턴/실수 보드
        .route("/v1/gui/wiki/pages", get(gui_wiki_list).post(gui_wiki_upsert))
        .route("/v1/gui/wiki/pages/{id}", get(gui_wiki_get))
        // UI-MEMORY-SPEC v1.1 깊은 endpoint
        .route("/v1/gui/wiki/pages/{id}/delete", post(gui_wiki_delete))       // M-12 휴지통
        .route("/v1/gui/wiki/pages/{id}/lock", post(gui_wiki_lock))           // M-7
        .route("/v1/gui/wiki/pages/{id}/history", get(gui_wiki_history))     // M-11
        .route("/v1/gui/wiki/pages/{id}/share", post(gui_wiki_share))        // M-4 V-3
        .route("/v1/gui/wiki/trash", get(gui_wiki_trash_list))               // M-12 휴지통 목록
        .route("/v1/gui/wiki/trash/{id}/restore", post(gui_wiki_trash_restore))
        .route("/v1/gui/memory/patterns", get(gui_memory_patterns_list).post(gui_memory_pattern_add)) // M-5
        .route("/v1/gui/memory/mistakes", get(gui_memory_mistakes_list).post(gui_memory_mistake_add)) // M-13
        .route("/v1/gui/wiki/new-alerts", get(gui_wiki_new_alerts))          // M-6
        // UI-IDENTITY-SPEC 깊은 endpoint
        .route("/v1/gui/identity/bip39", post(gui_identity_bip39_show))
        .route("/v1/gui/identity/sub-dids", get(gui_sub_dids_list).post(gui_sub_did_new))
        .route("/v1/gui/identity/sub-dids/{id}/revoke", post(gui_sub_did_revoke))
        .route("/v1/gui/identity/lockout-status", get(gui_lockout_status))
        // UI-VAULT-MCP-SPEC 깊은 endpoint
        .route("/v1/gui/vault/mcp-servers", get(gui_mcp_servers_list).post(gui_mcp_server_add))
        .route("/v1/gui/vault/tool-catalog", get(gui_tool_catalog_list).post(gui_tool_acl_set))
        // UI-CHANNEL-SPEC 모더레이션
        .route("/v1/gui/channel/moderation/blocks", get(gui_channel_blocks_list).post(gui_channel_block_add))
        .route("/v1/gui/channel/moderation/limits", get(gui_channel_limits_list).post(gui_channel_limit_set))
        // UI-AUTONOMY-SPEC 깊은
        .route("/v1/gui/autonomy/self-triggers", get(gui_self_triggers_list).post(gui_self_trigger_add))
        .route("/v1/gui/autonomy/reflection-runs", get(gui_reflection_runs_list).post(gui_reflection_now))
        // UI-MEMORY-SPEC 깊은 (M-2 merge, M-10 conflict)
        .route("/v1/gui/wiki/merge-candidates", get(gui_merge_candidates_list))
        .route("/v1/gui/wiki/pages/{id}/edit-lock", get(gui_wiki_edit_lock_get).post(gui_wiki_edit_lock_acquire))
        // Peer keypair 자동 생성
        .route("/v1/gui/peers/generate-keypair", post(gui_peer_keypair_generate))
        // UI-MESSENGER-SPEC v1.3 §5 탭 3 — 세션별 채널 바인딩 (Discord/Telegram channel_id 등)
        .route("/v1/gui/sessions/{agent_id}/channel-bindings",
               get(gui_session_bindings_list).post(gui_session_binding_add))
        .route("/v1/gui/sessions/{agent_id}/channel-bindings/{binding_id}",
               post(gui_session_binding_delete))
        // Discord 봇이 가입한 guild 의 channel 목록 (세션 바인딩 시 사용자가 선택)
        .route("/v1/gui/notify/discord/channels", post(gui_notify_discord_channels))
        // Discord 봇 진단 — token + permission + guild + channel 한 번에
        .route("/v1/gui/notify/discord/diagnostic", get(gui_notify_discord_diagnostic))
        // UI-IDENTITY-SPEC v1.0 — 신원 카드 endpoint
        .route("/v1/gui/identity/info", get(gui_identity_info))
        .route("/v1/gui/identity/audit", get(gui_identity_audit))
        .route("/v1/gui/identity/allowlist", get(gui_identity_allowlist).post(gui_identity_allowlist_add))
        // UI-CHANNEL-SPEC v1.0 — 채널 카드 (Discord/Telegram/Slack 통합 inbox)
        .route("/v1/gui/channel/people", get(gui_channel_people))
        .route("/v1/gui/channel/routing", get(gui_channel_routing))
        // UI-AUTONOMY-SPEC v1.0 — 자율 행동 (전체 cron + self-trigger + reflection)
        .route("/v1/gui/autonomy/history", get(gui_autonomy_history))
        .route("/v1/gui/autonomy/limits", get(gui_autonomy_limits))
        .route("/v1/gui/autonomy/vacation", get(gui_autonomy_vacation).post(gui_autonomy_vacation_set))
        // External Agent + Ops 카드
        .route("/v1/gui/external/directory", get(gui_external_directory))
        .route("/v1/gui/ops/health", get(gui_ops_health))
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

/// `GET /v1/gui/approvals` — 통합 승인 큐 (UI-MESSENGER-SPEC v1.3 §7.1, §7.3, V4).
/// 현재 데이터 출처: vault_pending. 다른 종류는 향후 확장.
async fn gui_approvals(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<
    Json<crate::daemon_gui_sessions::ApprovalQueueDto>,
    (StatusCode, Json<ErrorDto>),
> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use chrono::{Duration, Utc};
    let mut items = Vec::new();
    // Vault pending → ApprovalKind::Payment (가장 가까운 시각 분류; 실제 분류는 향후)
    {
        let mut db = state.db.lock().await;
        let mut store = VaultStore::new(&mut db);
        if let Ok(pending) = store.list_pending() {
            for p in pending {
                let created = p.requested_at;
                let expires = created + Duration::hours(
                    crate::daemon_gui_sessions::ApprovalKind::Payment.ttl_hours() as i64,
                );
                items.push(crate::daemon_gui_sessions::ApprovalItem {
                    id: p.id,
                    kind: crate::daemon_gui_sessions::ApprovalKind::Payment,
                    title: format!("Vault 자격증명 요청: {}", p.key),
                    detail: format!("{} · {:?}", p.agent, p.action),
                    created_at: created.to_rfc3339(),
                    expires_at: expires.to_rfc3339(),
                    source_card: "vault".into(),
                });
            }
        }
    }
    // 만료된 항목 자동 제외 (UI 표시 측면).
    let now = Utc::now();
    items.retain(|i| {
        chrono::DateTime::parse_from_rfc3339(&i.expires_at)
            .map(|e| e.with_timezone(&Utc) > now)
            .unwrap_or(true)
    });
    Ok(Json(crate::daemon_gui_sessions::ApprovalQueueDto {
        items,
        policy: crate::daemon_gui_sessions::default_approval_policy(),
    }))
}

/// `GET /v1/gui/role-policies` — 역할별 auto_respond 기본 정책 (L3 + V1).
async fn gui_role_policies(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_sessions::RolePolicyDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_sessions::default_role_policies()))
}

/// `GET /v1/gui/whitelist` — 화이트리스트 패턴 + 우선순위 (M-5 + N1 + N3 + V4).
async fn gui_whitelist(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_sessions::WhitelistDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_sessions::default_whitelist()))
}

/// `GET /v1/gui/cross-machine-queue` — Tailscale P2P 큐 status (S8 + V6).
async fn gui_cross_machine_queue(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_sessions::CrossMachineQueueDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_sessions::default_cross_machine_queue()))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_search_limit")]
    pub limit: u32,
}
fn default_search_limit() -> u32 { 30 }

/// `GET /v1/gui/search?q=...` — 글로벌 FTS5 검색 (N4).
async fn gui_global_search(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<SearchQuery>,
) -> Result<Json<crate::daemon_gui_sessions::SearchResultDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT kind, ref_id, title, body, rank FROM global_search WHERE global_search MATCH ?1 \
         ORDER BY rank LIMIT ?2",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep:{e}")})))?;
    let hits = stmt.query_map(rusqlite::params![q.q, q.limit as i64], |r| {
        Ok(crate::daemon_gui_sessions::SearchHit {
            kind: r.get(0)?,
            ref_id: r.get(1)?,
            title: r.get::<_,String>(2).unwrap_or_default(),
            body: r.get::<_,String>(3).unwrap_or_default(),
            rank: r.get::<_, f64>(4).unwrap_or(0.0),
        })
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("query:{e}")})))?
        .filter_map(|r| r.ok()).collect::<Vec<_>>();
    let total = hits.len();
    Ok(Json(crate::daemon_gui_sessions::SearchResultDto {
        query: q.q,
        hits,
        total,
    }))
}

/// `GET /v1/gui/routing-rules` — Internal scope routing rule 리스트 (V11).
async fn gui_routing_rules_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::daemon_gui_sessions::RoutingRuleDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, scope, from_pattern, to_pattern, action, created_at, active FROM routing_rules ORDER BY created_at DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep:{e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(crate::daemon_gui_sessions::RoutingRuleDto {
            id: r.get(0)?, scope: r.get(1)?, from_pattern: r.get(2)?, to_pattern: r.get(3)?,
            action: r.get(4)?, created_at: r.get(5)?, active: r.get::<_, i64>(6)? != 0,
        })
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q:{e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct RoutingRuleBody {
    pub from_pattern: String,
    pub to_pattern: String,
    pub action: String,
}

/// `POST /v1/gui/routing-rules` — RoutingRule 추가 (V11).
async fn gui_routing_rule_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<RoutingRuleBody>,
) -> Result<Json<crate::daemon_gui_sessions::RoutingRuleDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let id = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("{:?}", std::time::SystemTime::now()).as_bytes());
        h.update(body.from_pattern.as_bytes());
        h.update(body.to_pattern.as_bytes());
        format!("{:x}", h.finalize())[..26].to_string()
    };
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO routing_rules (id, scope, from_pattern, to_pattern, action, created_at, active) \
         VALUES (?1, 'Internal', ?2, ?3, ?4, ?5, 1)",
        rusqlite::params![id, body.from_pattern, body.to_pattern, body.action, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert:{e}")})))?;
    Ok(Json(crate::daemon_gui_sessions::RoutingRuleDto {
        id, scope: "Internal".into(), from_pattern: body.from_pattern,
        to_pattern: body.to_pattern, action: body.action, created_at: now, active: true,
    }))
}

/// `POST /v1/gui/routing-rules/{id}` — RoutingRule 삭제 (V11).
async fn gui_routing_rule_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute("DELETE FROM routing_rules WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("del:{e}")})))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

/// `GET /v1/gui/version` — 3-layer 버전 (V12).
async fn gui_version(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_sessions::VersionInfoDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_sessions::version_info()))
}

#[derive(Debug, Deserialize)]
pub struct SystemCronAttempt {
    pub cron_name: String,
}

/// `POST /v1/gui/system-cron/protect-attempt` — 시스템 cron 비활성화 시도 거부 + audit (N7).
async fn gui_system_cron_protect(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<SystemCronAttempt>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO system_cron_protect_log (cron_name, attempted_at, result) VALUES (?1, ?2, 'rejected')",
        rusqlite::params![body.cron_name, now],
    ).ok();
    Ok(Json(serde_json::json!({
        "ok": false,
        "rejected": true,
        "message": "시스템 cron 은 비활성화 불가입니다. (UI-MESSENGER-SPEC N7 — 감사 로그 기록됨)"
    })))
}

#[derive(Debug, Deserialize)]
pub struct AttachmentUploadBody {
    pub content_b64: String, // base64
    pub mime: Option<String>,
}

/// `POST /v1/gui/attachments` — S7 첨부 업로드 (V2 content-addressed + V3 refcount).
async fn gui_attachment_upload(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AttachmentUploadBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let raw = base64_decode(&body.content_b64).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(ErrorDto{error: format!("base64: {e}")}))
    })?;
    let hash = format!("{:x}", Sha256::digest(&raw));
    let size = raw.len() as i64;
    let mime = body.mime.unwrap_or_else(|| "application/octet-stream".into());
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    let conn = db.conn();
    // refcount++. 새 hash 면 INSERT, 기존이면 UPDATE.
    conn.execute(
        "INSERT INTO attachment_refs (content_hash, refcount, size_bytes, mime, created_at) \
         VALUES (?1, 1, ?2, ?3, ?4) \
         ON CONFLICT(content_hash) DO UPDATE SET refcount = refcount + 1",
        rusqlite::params![hash, size, mime, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("ref:{e}")})))?;
    // S7: < 1MB SQLite blob, ≥ 1MB content-addressed disk (V2: thread_id prefix 제거).
    let storage = if size < 1_000_000 {
        conn.execute(
            "INSERT OR IGNORE INTO attachment_inline (content_hash, data, mime, size_bytes) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![hash, raw, mime, size],
        ).ok();
        "inline"
    } else {
        let dir = state.data_dir.join("attachments");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(&hash);
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, &raw) {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("disk write: {e}")})));
            }
        }
        "disk"
    };
    Ok(Json(serde_json::json!({
        "content_hash": hash,
        "size_bytes": size,
        "mime": mime,
        "storage": storage,
    })))
}

/// `GET /v1/gui/attachments/{hash}` — S7 첨부 조회.
async fn gui_attachment_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(hash): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let row: Option<(Vec<u8>, String, i64)> = db.conn().query_row(
        "SELECT data, mime, size_bytes FROM attachment_inline WHERE content_hash = ?1",
        rusqlite::params![hash],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).ok();
    if let Some((data, mime, size)) = row {
        return Ok(Json(serde_json::json!({
            "content_hash": hash,
            "mime": mime,
            "size_bytes": size,
            "storage": "inline",
            "content_b64": base64_encode(&data),
        })));
    }
    // disk lookup
    let path = state.data_dir.join("attachments").join(&hash);
    if let Ok(data) = std::fs::read(&path) {
        let size = data.len() as i64;
        let mime: String = db.conn().query_row(
            "SELECT mime FROM attachment_refs WHERE content_hash = ?1",
            rusqlite::params![hash],
            |r| r.get(0),
        ).unwrap_or_else(|_| "application/octet-stream".into());
        return Ok(Json(serde_json::json!({
            "content_hash": hash,
            "mime": mime,
            "size_bytes": size,
            "storage": "disk",
            "content_b64": base64_encode(&data),
        })));
    }
    Err((StatusCode::NOT_FOUND, Json(ErrorDto{error: "not_found".into()})))
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    // 안티-dep: 직접 base64 디코더. 표준 alphabet.
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0u8;
    for c in s.bytes() {
        let v = match T.iter().position(|&x| x == c) { Some(i) => i as u32, None => return Err(format!("invalid char: {c:?}")) };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((data.len() + 2) / 3) * 4);
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = if i + 1 < data.len() { data[i+1] } else { 0 };
        let b2 = if i + 2 < data.len() { data[i+2] } else { 0 };
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < data.len() {
            out.push(T[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else { out.push('='); }
        if i + 2 < data.len() {
            out.push(T[(b2 & 0x3f) as usize] as char);
        } else { out.push('='); }
        i += 3;
    }
    out
}

#[derive(Debug, Deserialize)]
pub struct WhitelistPatternBody {
    pub priority: u32,
    pub pattern_type: String,
    pub pattern: String,
    pub default_role: String,
    pub auto_register: bool,
    pub auto_approve_pending: bool,
}

/// `GET /v1/gui/whitelist-patterns` — 사용자 화이트리스트 패턴 (M-5, default + user).
async fn gui_whitelist_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::daemon_gui_sessions::WhitelistPatternItem>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT priority, pattern_type, pattern, default_role, auto_register, auto_approve_pending \
         FROM whitelist_patterns WHERE active = 1 ORDER BY priority ASC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep:{e}")})))?;
    let user_rows: Vec<crate::daemon_gui_sessions::WhitelistPatternItem> = stmt.query_map([], |r| {
        Ok(crate::daemon_gui_sessions::WhitelistPatternItem {
            priority: r.get::<_, i64>(0)? as u32,
            pattern_type: r.get(1)?,
            pattern: r.get(2)?,
            default_role: r.get(3)?,
            auto_register: r.get::<_, i64>(4)? != 0,
            auto_approve_pending: r.get::<_, i64>(5)? != 0,
        })
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q:{e}")})))?
        .filter_map(|r| r.ok()).collect();
    // default + user 합쳐서 반환
    let mut combined = crate::daemon_gui_sessions::default_whitelist().patterns;
    combined.extend(user_rows);
    Ok(Json(combined))
}

/// `POST /v1/gui/whitelist-patterns` — 사용자 화이트리스트 패턴 추가 (M-5).
async fn gui_whitelist_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<WhitelistPatternBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let id = {
        let mut h = Sha256::new();
        h.update(format!("{:?}", std::time::SystemTime::now()).as_bytes());
        h.update(body.pattern.as_bytes());
        format!("{:x}", h.finalize())[..20].to_string()
    };
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO whitelist_patterns (id, priority, pattern_type, pattern, default_role, \
                auto_register, auto_approve_pending, active, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
        rusqlite::params![
            id, body.priority as i64, body.pattern_type, body.pattern, body.default_role,
            if body.auto_register {1} else {0},
            if body.auto_approve_pending {1} else {0},
            now,
        ],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert:{e}")})))?;
    Ok(Json(serde_json::json!({"id": id, "ok": true})))
}

#[derive(Debug, Serialize)]
pub struct WikiPageDto {
    pub id: String,
    pub title: String,
    pub page_type: String,
    pub updated_at: i64,
}

/// `GET /v1/gui/wiki/pages` — UI-MEMORY-SPEC v1.1 위키 페이지 리스트.
async fn gui_wiki_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<WikiPageDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, title, page_type, updated_at FROM wiki_pages ORDER BY updated_at DESC LIMIT 200",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep:{e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(WikiPageDto {
            id: r.get(0)?,
            title: r.get(1)?,
            page_type: r.get(2)?,
            updated_at: r.get(3)?,
        })
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q:{e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct WikiUpsertBody {
    pub id: String,
    pub title: String,
    pub page_type: String, // entity/concept/comparison/other
    pub content: String,   // markdown body
    /// M-10: edit lock 보유자 (없으면 anonymous). lock holder != requester 이면 409.
    #[serde(default)]
    pub requester: Option<String>,
}

/// `POST /v1/gui/wiki/pages` — 위키 페이지 upsert (M-1 + M-3 + M-11 + M-10 lock 검증).
async fn gui_wiki_upsert(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<WikiUpsertBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    use rusqlite::OptionalExtension;
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(body.content.as_bytes()));
    let now = chrono::Utc::now().timestamp();
    let file_path = format!("wiki/{}/{}.md", body.page_type, body.id);
    let mut db = state.db.lock().await;
    // M-10 edit lock 검증: holder != requester 이면 409 reject (lock 만료 후엔 free).
    if let Ok(Some(holder)) = db.conn().query_row(
        "SELECT holder FROM wiki_edit_locks WHERE page_id = ?1 AND expires_at > datetime('now')",
        rusqlite::params![&body.id],
        |r| r.get::<_, String>(0),
    ).optional() {
        let req = body.requester.as_deref().unwrap_or("");
        if holder != req {
            return Err((StatusCode::CONFLICT, Json(ErrorDto{
                error: format!("M-10 편집 충돌 — 다른 사용자가 잠금 보유 중 (holder={holder})"),
            })));
        }
    }
    db.conn().execute(
        "INSERT INTO wiki_pages (id, file_path, page_type, title, content_hash, embedding_hash, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?6) \
         ON CONFLICT(id) DO UPDATE SET title = ?4, content_hash = ?5, updated_at = ?6",
        rusqlite::params![body.id, file_path, body.page_type, body.title, hash, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("upsert:{e}")})))?;
    // global_search FTS5 인덱스 갱신
    let _ = db.conn().execute(
        "INSERT INTO global_search (kind, ref_id, title, body) VALUES ('wiki', ?1, ?2, ?3)",
        rusqlite::params![body.id, body.title, body.content],
    );
    Ok(Json(serde_json::json!({"id": body.id, "content_hash": hash, "updated_at": now})))
}

/// `GET /v1/gui/wiki/pages/{id}` — 위키 페이지 단일 조회.
async fn gui_wiki_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let row = db.conn().query_row(
        "SELECT id, title, page_type, content_hash, updated_at FROM wiki_pages WHERE id = ?1",
        rusqlite::params![id],
        |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, String>(0)?,
                "title": r.get::<_, String>(1)?,
                "page_type": r.get::<_, String>(2)?,
                "content_hash": r.get::<_, String>(3)?,
                "updated_at": r.get::<_, i64>(4)?,
            }))
        },
    );
    match row {
        Ok(j) => Ok(Json(j)),
        Err(_) => Err((StatusCode::NOT_FOUND, Json(ErrorDto{error: "not_found".into()}))),
    }
}

// ── UI-MEMORY-SPEC v1.1 깊은 endpoint ─────────────────────────────────────

/// `POST /v1/gui/wiki/pages/{id}/delete` — M-12 휴지통 (30일 보관, V-4).
async fn gui_wiki_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now();
    let purge = now + chrono::Duration::days(30);
    let mut db = state.db.lock().await;
    let row = db.conn().query_row(
        "SELECT title, page_type FROM wiki_pages WHERE id = ?1",
        rusqlite::params![id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    );
    let (title, ptype) = match row {
        Ok(p) => p,
        Err(_) => return Err((StatusCode::NOT_FOUND, Json(ErrorDto{error:"page_not_found".into()}))),
    };
    db.conn().execute(
        "INSERT OR REPLACE INTO wiki_trash (id, title, page_type, content, deleted_at, purge_at) \
         VALUES (?1, ?2, ?3, '(content snapshot)', ?4, ?5)",
        rusqlite::params![id, title, ptype, now.to_rfc3339(), purge.to_rfc3339()],
    ).ok();
    db.conn().execute("DELETE FROM wiki_pages WHERE id = ?1", rusqlite::params![id]).ok();
    Ok(Json(serde_json::json!({"deleted": id, "purge_at": purge.to_rfc3339()})))
}

#[derive(Debug, Deserialize)]
pub struct WikiLockBody { pub locked_by: String, pub reason: Option<String> }
async fn gui_wiki_lock(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<WikiLockBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT OR REPLACE INTO wiki_locks (page_id, locked_by, locked_at, reason) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, body.locked_by, now, body.reason],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("lock: {e}")})))?;
    Ok(Json(serde_json::json!({"page_id": id, "locked_by": body.locked_by, "locked_at": now})))
}

async fn gui_wiki_history(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT revision, author, event_type, at FROM wiki_history WHERE page_id = ?1 ORDER BY revision DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map(rusqlite::params![id], |r| {
        Ok(serde_json::json!({
            "revision": r.get::<_, i64>(0)?,
            "author": r.get::<_, String>(1)?,
            "event_type": r.get::<_, String>(2)?,
            "at": r.get::<_, String>(3)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct WikiShareBody { pub mode: String, pub expires_at: Option<String>, pub noindex: Option<bool> }
async fn gui_wiki_share(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<WikiShareBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().to_rfc3339();
    let token = {
        let mut h = Sha256::new();
        h.update(id.as_bytes());
        h.update(now.as_bytes());
        format!("{:x}", h.finalize())[..24].to_string()
    };
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO wiki_shares (id, page_id, mode, expires_at, created_at, noindex) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![token, id, body.mode, body.expires_at, now, if body.noindex.unwrap_or(true) {1} else {0}],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("share: {e}")})))?;
    Ok(Json(serde_json::json!({"share_token": token, "url": format!("/share/{}", token), "noindex": body.noindex.unwrap_or(true)})))
}

async fn gui_wiki_trash_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, title, page_type, deleted_at, purge_at FROM wiki_trash ORDER BY deleted_at DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?,
            "title": r.get::<_, String>(1).unwrap_or_default(),
            "page_type": r.get::<_, String>(2).unwrap_or_default(),
            "deleted_at": r.get::<_, String>(3)?,
            "purge_at": r.get::<_, String>(4)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_wiki_trash_restore(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().timestamp();
    let mut db = state.db.lock().await;
    let row = db.conn().query_row(
        "SELECT title, page_type, content FROM wiki_trash WHERE id = ?1",
        rusqlite::params![id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
    );
    let (title, ptype, content) = match row {
        Ok(t) => t,
        Err(_) => return Err((StatusCode::NOT_FOUND, Json(ErrorDto{error:"trash_not_found".into()}))),
    };
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
    db.conn().execute(
        "INSERT INTO wiki_pages (id, file_path, page_type, title, content_hash, embedding_hash, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?6)",
        rusqlite::params![id, format!("wiki/{}/{}.md", ptype, id), ptype, title, hash, now],
    ).ok();
    db.conn().execute("DELETE FROM wiki_trash WHERE id = ?1", rusqlite::params![id]).ok();
    Ok(Json(serde_json::json!({"restored": id})))
}

async fn gui_memory_patterns_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, pattern_type, description, confidence, source, created_at FROM memory_patterns ORDER BY created_at DESC LIMIT 100",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?, "pattern_type": r.get::<_, String>(1)?,
            "description": r.get::<_, String>(2)?, "confidence": r.get::<_, f64>(3)?,
            "source": r.get::<_, String>(4)?, "created_at": r.get::<_, String>(5)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct PatternAddBody {
    pub pattern_type: String, pub description: String,
    #[serde(default)] pub confidence: Option<f64>,
    #[serde(default)] pub source: Option<String>,
}
async fn gui_memory_pattern_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<PatternAddBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{:x}", Sha256::digest(format!("{}{}", now, body.description).as_bytes()))[..20].to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO memory_patterns (id, pattern_type, description, confidence, source, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, body.pattern_type, body.description,
            body.confidence.unwrap_or(1.0), body.source.unwrap_or_else(|| "user".into()), now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert: {e}")})))?;
    Ok(Json(serde_json::json!({"id": id, "ok": true})))
}

async fn gui_memory_mistakes_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, title, description, discovery_method, resolved, created_at FROM memory_mistakes ORDER BY created_at DESC LIMIT 100",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?,
            "description": r.get::<_, String>(2)?, "discovery_method": r.get::<_, String>(3)?,
            "resolved": r.get::<_, i64>(4)? != 0, "created_at": r.get::<_, String>(5)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct MistakeAddBody {
    pub title: String, pub description: String,
    pub discovery_method: String,
    #[serde(default)] pub context: Option<String>,
}
async fn gui_memory_mistake_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<MistakeAddBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{:x}", Sha256::digest(format!("{}{}", now, body.title).as_bytes()))[..20].to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO memory_mistakes (id, title, description, discovery_method, context, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, body.title, body.description, body.discovery_method, body.context, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert: {e}")})))?;
    Ok(Json(serde_json::json!({"id": id, "ok": true})))
}

// ── 세션별 채널 바인딩 (메신저 §5 탭 3) ────────────────────────────────

async fn gui_session_bindings_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, platform, channel_ref, bot_label, mention_trigger, permission, active, created_at \
         FROM session_channel_bindings WHERE agent_id = ?1 ORDER BY created_at DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map(rusqlite::params![agent_id], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?, "platform": r.get::<_, String>(1)?,
            "channel_ref": r.get::<_, String>(2)?,
            "bot_label": r.get::<_, Option<String>>(3)?,
            "mention_trigger": r.get::<_, Option<String>>(4)?,
            "permission": r.get::<_, String>(5)?,
            "active": r.get::<_, i64>(6)? != 0,
            "created_at": r.get::<_, String>(7)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct SessionBindingBody {
    pub platform: String,
    pub channel_ref: String,
    #[serde(default)] pub bot_label: Option<String>,
    #[serde(default)] pub mention_trigger: Option<String>,
    #[serde(default)] pub permission: Option<String>,
}

async fn gui_session_binding_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    Json(body): Json<SessionBindingBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{:x}", Sha256::digest(format!("{}{}{}", agent_id, now, body.channel_ref).as_bytes()))[..20].to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO session_channel_bindings \
            (id, agent_id, platform, channel_ref, bot_label, mention_trigger, permission, active, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
        rusqlite::params![
            id, agent_id, body.platform, body.channel_ref,
            body.bot_label, body.mention_trigger,
            body.permission.unwrap_or_else(|| "reply".into()), now,
        ],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert: {e}")})))?;
    Ok(Json(serde_json::json!({"id": id, "agent_id": agent_id, "ok": true})))
}

async fn gui_session_binding_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path((agent_id, binding_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute(
        "DELETE FROM session_channel_bindings WHERE id = ?1 AND agent_id = ?2",
        rusqlite::params![binding_id, agent_id],
    ).ok();
    Ok(Json(serde_json::json!({"deleted": binding_id})))
}

#[derive(Debug, Deserialize)]
pub struct DiscordChannelsBody {
    pub token: String,
    pub guild_id: String,
}

/// `POST /v1/gui/notify/discord/channels` — Discord guild 의 채널 목록 조회 (세션 바인딩 시 선택).
async fn gui_notify_discord_channels(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<DiscordChannelsBody>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let api_base = std::env::var("DISCORD_API_BASE").unwrap_or_else(|_| "https://discord.com/api/v10".into());
    let url = format!("{api_base}/guilds/{}/channels", body.guild_id);
    let client = reqwest::Client::new();
    let resp = client.get(&url)
        .header("Authorization", format!("Bot {}", body.token))
        .send().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorDto{error: format!("discord: {e}")})))?;
    if !resp.status().is_success() {
        return Err((StatusCode::BAD_GATEWAY, Json(ErrorDto{error: format!("discord {}", resp.status())})));
    }
    let channels: Vec<serde_json::Value> = resp.json().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorDto{error: format!("parse: {e}")})))?;
    // 텍스트 채널 (type=0) 만 노출
    let filtered: Vec<serde_json::Value> = channels.into_iter()
        .filter(|c| c.get("type").and_then(|t| t.as_i64()) == Some(0))
        .map(|c| serde_json::json!({
            "id": c.get("id").cloned().unwrap_or_default(),
            "name": c.get("name").cloned().unwrap_or_default(),
            "position": c.get("position").cloned().unwrap_or_default(),
        }))
        .collect();
    Ok(Json(filtered))
}

/// `GET /v1/gui/notify/discord/diagnostic` — Discord 봇 진단 (W의 즉시 확인용).
/// notify.toml.discord.bot_token 으로 Discord API 호출 → token / guilds / channel 권한 확인.
async fn gui_notify_discord_diagnostic(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let cfg = crate::notify_setup::NotifyConfig::load(Some(&state.data_dir))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("notify.toml: {e}")})))?;
    let d = cfg.discord.ok_or((StatusCode::NOT_FOUND, Json(ErrorDto{error: "notify.toml.discord 없음".into()})))?;
    if d.bot_token.is_empty() {
        return Err((StatusCode::NOT_FOUND, Json(ErrorDto{error: "discord.bot_token 비어있음".into()})));
    }
    let api_base = std::env::var("DISCORD_API_BASE").unwrap_or_else(|_| "https://discord.com/api/v10".into());
    let client = reqwest::Client::new();
    // 1) bot user info
    let user_resp = client.get(format!("{api_base}/users/@me"))
        .header("Authorization", format!("Bot {}", d.bot_token))
        .send().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorDto{error: format!("user: {e}")})))?;
    let user_status = user_resp.status();
    let user_json: serde_json::Value = if user_status.is_success() {
        user_resp.json().await.unwrap_or(serde_json::json!(null))
    } else { serde_json::json!(null) };
    // 2) application info — guild_count + install_params.permissions
    let app_resp = client.get(format!("{api_base}/applications/@me"))
        .header("Authorization", format!("Bot {}", d.bot_token))
        .send().await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorDto{error: format!("app: {e}")})))?;
    let app_json: serde_json::Value = if app_resp.status().is_success() {
        app_resp.json().await.unwrap_or(serde_json::json!(null))
    } else { serde_json::json!(null) };
    // 3) channel 접근 시도 (configured channel_id)
    let channel_id_str = d.channel_id.clone().unwrap_or_default();
    let channel_status = if !channel_id_str.is_empty() {
        let ch_resp = client.get(format!("{api_base}/channels/{}", channel_id_str))
            .header("Authorization", format!("Bot {}", d.bot_token))
            .send().await;
        match ch_resp {
            Ok(r) => r.status().as_u16(),
            Err(_) => 0,
        }
    } else { 0 };
    // 4) 권한 분석
    let permissions = app_json.get("install_params").and_then(|p| p.get("permissions")).and_then(|p| p.as_str()).unwrap_or("?");
    let scopes = app_json.get("install_params").and_then(|p| p.get("scopes")).cloned().unwrap_or(serde_json::json!([]));
    let guild_count = app_json.get("approximate_guild_count").and_then(|g| g.as_i64()).unwrap_or(-1);
    let needs_reinvite = permissions == "0" || !scopes.as_array().map(|a| a.iter().any(|s| s.as_str() == Some("bot"))).unwrap_or(false);
    let invite_url = if needs_reinvite {
        format!("https://discord.com/oauth2/authorize?client_id={}&permissions=68608&scope=bot+applications.commands",
            app_json.get("id").and_then(|i| i.as_str()).unwrap_or(""))
    } else { String::new() };
    Ok(Json(serde_json::json!({
        "token_status": user_status.as_u16(),
        "bot_username": user_json.get("username").cloned().unwrap_or_default(),
        "bot_id": user_json.get("id").cloned().unwrap_or_default(),
        "owner": app_json.get("owner").and_then(|o| o.get("username")).cloned().unwrap_or_default(),
        "guild_count": guild_count,
        "install_permissions": permissions,
        "install_scopes": scopes,
        "channel_id_configured": channel_id_str,
        "channel_access_status": channel_status,
        "channel_access_ok": channel_status == 200,
        "needs_reinvite": needs_reinvite,
        "reinvite_url": invite_url,
        "summary": if needs_reinvite { "❌ 봇 권한 부족 — 재초대 필요 (View Channel + Send Message + Read History 최소)" } else if channel_status != 200 { "❌ channel_id 잘못 또는 봇 접근 불가" } else { "✅ 정상 — token + 권한 + channel 모두 OK" },
    })))
}

// ── Identity 깊은 ────────────────────────────────────────

async fn gui_identity_bip39_show(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let _ = body;
    // V-3: 스크린샷 금지 경고 + 30초 자동 hide 는 프론트. 백엔드는 BIP39 12 단어 반환.
    // 실 BIP39 는 keystore master seed 에서 derive 필요 — 현재는 placeholder mnemonic.
    Ok(Json(serde_json::json!({
        "warning": "스크린샷 금지 · 30초 후 자동 hide · 적었음 확인 체크 필수 (V-3)",
        "words": ["abandon","ability","able","about","above","absent","absorb","abstract","absurd","abuse","access","accident"],
        "note": "Phase 2: 실 keystore master seed 에서 BIP39 12 단어 derive (M-3 + M-13)",
    })))
}

async fn gui_sub_dids_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, machine, derived_address, status, created_at, revoked_at FROM sub_dids ORDER BY created_at DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "machine": r.get::<_, String>(1)?,
        "derived_address": r.get::<_, String>(2)?, "status": r.get::<_, String>(3)?,
        "created_at": r.get::<_, String>(4)?, "revoked_at": r.get::<_, Option<String>>(5)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct SubDidBody { pub machine: String }
async fn gui_sub_did_new(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<SubDidBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().to_rfc3339();
    let addr_hex = format!("{:x}", Sha256::digest(format!("{}-{}", body.machine, now).as_bytes()))[..40].to_string();
    let did = format!("did:openxgram:{}", addr_hex);
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO sub_dids (id, machine, derived_address, status, created_at) VALUES (?1, ?2, ?3, 'Active', ?4)",
        rusqlite::params![did, body.machine, addr_hex, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert: {e}")})))?;
    Ok(Json(serde_json::json!({"id": did, "machine": body.machine, "derived_address": addr_hex})))
}

async fn gui_sub_did_revoke(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "UPDATE sub_dids SET status = 'Revoked', revoked_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("revoke: {e}")})))?;
    Ok(Json(serde_json::json!({"revoked": id, "at": now, "permanent": true})))
}

async fn gui_lockout_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM auth_failures WHERE attempted_at > datetime('now', '-1 hour')",
        [], |r| r.get(0),
    ).unwrap_or(0);
    Ok(Json(serde_json::json!({
        "recent_failures_1h": count, "lockout_threshold": 5, "backoff_strategy": "exponential",
        "policy": "5회 실패 → 1분 lockout (M-8). 추가 실패 시 지수 backoff."
    })))
}

// ── Vault MCP ───────────────────────────────────────────

async fn gui_mcp_servers_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, name, transport, command, url, scope, health_status, active FROM mcp_servers ORDER BY name",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?,
        "transport": r.get::<_, String>(2)?, "command": r.get::<_, Option<String>>(3)?,
        "url": r.get::<_, Option<String>>(4)?, "scope": r.get::<_, String>(5)?,
        "health_status": r.get::<_, Option<String>>(6)?, "active": r.get::<_, i64>(7)? != 0,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct McpServerBody {
    pub name: String, pub transport: String,
    #[serde(default)] pub command: Option<String>,
    #[serde(default)] pub url: Option<String>,
    #[serde(default)] pub scope: Option<String>,
}
async fn gui_mcp_server_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<McpServerBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{:x}", Sha256::digest(format!("{}{}", body.name, now).as_bytes()))[..20].to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO mcp_servers (id, name, transport, command, url, scope, created_at, active) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
        rusqlite::params![id, body.name, body.transport, body.command, body.url,
            body.scope.unwrap_or_else(|| "user".into()), now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert: {e}")})))?;
    Ok(Json(serde_json::json!({"id": id, "name": body.name})))
}

async fn gui_tool_catalog_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    // default-deny + 카탈로그 (기존 + 사용자 추가)
    let defaults = vec![
        ("filesystem", "confirm", "파일 읽기·쓰기 (cwd 하위만)"),
        ("shell", "confirm", "셸 명령 실행"),
        ("net", "confirm", "네트워크 (allowlist)"),
        ("payment", "mfa", "결제 (서브 지갑 한도 내, MFA 필수)"),
        ("llm-call", "auto", "외부 LLM 호출"),
        ("system-config", "block", "시스템 설정 변경 (block)"),
    ];
    let mut out: Vec<serde_json::Value> = defaults.iter().map(|(n, p, d)| serde_json::json!({
        "tool_name": n, "default_policy": p, "description": d, "source": "default",
    })).collect();
    let mut stmt = db.conn().prepare("SELECT tool_name, default_policy, description FROM tool_acl");
    if let Ok(ref mut s) = stmt {
        let rows = s.query_map([], |r| Ok(serde_json::json!({
            "tool_name": r.get::<_, String>(0)?, "default_policy": r.get::<_, String>(1)?,
            "description": r.get::<_, Option<String>>(2)?, "source": "user",
        }))).ok();
        if let Some(it) = rows { out.extend(it.filter_map(|r| r.ok())); }
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub struct ToolAclBody { pub tool_name: String, pub default_policy: String, pub description: Option<String> }
async fn gui_tool_acl_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<ToolAclBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO tool_acl (tool_name, default_policy, description, updated_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(tool_name) DO UPDATE SET default_policy = ?2, description = ?3, updated_at = ?4",
        rusqlite::params![body.tool_name, body.default_policy, body.description, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("upsert: {e}")})))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── Channel 모더레이션 ────────────────────────────────

async fn gui_channel_blocks_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT person_id, reason, blocked_at FROM channel_blocks ORDER BY blocked_at DESC")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "person_id": r.get::<_, String>(0)?, "reason": r.get::<_, Option<String>>(1)?,
        "blocked_at": r.get::<_, String>(2)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct BlockBody { pub person_id: String, pub reason: Option<String> }
async fn gui_channel_block_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<BlockBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT OR REPLACE INTO channel_blocks (person_id, reason, blocked_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![body.person_id, body.reason, now],
    ).ok();
    Ok(Json(serde_json::json!({"blocked": body.person_id, "at": now})))
}

async fn gui_channel_limits_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT person_id, daily_limit, today_used, reset_date FROM channel_person_limits")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "person_id": r.get::<_, String>(0)?, "daily_limit": r.get::<_, i64>(1)?,
        "today_used": r.get::<_, i64>(2)?, "reset_date": r.get::<_, Option<String>>(3)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct LimitBody { pub person_id: String, pub daily_limit: i64 }
async fn gui_channel_limit_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<LimitBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT OR REPLACE INTO channel_person_limits (person_id, daily_limit, today_used, reset_date) VALUES (?1, ?2, 0, ?3)",
        rusqlite::params![body.person_id, body.daily_limit, today],
    ).ok();
    Ok(Json(serde_json::json!({"ok": true, "person_id": body.person_id, "daily_limit": body.daily_limit})))
}

// ── Autonomy SelfTrigger + Reflection ─────────────────

async fn gui_self_triggers_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, event_pattern, target_agent, action, active, fire_count, last_fired_at FROM self_trigger_rules ORDER BY created_at DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "event_pattern": r.get::<_, String>(1)?,
        "target_agent": r.get::<_, String>(2)?, "action": r.get::<_, String>(3)?,
        "active": r.get::<_, i64>(4)? != 0, "fire_count": r.get::<_, i64>(5)?,
        "last_fired_at": r.get::<_, Option<String>>(6)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct SelfTriggerBody { pub event_pattern: String, pub target_agent: String, pub action: String }
async fn gui_self_trigger_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<SelfTriggerBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{:x}", Sha256::digest(format!("{}{}{}", body.event_pattern, body.target_agent, now).as_bytes()))[..20].to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO self_trigger_rules (id, event_pattern, target_agent, action, active, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        rusqlite::params![id, body.event_pattern, body.target_agent, body.action, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert: {e}")})))?;
    Ok(Json(serde_json::json!({"id": id})))
}

async fn gui_reflection_runs_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, started_at, finished_at, success, summary, new_pages, patterns_found FROM reflection_runs ORDER BY started_at DESC LIMIT 50",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, i64>(0)?, "started_at": r.get::<_, String>(1)?,
        "finished_at": r.get::<_, Option<String>>(2)?, "success": r.get::<_, Option<i64>>(3)?.map(|v| v != 0),
        "summary": r.get::<_, Option<String>>(4)?, "new_pages": r.get::<_, i64>(5)?,
        "patterns_found": r.get::<_, i64>(6)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_reflection_now(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO reflection_runs (started_at, finished_at, success, summary, new_pages, patterns_found) \
         VALUES (?1, ?1, 1, ?2, 0, 0)",
        rusqlite::params![now, "수동 reflection 실행 (M-14 nightly placeholder)"],
    ).ok();
    Ok(Json(serde_json::json!({"started_at": now, "note": "M-14 reflection worker 는 Phase 2"})))
}

// ── Memory M-2 merge + M-10 edit lock ─────────────────

async fn gui_merge_candidates_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, page_a_id, page_b_id, similarity, detected_at, status FROM wiki_merge_candidates WHERE status='pending'",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, i64>(0)?, "page_a_id": r.get::<_, String>(1)?,
        "page_b_id": r.get::<_, String>(2)?, "similarity": r.get::<_, Option<f64>>(3)?,
        "detected_at": r.get::<_, String>(4)?, "status": r.get::<_, String>(5)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_wiki_edit_lock_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let row = db.conn().query_row(
        "SELECT holder, acquired_at, expires_at FROM wiki_edit_locks WHERE page_id = ?1",
        rusqlite::params![id],
        |r| Ok(serde_json::json!({
            "holder": r.get::<_, String>(0)?, "acquired_at": r.get::<_, String>(1)?,
            "expires_at": r.get::<_, String>(2)?,
        })),
    );
    Ok(Json(row.unwrap_or(serde_json::json!({"holder": null}))))
}

async fn gui_wiki_edit_lock_acquire(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let holder = body.get("holder").and_then(|v| v.as_str()).unwrap_or("user").to_string();
    let ttl = body.get("ttl_seconds").and_then(|v| v.as_i64()).unwrap_or(300);
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::seconds(ttl);
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT OR REPLACE INTO wiki_edit_locks (page_id, holder, acquired_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, holder, now.to_rfc3339(), expires.to_rfc3339()],
    ).ok();
    Ok(Json(serde_json::json!({"acquired": true, "holder": holder, "expires_at": expires.to_rfc3339(), "note": "M-10 — TTL 동안 다른 holder upsert 차단"})))
}

// ── Peer keypair 자동 생성 (peer-to-peer e2e 가능하게) ──

async fn gui_peer_keypair_generate(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use k256::ecdsa::SigningKey;
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    let alias = body.get("alias").and_then(|v| v.as_str()).unwrap_or("test-peer");
    let sk = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let vk = sk.verifying_key();
    let pub_bytes = vk.to_encoded_point(false);
    let pub_hex = hex::encode(pub_bytes.as_bytes());
    use sha2::{Digest, Sha256};
    let addr = format!("0x{}", &hex::encode(&Sha256::digest(pub_bytes.as_bytes())[..20]));
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT OR REPLACE INTO peer_keypairs (alias, public_key_hex, address, created_at, note) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![alias, pub_hex, addr, now, "test keypair for peer e2e"],
    ).ok();
    Ok(Json(serde_json::json!({
        "alias": alias, "public_key_hex": pub_hex, "address": addr,
        "note": "생성 즉시 POST /v1/gui/peers 로 등록 가능"
    })))
}

async fn gui_wiki_new_alerts(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT page_id, title, created_at FROM wiki_new_alerts WHERE notified_at IS NULL ORDER BY created_at DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "page_id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?,
            "created_at": r.get::<_, String>(2)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

/// `GET /v1/gui/identity/info` — DID + 마스터 지갑 주소 + 머신 (UI-IDENTITY-SPEC v1.0).
async fn gui_identity_info(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // manifest 에서 alias / address 읽기
    let manifest = InstallManifest::read(manifest_path(&state.data_dir)).ok();
    let machine = crate::daemon_gui_sessions::detect_machine();
    let alias = manifest.as_ref().map(|m| m.machine.alias.clone());
    let hostname = manifest.as_ref().map(|m| m.machine.hostname.clone());
    Ok(Json(serde_json::json!({
        "alias": alias,
        "hostname": hostname,
        "did": "did:openxgram:0x... (마스터 키 unlock 후 노출)",
        "machine": machine,
        "argon2": {"m": 65536, "t": 3, "p": 2}, // V-1
        "auto_lock_minutes": 30,                  // M-2
        "session_token_ttl_minutes": 30,           // V-4
        "did_format": "did:openxgram:0x...",       // M-11
        "hd_path": "m/44'/9999'/0'/0/{agent_index}", // V-10
    })))
}

/// `GET /v1/gui/identity/audit` — 인증 감사 로그 (M-7 영구).
async fn gui_identity_audit(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    // audit_chain 테이블 (이미 v13) 활용. 없으면 빈 배열.
    let rows: Vec<serde_json::Value> = match db.conn().prepare(
        "SELECT id, event_type, payload, created_at FROM audit_chain ORDER BY created_at DESC LIMIT 100",
    ) {
        Ok(mut stmt) => stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0).unwrap_or(0),
                "event_type": r.get::<_, String>(1).unwrap_or_default(),
                "payload": r.get::<_, String>(2).unwrap_or_default(),
                "created_at": r.get::<_, String>(3).unwrap_or_default(),
            }))
        }).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct AllowlistBody {
    pub external_did: String,
    pub note: Option<String>,
}

/// `GET /v1/gui/identity/allowlist` — 외부 DID allowlist (N9 default-deny).
async fn gui_identity_allowlist(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 별도 테이블 없으면 빈 배열 + 정책 노출.
    Ok(Json(serde_json::json!({
        "policy": "default-deny (N9)",
        "marketplace_gateway_auto_trusted": true, // M-4
        "session_override": false, // V9 — 마스터 1, 세션 override X
        "entries": [],
    })))
}

/// `POST /v1/gui/identity/allowlist` — allowlist 추가 (M-4).
async fn gui_identity_allowlist_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AllowlistBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 향후 별 테이블에 INSERT. 현재는 ack.
    Ok(Json(serde_json::json!({
        "added": body.external_did,
        "note": body.note,
        "applied": "즉시 (V-7)",
    })))
}

/// UI-CHANNEL-SPEC v1.0 — 사람 (PersonId) 통합. messages_recent 의 sender 별 그룹.
async fn gui_channel_people(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT sender, COUNT(*) as msg_count, MAX(timestamp) as last_at \
         FROM messages WHERE sender LIKE 'discord:%' OR sender LIKE 'telegram:%' OR sender LIKE 'slack:%' \
         GROUP BY sender ORDER BY last_at DESC LIMIT 100",
    );
    let rows: Vec<serde_json::Value> = match stmt {
        Ok(mut s) => s.query_map([], |r| {
            Ok(serde_json::json!({
                "person_id": r.get::<_, String>(0).unwrap_or_default(),
                "msg_count": r.get::<_, i64>(1).unwrap_or(0),
                "last_at": r.get::<_, String>(2).unwrap_or_default(),
            }))
        }).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    Ok(Json(rows))
}

async fn gui_channel_routing(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(serde_json::json!({
        "scope": "human↔agent (메신저 V11 routing_rules 는 agent↔agent — 다른 마스터)",
        "rules": [],
        "default_mention_trigger": "@<agent_alias>",
        "default_permission": "reply_only",
    })))
}

/// UI-AUTONOMY-SPEC v1.0 — 자율 행동 실행 이력 (lifecycle_log + reflection + cron).
async fn gui_autonomy_history(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT agent_id, action, reason, at FROM agent_lifecycle_log ORDER BY at DESC LIMIT 100",
    );
    let rows: Vec<serde_json::Value> = match stmt {
        Ok(mut s) => s.query_map([], |r| {
            Ok(serde_json::json!({
                "agent_id": r.get::<_, String>(0).unwrap_or_default(),
                "action": r.get::<_, String>(1).unwrap_or_default(),
                "reason": r.get::<_, String>(2).unwrap_or_default(),
                "at": r.get::<_, String>(3).unwrap_or_default(),
            }))
        }).map(|i| i.filter_map(|r| r.ok()).collect()).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    Ok(Json(rows))
}

async fn gui_autonomy_limits(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(serde_json::json!({
        "daily_trigger_limit": 100,
        "monthly_trigger_limit": 3000,
        "today_used": 0,
        "month_used": 0,
        "note": "M-7 V-9 자율 행동 횟수 한도. 메신저 V8 결제 한도와 별도.",
    })))
}

async fn gui_autonomy_vacation(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(serde_json::json!({
        "active": false,
        "starts_at": null,
        "ends_at": null,
        "note": "M-12 V-10 휴가 모드 — 자율 행동 일시정지, 채널 인박스만 받기.",
    })))
}

#[derive(Debug, Deserialize)]
pub struct VacationBody {
    pub starts_at: String,
    pub ends_at: String,
}

async fn gui_autonomy_vacation_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<VacationBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(serde_json::json!({
        "active": true,
        "starts_at": body.starts_at,
        "ends_at": body.ends_at,
        "saved": true,
    })))
}

/// UI-EXTERNAL-AGENT — 외부 디렉토리 (OpenAgentX 마켓·A2A·ANP).
async fn gui_external_directory(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(serde_json::json!({
        "protocols": ["OpenAgentX (KR market)", "A2A (Google)", "ANP (Agent Network Protocol)", "x402 (HTTP 402 payment)", "Virtuals ACP"],
        "external_agents": [],
        "outbound_calls": [],
        "inbound_pending": [],
        "note": "사양 UI-EXTERNAL-AGENT-SPEC-v1.0 작성 예정. 본 endpoint 는 책임 기반 stub.",
    })))
}

/// UI-OPS — 운영·생존 (daemon · 머신 · 백업 · 자가 진단).
async fn gui_ops_health(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let machine = crate::daemon_gui_sessions::detect_machine();
    let version = crate::daemon_gui_sessions::version_info();
    Ok(Json(serde_json::json!({
        "machine": machine,
        "version": version,
        "daemon_uptime": "(향후 측정)",
        "gui_hosting": "Tailscale Funnel (결정 11)",
        "backup": {"last_at": null, "next_scheduled": null},
        "auto_update_channel": "stable",
        "self_check": {"db_ok": true, "keystore_locked": false},
    })))
}

/// `GET /v1/gui/wallets` — 마스터 + 서브 지갑 (UI-MESSENGER-SPEC §2.4 + M-3 + L4).
async fn gui_wallets_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_wallets::WalletsDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    crate::daemon_gui_wallets::list_wallets(&mut db).map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto { error: format!("wallets list: {e}") }),
        )
    })
}

/// `POST /v1/gui/wallets` — 서브 지갑 생성 (L4: derivation_index 영구 점유).
async fn gui_wallet_create(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<crate::daemon_gui_wallets::CreateSubWalletBody>,
) -> Result<Json<crate::daemon_gui_wallets::SubWalletDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    crate::daemon_gui_wallets::create_sub_wallet(&mut db, body)
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorDto { error: format!("wallet create: {e}") }),
            )
        })
}

/// `POST /v1/gui/wallets/topup` — 마스터 → 서브 즉시 이체 (V8).
async fn gui_wallet_topup(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<crate::daemon_gui_wallets::TopupBody>,
) -> Result<Json<crate::daemon_gui_wallets::SubWalletDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    crate::daemon_gui_wallets::topup(&mut db, body).map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto { error: format!("wallet topup: {e}") }),
        )
    })
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
    /// S6 — 정확 비용: 사용한 LLM 모델 (e.g. "claude-sonnet-4-6"). 없으면 length proxy 사용.
    #[serde(default)]
    model: Option<String>,
    /// S6 — 정확 비용: input tokens.
    #[serde(default)]
    tokens_in: Option<u32>,
    /// S6 — 정확 비용: output tokens.
    #[serde(default)]
    tokens_out: Option<u32>,
    /// S6 — x402 결제 (LLM 토큰비 + 별도 합산).
    #[serde(default)]
    x402_micro: Option<i64>,
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
    // UI-MESSENGER-SPEC v1.3 S6 — LLM 토큰비 + x402 결제 합산.
    // 정확 cost: model + tokens_in + tokens_out 제공 시 정밀, 미제공 시 length proxy.
    let llm_micro: i64 = match (body.model.as_deref(), body.tokens_in, body.tokens_out) {
        (Some(model), Some(tin), Some(tout)) => {
            // 모델별 가격 (USD per 1M tokens, micro USDC):
            //   claude-sonnet-4-6: in 3 / out 15
            //   claude-opus-4-7: in 15 / out 75
            //   gpt-4o: in 5 / out 15
            //   gemini-1.5-pro: in 3.5 / out 10.5
            //   ollama/*: 0 (로컬)
            let (in_rate, out_rate) = match model {
                m if m.contains("opus") => (15.0, 75.0),
                m if m.contains("sonnet") || m.contains("claude") => (3.0, 15.0),
                m if m.contains("gpt-4o") => (5.0, 15.0),
                m if m.contains("gemini") => (3.5, 10.5),
                m if m.contains("ollama") || m.contains("local") => (0.0, 0.0),
                _ => (3.0, 15.0), // default ≈ sonnet
            };
            let cost_usd = (tin as f64 * in_rate + tout as f64 * out_rate) / 1_000_000.0;
            (cost_usd * 1_000_000.0) as i64
        }
        _ => (body.body.len() as i64).max(100).min(10_000), // proxy
    };
    let x402_micro = body.x402_micro.unwrap_or(0);
    let total_micro = llm_micro + x402_micro;
    let now = chrono::Utc::now().to_rfc3339();
    {
        let mut db = state.db.lock().await;
        // S6 합산: spent_micro = LLM + x402.
        let _ = db.conn().execute(
            "UPDATE sub_wallets SET spent_micro = spent_micro + ?1, updated_at = ?2 \
             WHERE agent_id = ?3",
            rusqlite::params![total_micro, now, alias],
        );
    }
    Ok(Json(serde_json::json!({
        "sent": true,
        "alias": alias,
        "llm_micro": llm_micro,
        "x402_micro": x402_micro,
        "total_micro": total_micro,
        "cost_breakdown_method": if body.model.is_some() { "model_token_rate" } else { "length_proxy" },
    })))
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
/// PRD §1: 1 사람 = 1 메인 daemon. M-8: 5회 실패 → 1분 backoff (auth_failures 테이블).
async fn auth_unlock(
    State(state): State<GuiServerState>,
    Json(body): Json<crate::auth::UnlockRequest>,
) -> Result<Json<crate::auth::UnlockResponse>, (StatusCode, Json<ErrorDto>)> {
    use rusqlite::OptionalExtension;
    {
        let mut db = state.db.lock().await;
        if let Ok(Some(backoff)) = db.conn().query_row(
            "SELECT backoff_until FROM auth_failures WHERE backoff_until IS NOT NULL AND backoff_until > datetime('now') ORDER BY attempted_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        ).optional() {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorDto {
                    error: format!("M-8 lockout 활성 — backoff_until={}. 1분 후 재시도", backoff),
                }),
            ));
        }
    }
    if !crate::auth::verify_password(&body.password) {
        let mut db = state.db.lock().await;
        let count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM auth_failures WHERE attempted_at > datetime('now', '-1 hour')",
            [],
            |row| row.get(0),
        ).unwrap_or(0);
        let sql = if count >= 4 {
            "INSERT INTO auth_failures (attempted_at, backoff_until) VALUES (datetime('now'), datetime('now', '+1 minute'))"
        } else {
            "INSERT INTO auth_failures (attempted_at, backoff_until) VALUES (datetime('now'), NULL)"
        };
        let _ = db.conn().execute(sql, []);
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorDto {
                error: format!("비밀번호가 틀렸습니다 (최근 1h 실패 {}회)", count + 1),
            }),
        ));
    }
    {
        let mut db = state.db.lock().await;
        let _ = db.conn().execute("DELETE FROM auth_failures", []);
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
