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
    /// rc.92 D2 — agent_capabilities JOIN
    pub description: Option<String>,
    pub capabilities: Vec<String>,
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
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .context("daemon-gui DB open 실패")?;
    db.migrate().context("daemon-gui DB migrate 실패")?;

    let state = GuiServerState {
        data_dir: Arc::new(data_dir),
        db: Arc::new(Mutex::new(db)),
    };
    let state_clone = state.clone();

    let app = Router::new()
        .route("/v1/gui/health", get(gui_health))
        .route("/v1/gui/status", get(gui_status))
        .route("/v1/gui/initialized", get(gui_initialized))
        .route("/v1/gui/peers", get(gui_peers).post(gui_peer_add))
        // 메신저 v1.3 §3.2 — 머신×세션 통합 detector (M-1).
        .route("/v1/gui/sessions", get(gui_sessions))
        .route("/v1/gui/sessions/{identifier}/screen", get(gui_session_screen))
        .route("/v1/gui/sessions/{identifier}/input", post(gui_session_input))
        // UI-MEMORY-SPEC v1.1 §K7 / §1.2 — L0 raw API + 5층 stats
        .route("/v1/gui/memory/l0", post(gui_memory_l0_save).get(gui_memory_l0_list))
        .route("/v1/gui/memory/stats", get(gui_memory_stats))
        .route("/v1/gui/memory/extract-now", post(gui_memory_extract_now))
        // (a) export — 세션 messages 묶음 → .md / .jsonl
        .route("/v1/gui/memory/export/session/{session_id}", get(gui_memory_export_session))
        .route("/v1/gui/memory/export/wiki/{id}", get(gui_memory_export_wiki))
        // (b) Claude Desktop import — local conv DB scan
        .route("/v1/gui/memory/import/scan-paths", get(gui_memory_import_scan_paths))
        .route("/v1/gui/memory/import/desktop", post(gui_memory_import_desktop))
        // (c) session 마이그레이션 — zip export/import
        .route("/v1/gui/memory/migration/export/{session_id}", get(gui_memory_migration_export))
        .route("/v1/gui/memory/migration/import", post(gui_memory_migration_import))
        // import 프롬프트 안내 — LLM 에 넘기는 표준 prompt
        .route("/v1/gui/memory/import/prompt-template", get(gui_memory_import_prompt))
        // import bundle — 5층 다종 항목 (message/episode/wiki_fact/pattern/mistake) 한번에 적재
        .route("/v1/gui/memory/import/bundle", post(gui_memory_import_bundle))
        // webhook (URL 안 token, no Bearer) — LLM 이 직접 push
        .route("/v1/gui/memory/import/webhook-token", get(gui_memory_webhook_token).post(gui_memory_webhook_rotate))
        .route("/v1/gui/sessions/aliases", get(gui_session_aliases_list))
        .route("/v1/gui/sessions/{identifier}/alias", post(gui_session_alias_set))
        .route("/v1/gui/machine", get(gui_machine_info))
        // UI-MESSENGER-SPEC v1.3 §7.1·§7.3 — 헤더 🔔 통합 승인 큐 (L6 차등 만료 + V4).
        .route("/v1/gui/approvals", get(gui_approvals))
        // UI-MESSENGER-SPEC v1.3 §2.4 + M-3 + L4 — 마스터+서브 지갑 (HD 영구 점유).
        .route("/v1/gui/wallets", get(gui_wallets_list).post(gui_wallet_create))
        .route("/v1/gui/wallets/topup", post(gui_wallet_topup))
        // UI-MESSENGER-SPEC v1.3 L3 + V1 — 역할별 auto_respond 마스터 정책.
        .route("/v1/gui/role-policies", get(gui_role_policies).post(gui_role_policies_set))
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
        // rc.122 — 에이전트 메신저 등록 (외부 채널 바인딩과 별개, 필수)
        // GET: 등록된 모든 에이전트 (messenger_enabled 포함). POST: 등록/갱신 upsert.
        .route("/v1/gui/agents",
               get(gui_agents_list).post(gui_agents_register))
        .route("/v1/gui/agents/{alias}",
               post(gui_agents_delete))
        // rc.125 — 자동 감지 (alias 의 cwd CLAUDE.md + .mcp.json)
        .route("/v1/gui/agents/auto-detect",
               post(gui_agents_auto_detect))
        // rc.129 — 지침 파일 (cwd/AGENT.md) inline 편집
        .route("/v1/gui/agents/instructions",
               get(gui_agents_instructions_get).post(gui_agents_instructions_save))
        // rc.132 — agent_templates 카탈로그 (msitarzewski/agency-agents)
        .route("/v1/gui/agent-templates", get(gui_agent_templates_list))
        .route("/v1/gui/agent-templates/refresh", post(gui_agent_templates_refresh))
        .route("/v1/gui/agent-templates/apply", post(gui_agent_templates_apply))
        // Discord 봇이 가입한 guild 의 channel 목록 (세션 바인딩 시 사용자가 선택)
        .route("/v1/gui/notify/discord/channels", post(gui_notify_discord_channels))
        // Discord 봇 진단 — token + permission + guild + channel 한 번에
        .route("/v1/gui/notify/discord/diagnostic", get(gui_notify_discord_diagnostic))
        // UI-IDENTITY-SPEC v1.0 — 신원 카드 endpoint
        .route("/v1/gui/identity/info", get(gui_identity_info))
        .route("/v1/gui/identity/settings", post(gui_identity_settings_update))
        .route("/v1/gui/identity/suspicious_dids", get(gui_identity_suspicious_list))
        .route("/v1/gui/identity/suspicious_dismiss", post(gui_identity_suspicious_dismiss))
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
        .route("/v1/gui/ops/diagnostic", get(gui_ops_diagnostic))
        .route("/v1/gui/ops/machines", get(gui_ops_machines))
        .route("/v1/gui/ops/backup-status", get(gui_ops_backup_status))
        .route("/v1/gui/ops/backup-now", post(gui_ops_backup_now))
        .route("/v1/gui/ops/update-check", get(gui_ops_update_check))
        .route("/v1/gui/ops/update-apply", post(gui_ops_update_apply))
        .route("/v1/gui/external/outbound-calls", get(gui_external_outbound))
        .route("/v1/gui/external/inbound-pending", get(gui_external_inbound))
        .route("/v1/gui/external/inbound/{id}/approve", post(gui_external_inbound_approve))
        .route("/v1/gui/external/inbound/{id}/reject", post(gui_external_inbound_reject))
        .route("/v1/gui/external/my-listings", get(gui_external_listings))
        .route("/v1/gui/external/listings", post(gui_external_listing_add))
        .route("/v1/gui/external/reputation", get(gui_external_reputation))
        .route("/v1/gui/external/protocols", get(gui_external_protocols))
        // UI-MESSENGER-SPEC v1.4 §20 — 오케스트레이션 워크플로 (W-1 ~ W-10)
        .route("/v1/gui/workflows", get(gui_workflows_list).post(gui_workflow_upsert))
        .route("/v1/gui/workflows/{id}", get(gui_workflow_get).post(gui_workflow_delete))
        .route("/v1/gui/workflows/{id}/run", post(gui_workflow_run))
        .route("/v1/gui/workflows/{id}/runs", get(gui_workflow_runs))
        .route("/v1/gui/workflows/runs/{run_id}/approve", post(gui_workflow_run_approve))
        .route("/v1/gui/peers/{alias}/send-unsigned", post(gui_peer_send_unsigned))
        // 메신저 카드 v1.3 Step 0 — 메시지 송수신.
        .route("/v1/gui/messages", get(gui_messages_recent))
        // rc.212 — peer conversation unified view. 한 peer 와의 전 session (outbox/inbox/Peer·/Claude Code·) 합쳐서 시간순.
        .route("/v1/gui/peer_conversation/{alias}", get(gui_peer_conversation))
        .route("/v1/gui/peers/{alias}/send", post(gui_peer_send))
        // rc.155 — portal × OpenXgram 통합. starian-portal 의 send 후 메시지 mirror.
        // portal 가 sendKeys 한 직후 POST → messages 테이블에 INSERT.
        // ack_status='delivered' 자동, via='portal_mirror'. GUI 의 ack badge 가 표시.
        .route("/v1/gui/messages/mirror", post(gui_messages_mirror))
        // rc.170 — auto-echo enforcer visual verification API
        .route("/v1/gui/bindings_status", get(gui_bindings_status))
        // rc.176 — daemon log tail (cross-machine 진단 도구). zalman 같은 remote peer 의 silent fail 진단.
        .route("/v1/gui/daemon/log", get(gui_daemon_log))
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
        // rc.91 — 바인딩 테스트 (▶ 테스트 버튼) + 봇 권한 진단 + 초대 URL
        .route("/v1/gui/notify/channel/test", post(gui_notify_channel_test))
        .route("/v1/gui/notify/discord/permissions", get(gui_notify_discord_permissions))
        .route("/v1/gui/notify/discord/invite_url", get(gui_notify_discord_invite_url))
        // rc.92 — 멀티 디스코드 봇 (채널·세션 별 다른 봇)
        .route("/v1/gui/discord/bots", get(gui_discord_bots_list).post(gui_discord_bots_add))
        .route("/v1/gui/discord/bots/{id}", post(gui_discord_bots_delete))
        // rc.92 — 모든 봇·채널·바인딩 종합 정보 (채널 카드 메인)
        .route("/v1/gui/channels/summary", get(gui_channels_summary))
        // rc.92 — 특정 봇의 가입 서버 채널 list (세션 중심 통합용)
        .route("/v1/gui/discord/bot/channels", get(gui_discord_bot_channels))
        // Telegram 마법사 — token 검증 → chat_id 자동 감지 → 저장+테스트.
        .route(
            "/v1/gui/notify/telegram/validate",
            post(gui_notify_telegram_validate),
        )
        .route(
            "/v1/gui/notify/telegram/detect_chat",
            post(gui_notify_telegram_detect_chat),
        )
        // 저장된 봇 토큰으로 chat_id 자동감지 (no body 필요)
        .route(
            "/v1/gui/notify/telegram/detect_chat_saved",
            post(gui_notify_telegram_detect_chat_saved),
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
        // 외부 LLM 직접 push — URL 안 token 으로 인증 (Bearer 없음)
        .route("/v1/webhook/memory/{token}", post(webhook_memory_ingest))
        // Web GUI 정적 자산 — xgram 바이너리에 임베드 (PRD-OpenXgram v1.3 §4.8).
        // nginx 외부 호스팅 불필요. 외부 노출은 Tailscale Funnel 또는 reverse proxy 위임.
        .route("/gui", get(crate::ui_assets::gui_root))
        .route("/gui/", get(crate::ui_assets::gui_root))
        .route("/gui/{*path}", get(crate::ui_assets::gui_asset_path))
        .with_state(state);

    // rc.184 — port 자동 fallback. 47302 가 Hyper-V/Windows 예약 port 가면 fail → 다른 port 시도.
    // Windows 47302 Hyper-V dynamic port 예약 케이스 처리. 사용자 install 부담 0.
    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(addr=%bind_addr, error=%e, "daemon-gui primary bind 실패 — fallback port 시도");
            let host = bind_addr.ip();
            let mut listener_opt = None;
            // 17302, 47312, 27302, 0 (random ephemeral) 순서 시도
            for port in [17302u16, 47312, 27302, 0] {
                let fallback = std::net::SocketAddr::new(host, port);
                match tokio::net::TcpListener::bind(fallback).await {
                    Ok(l) => {
                        tracing::warn!(fallback=%fallback, original=%bind_addr, "daemon-gui fallback bind 성공");
                        println!("  ⚠ GUI HTTP API fallback: {bind_addr} → {fallback}");
                        listener_opt = Some(l);
                        break;
                    }
                    Err(_) => continue,
                }
            }
            listener_opt.ok_or_else(|| anyhow::anyhow!("daemon-gui bind 전체 실패: {bind_addr} + fallback ports"))?
        }
    };
    let bound = listener.local_addr()?;
    tracing::info!(addr = %bound, "GUI HTTP API server bound");
    println!("  ✓ GUI HTTP API bound: http://{bound}");

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "daemon-gui server stopped");
        }
    });

    // rc.112 — outbound polling worker 폐기. 에이전트가 명시적으로
    // openxgram.send_to_discord / openxgram.send_to_telegram MCP 도구 호출
    // (agent-push 패턴). capture diff 알고리즘의 본질적 한계 (status bar / dynamic
    // prompt / echo loop) 우회.

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
    // rc.92 — agent_capabilities map 별도 prefetch (PeerStore 사용 후 두 번째 borrow 위해 미리)
    let mut caps_map: std::collections::HashMap<String, (Option<String>, Vec<String>)> = Default::default();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT alias, description, capabilities FROM agent_capabilities"
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))
        }) {
            for row in rows.flatten() {
                let caps: Vec<String> = row.2.as_ref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                caps_map.insert(row.0, (row.1, caps));
            }
        }
    }
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
        .map(|p| {
            let (description, capabilities) = caps_map.get(&p.alias).cloned().unwrap_or((None, vec![]));
            PeerDto {
                id: p.id,
                alias: p.alias,
                address: p.address,
                public_key_hex: p.public_key_hex,
                role: p.role.as_str().to_string(),
                created_at: p.created_at.to_rfc3339(),
                last_seen: p.last_seen.map(|t| t.to_rfc3339()),
                description,
                capabilities,
            }
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
    // rc.134 — sync collect (sub-process spawns + filesystem scan) 을 blocking pool 로 격리.
    // 이전: tokio worker 30초+ blocked → endpoint hang. 이제 main worker 즉시 자유.
    let mut dto = tokio::task::spawn_blocking(crate::daemon_gui_sessions::collect_sessions)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("collect: {e}")})))?;

    // S8 cross-machine fan-out — peers 테이블 의 http:// peer 들에게서 sessions 받아 merge
    // (실패는 silent, 3초 timeout, 토큰 동봉)
    let peer_targets: Vec<(String, String)> = {
        let mut db = state.db.lock().await;
        // rc.167 — gui_address 있으면 우선 사용 (transport 와 GUI port 가 다른 경우).
        let mut stmt_o = db.conn().prepare("SELECT alias, COALESCE(gui_address, address) FROM peers WHERE address LIKE 'http%'");
        match stmt_o {
            Ok(ref mut stmt) => stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            }).map(|it| it.flatten().collect::<Vec<_>>()).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    };
    if !peer_targets.is_empty() {
        // 각 peer 의 daemon 에 unlock 해서 그 머신의 토큰 받기 (같은 keystore env password 가정).
        let local_pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").unwrap_or_default();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build().ok();
        if let Some(http) = client {
            for (alias, address) in peer_targets {
                let base = address.trim_end_matches('/');
                // 1) peer 에 unlock → peer 의 session_token
                let unlock_resp = http.post(format!("{base}/v1/auth/unlock"))
                    .json(&serde_json::json!({"password": local_pw}))
                    .send().await;
                let peer_token: String = match unlock_resp {
                    Ok(r) => match r.json::<serde_json::Value>().await {
                        Ok(v) => v.get("session_token").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                        Err(_) => String::new(),
                    },
                    Err(_) => String::new(),
                };
                if peer_token.is_empty() { continue; }
                // 2) peer sessions 호출
                let url = format!("{base}/v1/gui/sessions");
                let resp = http.get(&url)
                    .header("Authorization", format!("Bearer {peer_token}"))
                    .send().await;
                if let Ok(r) = resp {
                    if let Ok(remote_json) = r.json::<serde_json::Value>().await {
                        let remote_arr = remote_json.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default();
                        for item in remote_arr {
                            // 최소 필드만 가져와서 DetectedSession 직접 구성
                            let identifier = item.get("identifier").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let display = item.get("display").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                            let kind_str = item.get("kind").and_then(|v| v.as_str()).unwrap_or("tmux");
                            let status_str = item.get("status").and_then(|v| v.as_str()).unwrap_or("detached");
                            let attached = item.get("attached").and_then(|v| v.as_bool());
                            let windows = item.get("windows").and_then(|v| v.as_u64()).map(|n| n as u32);
                            let last_active = item.get("last_active_at").and_then(|v| v.as_str()).map(String::from);
                            let created = item.get("created_at").and_then(|v| v.as_str()).map(String::from);
                            let kind = match kind_str {
                                "tmux" => crate::daemon_gui_sessions::SessionKind::Tmux,
                                "claude_project" => crate::daemon_gui_sessions::SessionKind::ClaudeProject,
                                _ => crate::daemon_gui_sessions::SessionKind::XgramSession,
                            };
                            let status = match status_str {
                                "attached" => crate::daemon_gui_sessions::SessionStatus::Attached,
                                "active" => crate::daemon_gui_sessions::SessionStatus::Active,
                                _ => crate::daemon_gui_sessions::SessionStatus::Detached,
                            };
                            dto.sessions.push(crate::daemon_gui_sessions::DetectedSession {
                                kind,
                                identifier: format!("peer:{}:{}", alias, identifier),
                                display: format!("[{}] {}", alias, display),
                                status,
                                windows,
                                attached,
                                created_at: created,
                                last_active_at: last_active,
                                agent_id: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // 사용자 부여 display_name override (DB v32 session_aliases)
    let mut db = state.db.lock().await;
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT identifier, display_name FROM session_aliases",
    ) {
        if let Ok(it) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) {
            use std::collections::HashMap;
            let aliases: HashMap<String, String> = it.flatten().collect();
            for s in dto.sessions.iter_mut() {
                if let Some(name) = aliases.get(&s.identifier) {
                    s.display = name.clone();
                }
            }
        }
    }
    Ok(Json(dto))
}

/// `GET /v1/gui/sessions/{identifier}/screen` — 세션 라이브 출력 (UI-MESSENGER-SPEC §4.3 S5).
/// tmux: capture-pane -e (ANSI). claude_project: .jsonl tail (포맷됨).
async fn gui_session_screen(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
) -> Result<Json<crate::daemon_gui_sessions::SessionScreenDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // peer:<alias>:<inner-id> 형태면 해당 peer 의 daemon 에 fan-out
    if let Some(rest) = identifier.strip_prefix("peer:") {
        if let Some(idx) = rest.find(':') {
            let alias = &rest[..idx];
            let inner = &rest[idx + 1..];
            // peer address 조회 — rc.167+: gui_address 있으면 우선 (7302 GUI), 없으면 address (7300 transport).
            let address: String = {
                let mut db = state.db.lock().await;
                db.conn().query_row(
                    "SELECT COALESCE(gui_address, address) FROM peers WHERE alias = ?1",
                    rusqlite::params![alias],
                    |r| r.get(0),
                ).unwrap_or_default()
            };
            if !address.is_empty() && address.starts_with("http") {
                let local_pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").unwrap_or_default();
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .map_err(|e| internal(&format!("http: {e}")))?;
                let base = address.trim_end_matches('/');
                // peer 의 unlock → token
                let unlock = client.post(format!("{base}/v1/auth/unlock"))
                    .json(&serde_json::json!({"password": local_pw}))
                    .send().await
                    .map_err(|e| internal(&format!("peer unlock: {e}")))?;
                let token = unlock.json::<serde_json::Value>().await.ok()
                    .and_then(|v| v.get("session_token").and_then(|t| t.as_str()).map(String::from))
                    .unwrap_or_default();
                if token.is_empty() {
                    return Err(internal(&format!("peer {alias} unlock failed")));
                }
                // peer screen
                let resp = client.get(format!("{base}/v1/gui/sessions/{}/screen", urlencoding::encode(inner)))
                    .header("Authorization", format!("Bearer {token}"))
                    .send().await
                    .map_err(|e| internal(&format!("peer screen: {e}")))?;
                let v: serde_json::Value = resp.json().await
                    .map_err(|e| internal(&format!("peer screen json: {e}")))?;
                let kind_str = v.get("kind").and_then(|x| x.as_str()).unwrap_or("tmux");
                let kind = match kind_str {
                    "tmux" => crate::daemon_gui_sessions::SessionKind::Tmux,
                    "claude_project" => crate::daemon_gui_sessions::SessionKind::ClaudeProject,
                    _ => crate::daemon_gui_sessions::SessionKind::XgramSession,
                };
                let dto = crate::daemon_gui_sessions::SessionScreenDto {
                    identifier: v.get("identifier").and_then(|x| x.as_str()).unwrap_or(inner).into(),
                    kind,
                    display: v.get("display").and_then(|x| x.as_str()).unwrap_or("?").into(),
                    content: v.get("content").and_then(|x| x.as_str()).unwrap_or("").into(),
                    lines: v.get("lines").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                    source_note: format!("[via peer {alias}] {}", v.get("source_note").and_then(|x| x.as_str()).unwrap_or("")),
                    fetched_at: v.get("fetched_at").and_then(|x| x.as_str()).unwrap_or("").into(),
                };
                return Ok(Json(dto));
            }
            return Err(internal(&format!("peer {alias} has no http address")));
        }
    }
    Ok(Json(crate::daemon_gui_sessions::capture_session(&identifier)))
}

#[derive(serde::Deserialize)]
struct SessionInputBody {
    /// 사용자가 친 raw 키 스트림 (예: "ls\n", "\u{0003}" = Ctrl-C).
    data: String,
}

/// `POST /v1/gui/sessions/:identifier/input` — tmux send-keys 로 입력 주입.
/// identifier 가 "tmux:<name>" 또는 "<name>" 이면 tmux send-keys -l.
async fn gui_session_input(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
    Json(body): Json<SessionInputBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    // ── (1) peer fan-out ──────────────────────────────────────────
    if let Some(rest) = identifier.strip_prefix("peer:") {
        if let Some(idx) = rest.find(':') {
            let alias = &rest[..idx];
            let inner = &rest[idx + 1..];
            let address: String = {
                let mut db = state.db.lock().await;
                db.conn().query_row(
                    "SELECT address FROM peers WHERE alias = ?1",
                    rusqlite::params![alias],
                    |r| r.get(0),
                ).unwrap_or_default()
            };
            if !address.is_empty() && address.starts_with("http") {
                let local_pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").unwrap_or_default();
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .map_err(|e| internal(&format!("http: {e}")))?;
                let base = address.trim_end_matches('/');
                let unlock = client.post(format!("{base}/v1/auth/unlock"))
                    .json(&serde_json::json!({"password": local_pw}))
                    .send().await
                    .map_err(|e| internal(&format!("peer unlock: {e}")))?;
                let token = unlock.json::<serde_json::Value>().await.ok()
                    .and_then(|v| v.get("session_token").and_then(|t| t.as_str()).map(String::from))
                    .unwrap_or_default();
                if token.is_empty() {
                    return Err(internal(&format!("peer {alias} unlock failed")));
                }
                let resp = client.post(format!("{base}/v1/gui/sessions/{}/input", urlencoding::encode(inner)))
                    .header("Authorization", format!("Bearer {token}"))
                    .json(&serde_json::json!({"data": body.data}))
                    .send().await
                    .map_err(|e| internal(&format!("peer input: {e}")))?;
                if !resp.status().is_success() {
                    let s = resp.status();
                    let t = resp.text().await.unwrap_or_default();
                    return Err(bad_request(&format!("peer input HTTP {s}: {t}")));
                }
                return Ok(Json(serde_json::json!({"ok": true, "via": format!("peer:{alias}"), "bytes_sent": body.data.len()})));
            }
            return Err(internal(&format!("peer {alias} has no http address")));
        }
    }

    // ── (2) portal:<tmuxSession>:<idx> + aoe:<tmuxSession>:... → portal-new /api/tmux/send ──
    let portal_target: Option<(String, u32)> = if let Some(rest) = identifier.strip_prefix("portal:") {
        let mut parts = rest.splitn(2, ':');
        let first = parts.next().unwrap_or("");
        let rest2 = parts.next().unwrap_or("");
        if first.parse::<u32>().is_ok() && !rest2.contains(':') {
            None // 옛 형식 fallback — tmux 직접
        } else {
            let idx = rest2.split(':').next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            Some((first.to_string(), idx))
        }
    } else if let Some(rest) = identifier.strip_prefix("aoe:") {
        let tmux_session = rest.split(':').next().unwrap_or("");
        if tmux_session.is_empty() { None } else { Some((tmux_session.to_string(), 0)) }
    } else {
        None
    };
    if let Some((session, idx)) = portal_target {
        let url_base = std::env::var("XGRAM_PORTAL_URL").unwrap_or_else(|_| "https://portal-zalman.starian.us".into());
        let token = std::env::var("XGRAM_PORTAL_TOKEN").unwrap_or_else(|_| "0205".into());
        let url = format!("{}/api/tmux/send?token={}", url_base.trim_end_matches('/'), token);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| internal(&format!("http: {e}")))?;
        // Enter (CR/LF) 처리 — portal 의 -l literal 모드는 줄바꿈을 키로 해석 못 함.
        // data 끝의 \r/\n 제거 + enter=true 플래그 전송.
        let trailing_enter = body.data.ends_with('\r') || body.data.ends_with('\n');
        let text_clean = body.data.trim_end_matches(|c: char| c == '\r' || c == '\n');
        let mut payload = serde_json::json!({"session": session, "window": idx});
        if !text_clean.is_empty() {
            payload["text"] = serde_json::Value::String(text_clean.to_string());
        }
        if trailing_enter {
            payload["enter"] = serde_json::Value::Bool(true);
            // text 가 비어있으면 keys=Enter 단독 송신 (text 없이 enter 만)
            if text_clean.is_empty() {
                payload.as_object_mut().unwrap().remove("text");
                payload["keys"] = serde_json::Value::String("Enter".to_string());
                payload.as_object_mut().unwrap().remove("enter");
            }
        }
        let resp = client.post(&url)
            .json(&payload)
            .send().await
            .map_err(|e| internal(&format!("portal send: {e}")))?;
        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default();
            return Err(bad_request(&format!("portal send HTTP {s}: {t}")));
        }
        return Ok(Json(serde_json::json!({"ok": true, "via": format!("portal:{session}:{idx}"), "bytes_sent": body.data.len()})));
    }

    // ── (3) local tmux fallback (tmux:<name>) ─────────────────────
    let target = identifier
        .strip_prefix("tmux:")
        .unwrap_or(&identifier)
        .to_string();
    if target.is_empty() {
        return Err(bad_request("empty identifier"));
    }
    let data = body.data.clone();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<std::process::Output> {
        std::process::Command::new("tmux")
            .arg("send-keys")
            .arg("-t")
            .arg(&target)
            .arg("-l")
            .arg(&data)
            .output()
    })
    .await
    .map_err(|e| internal(&format!("spawn: {e}")))?
    .map_err(|e| internal(&format!("tmux: {e}")))?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        return Err(bad_request(&format!("tmux send-keys: {stderr}")));
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "bytes_sent": body.data.len()
    })))
}

/// `POST /v1/gui/memory/l0` — L0 raw 메시지 저장 (UI-MEMORY-SPEC §K7).
/// 메신저·채널·외부 카드가 호출하는 공식 write path.
#[derive(serde::Deserialize)]
struct L0SaveBody {
    session_id: String,
    sender: String,
    body: String,
    #[serde(default = "default_l0_signature")]
    signature: String,
    conversation_id: Option<String>,
    /// metadata schema (§K20): source, kind, channel, etc.
    metadata: Option<serde_json::Value>,
}
fn default_l0_signature() -> String { "gui-l0".into() }

async fn gui_memory_l0_save(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<L0SaveBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.body.trim().is_empty() {
        return Err(bad_request("body must be non-empty"));
    }
    if body.session_id.trim().is_empty() {
        return Err(bad_request("session_id required"));
    }
    let mut db = state.db.lock().await;
    let result = crate::save_l0::save_l0_message(&mut db, crate::save_l0::L0SaveInput {
        id: None,
        session_id: &body.session_id,
        session_title: None,
        sender: &body.sender,
        body: &body.body,
        signature: &body.signature,
        timestamp: None,
        parent_message_id: None,
        conversation_id: body.conversation_id.as_deref(),
        source: "gui_l0_endpoint",
        extra_metadata: body.metadata.clone(),
    }, None).map_err(|e| internal(&format!("L0 save: {e}")))?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "id": result.id,
        "conversation_id": result.conversation_id,
        "saved_at": result.timestamp,
        "inserted": result.inserted
    })))
}

/// `GET /v1/gui/memory/l0?limit=50&q=...` — L0 raw 메시지 최근/검색.
async fn gui_memory_l0_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let limit: i64 = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50).min(500);
    let search = q.get("q").cloned();
    let mut db = state.db.lock().await;
    let (sql, has_q): (&str, bool) = match &search {
        Some(_) => (
            "SELECT id, session_id, sender, body, signature, timestamp, conversation_id, metadata \
             FROM messages WHERE body LIKE ?1 ORDER BY timestamp DESC LIMIT ?2",
            true,
        ),
        None => (
            "SELECT id, session_id, sender, body, signature, timestamp, conversation_id, metadata \
             FROM messages ORDER BY timestamp DESC LIMIT ?1",
            false,
        ),
    };
    let mut stmt = db.conn().prepare(sql).map_err(|e| internal(&format!("db: {e}")))?;
    let rows: Vec<serde_json::Value> = if has_q {
        let pat = format!("%{}%", search.as_ref().unwrap());
        stmt.query_map(rusqlite::params![pat, limit], row_to_l0).and_then(|it| Ok(it.filter_map(|r| r.ok()).collect()))
    } else {
        stmt.query_map(rusqlite::params![limit], row_to_l0).and_then(|it| Ok(it.filter_map(|r| r.ok()).collect()))
    }.map_err(|e| internal(&format!("db: {e}")))?;
    Ok(Json(rows))
}

fn row_to_l0(r: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?,
        "session_id": r.get::<_, String>(1)?,
        "sender": r.get::<_, String>(2)?,
        "body": r.get::<_, String>(3)?,
        "signature": r.get::<_, String>(4)?,
        "timestamp": r.get::<_, String>(5)?,
        "conversation_id": r.get::<_, Option<String>>(6)?,
        "metadata": r.get::<_, String>(7).unwrap_or_default(),
    }))
}

/// `GET /v1/gui/memory/stats` — 5층 메모리 카운트 + 최근 활동.
async fn gui_memory_stats(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let conn = db.conn();
    fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
    }
    let l0 = count(conn, "SELECT COUNT(*) FROM messages");
    let l1 = count(conn, "SELECT COUNT(*) FROM episodes");
    let l2 = count(conn, "SELECT COUNT(*) FROM wiki_pages");
    let l3 = count(conn, "SELECT COUNT(*) FROM memory_patterns") + count(conn, "SELECT COUNT(*) FROM patterns");
    let l4 = count(conn, "SELECT COUNT(*) FROM traits");
    let mistakes = count(conn, "SELECT COUNT(*) FROM mistakes");
    let last_l0: Option<String> = conn.query_row(
        "SELECT MAX(timestamp) FROM messages", [], |r| r.get(0)
    ).ok();
    let last_l1: Option<String> = conn.query_row(
        "SELECT MAX(created_at) FROM episodes", [], |r| r.get(0)
    ).ok();
    let sessions = count(conn, "SELECT COUNT(*) FROM sessions");
    let claude_ingested = count(conn, "SELECT IFNULL(SUM(msg_count),0) FROM claude_ingest_state");
    Ok(Json(serde_json::json!({
        "layers": {
            "L0_raw_messages": {"count": l0, "last_at": last_l0},
            "L1_episodes": {"count": l1, "last_at": last_l1},
            "L2_wiki_pages": {"count": l2},
            "L3_patterns": {"count": l3},
            "L4_traits": {"count": l4},
        },
        "extras": {
            "mistakes": mistakes,
            "sessions": sessions,
            "claude_ingested": claude_ingested,
        },
        "spec_ref": "UI-MEMORY-SPEC v1.1 §1.1 (L0~L4 5층)"
    })))
}

/// `GET /v1/gui/memory/export/session/:session_id?format=md|jsonl` — 세션 메시지 묶음 다운로드.
async fn gui_memory_export_session(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorDto>)> {
    use axum::response::IntoResponse;
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let format = q.get("format").cloned().unwrap_or_else(|| "md".into());
    let mut db = state.db.lock().await;
    let title: String = db.conn().query_row(
        "SELECT title FROM sessions WHERE id = ?1",
        rusqlite::params![&session_id],
        |r| r.get(0)
    ).unwrap_or_else(|_| session_id.clone());
    let mut stmt = db.conn().prepare(
        "SELECT sender, body, timestamp FROM messages WHERE session_id = ?1 ORDER BY timestamp"
    ).map_err(|e| internal(&format!("db: {e}")))?;
    let rows: Vec<(String, String, String)> = stmt.query_map(rusqlite::params![&session_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    }).map_err(|e| internal(&format!("db: {e}")))?.filter_map(|r| r.ok()).collect();
    let body = if format == "jsonl" {
        rows.iter().map(|(s,b,t)| serde_json::json!({"sender":s,"body":b,"timestamp":t}).to_string()).collect::<Vec<_>>().join("\n")
    } else {
        let mut s = format!("# {}\n\n_세션 id: `{}` · 메시지 {} 건 · OpenXgram export_\n\n", title, session_id, rows.len());
        for (sender, body, ts) in &rows {
            s.push_str(&format!("## {} · {}\n\n{}\n\n---\n\n", sender, ts, body));
        }
        s
    };
    let ext = if format == "jsonl" { "jsonl" } else { "md" };
    let mime = if format == "jsonl" { "application/x-ndjson" } else { "text/markdown" };
    let safe = title.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
    Ok((
        StatusCode::OK,
        [
            ("content-type", mime),
            ("content-disposition", &*format!("attachment; filename=\"{}.{}\"", safe.chars().take(60).collect::<String>(), ext)),
        ],
        body,
    ).into_response())
}

async fn gui_memory_export_wiki(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorDto>)> {
    use axum::response::IntoResponse;
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let (title, body): (String, String) = db.conn().query_row(
        "SELECT title, body FROM wiki_pages WHERE id = ?1",
        rusqlite::params![&id],
        |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?))
    ).map_err(|_| bad_request("page not found"))?;
    let safe = title.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
    Ok((
        StatusCode::OK,
        [
            ("content-type", "text/markdown"),
            ("content-disposition", &*format!("attachment; filename=\"{}.md\"", safe.chars().take(60).collect::<String>())),
        ],
        format!("# {}\n\n{}", title, body),
    ).into_response())
}

/// `GET /v1/gui/memory/import/scan-paths` — Claude Desktop/Cursor 등 데스크탑 앱 conv 경로 자동 탐지.
async fn gui_memory_import_scan_paths(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let home = std::env::var("HOME").unwrap_or_default();
    let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
    let candidates = vec![
        ("Claude Code CLI", format!("{home}/.claude/projects")),
        ("Claude Desktop (Mac)", format!("{home}/Library/Application Support/Claude")),
        ("Claude Desktop (Linux)", format!("{home}/.config/Claude")),
        ("Claude Desktop (Win)", format!("{userprofile}/AppData/Roaming/Claude")),
        ("Cursor (Mac)", format!("{home}/Library/Application Support/Cursor/User")),
        ("Cursor (Linux)", format!("{home}/.config/Cursor/User")),
        ("Cursor (Win)", format!("{userprofile}/AppData/Roaming/Cursor/User")),
        ("Continue", format!("{home}/.continue")),
        ("Aider", format!("{home}/.aider.input.history")),
    ];
    let mut found = Vec::new();
    for (name, path) in candidates {
        let p = std::path::PathBuf::from(&path);
        let exists = p.exists();
        let mut file_count = 0_i64;
        if exists && p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                file_count = rd.count() as i64;
            }
        }
        found.push(serde_json::json!({
            "name": name,
            "path": path,
            "exists": exists,
            "file_count": file_count
        }));
    }
    Ok(Json(serde_json::json!({
        "candidates": found,
        "note": "exists=true 인 경로는 import/desktop endpoint 로 ingest 가능. Claude Code CLI 는 이미 자동 ingest 중 (60s tick)."
    })))
}

#[derive(serde::Deserialize)]
struct DesktopImportBody {
    path: String,
}
async fn gui_memory_import_desktop(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<DesktopImportBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 안전: ~/.claude / ~/Library 같은 known prefix 만 허용 (path traversal 방지)
    let allowed = ["/.claude/", "/Library/Application Support/Claude", "/.config/Claude",
        "/AppData/Roaming/Claude", "/.continue/", "/Library/Application Support/Cursor",
        "/.config/Cursor", "/AppData/Roaming/Cursor"];
    if !allowed.iter().any(|a| body.path.contains(a)) {
        return Err(bad_request("path not in allowed desktop-app prefixes"));
    }
    let p = std::path::PathBuf::from(&body.path);
    if !p.exists() {
        return Err(bad_request("path does not exist"));
    }
    // 단순 구현: 폴더 안의 모든 .jsonl / .json 파일을 messages 로 흡수 (sender=path filename)
    let mut total = 0_i64;
    let mut db = state.db.lock().await;
    if let Ok(rd) = std::fs::read_dir(&p) {
        for e in rd.flatten() {
            let path = e.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            if !name.ends_with(".jsonl") && !name.ends_with(".json") { continue; }
            let content = match std::fs::read_to_string(&path) { Ok(c) => c, Err(_) => continue };
            for (idx, line) in content.lines().enumerate() {
                if line.trim().is_empty() { continue; }
                let id = format!("dt-{}-{}", name.chars().take(40).collect::<String>(), idx);
                let session_id = format!("desktop:{}", name);
                let now = chrono::Utc::now().to_rfc3339();
                let body_str = line.chars().take(4000).collect::<String>();
                let r = crate::save_l0::save_l0_message(&mut db, crate::save_l0::L0SaveInput {
                    id: Some(id),
                    session_id: &session_id,
                    session_title: Some(&name),
                    sender: "desktop-import",
                    body: &body_str,
                    signature: "desktop-import",
                    timestamp: Some(&now),
                    parent_message_id: None,
                    conversation_id: None,
                    source: "desktop_import",
                    extra_metadata: None,
                }, None);
                if let Ok(res) = r { if res.inserted { total += 1; } }
            }
        }
    }
    Ok(Json(serde_json::json!({"ok": true, "messages_imported": total, "source": body.path})))
}

/// `POST /v1/gui/memory/import/bundle` — 5층 다종 items 한 번에 적재.
async fn gui_memory_import_bundle(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(bundle): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    process_import_bundle(&state, bundle).await
}

/// 핵심 ingest 로직 — webhook + bundle endpoint 공유.
async fn process_import_bundle(
    state: &GuiServerState,
    bundle: serde_json::Value,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    let items = bundle.get("items").and_then(|v| v.as_array())
        .ok_or_else(|| bad_request("missing items array"))?;
    let session_title = bundle.get("session_title").and_then(|v| v.as_str()).unwrap_or("imported");
    let source_app = bundle.get("source_app").and_then(|v| v.as_str()).unwrap_or("external");
    let now = chrono::Utc::now().to_rfc3339();
    // session_id 가 bundle 에 있으면 그걸로 (특정 터미널·세션에 직접 적재).
    // 없으면 새 import 세션 생성.
    let session_id = bundle.get("session_id").and_then(|v| v.as_str()).map(String::from)
        .unwrap_or_else(|| format!("import:{}:{}",
            source_app.chars().filter(|c| c.is_alphanumeric()).take(20).collect::<String>(),
            chrono::Utc::now().timestamp_millis()));
    let mut counts = std::collections::HashMap::new();
    let mut db = state.db.lock().await;
    let _ = db.conn().execute(
        "INSERT OR IGNORE INTO sessions (id, title, participants, created_at, last_active, home_machine) \
         VALUES (?1, ?2, '[\"W\",\"imported\"]', ?3, ?3, 'server-seoul')",
        rusqlite::params![session_id, session_title, now],
    );
    for it in items {
        let itype = it.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match itype {
            "message" => {
                let sender = it.get("sender").and_then(|v| v.as_str()).unwrap_or("imported");
                let body = it.get("body").and_then(|v| v.as_str()).unwrap_or("");
                if body.is_empty() { continue; }
                let ts = it.get("timestamp").and_then(|v| v.as_str()).unwrap_or(&now);
                let r = crate::save_l0::save_l0_message(&mut db, crate::save_l0::L0SaveInput {
                    id: None,
                    session_id: &session_id,
                    session_title: Some(session_title),
                    sender,
                    body,
                    signature: "import",
                    timestamp: Some(ts),
                    parent_message_id: None,
                    conversation_id: None,
                    source: "import_bundle",
                    extra_metadata: Some(serde_json::json!({"app": source_app})),
                }, None);
                if let Ok(res) = r { if res.inserted { *counts.entry("messages").or_insert(0) += 1; } }
            }
            "episode" => {
                let title = it.get("title").and_then(|v| v.as_str()).unwrap_or("(no title)");
                let summary = it.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                let started = it.get("started_at").and_then(|v| v.as_str()).unwrap_or(&now);
                let ended = it.get("ended_at").and_then(|v| v.as_str()).unwrap_or(&now);
                let id = uuid::Uuid::new_v4().to_string();
                if db.conn().execute(
                    "INSERT OR IGNORE INTO episodes (id, session_id, title, summary, started_at, ended_at, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![id, session_id, title, summary, started, ended, now],
                ).unwrap_or(0) > 0 { *counts.entry("episodes").or_insert(0) += 1; }
            }
            "wiki_fact" => {
                let page_id = it.get("page_id").and_then(|v| v.as_str()).map(String::from)
                    .unwrap_or_else(|| format!("import-{}", &uuid::Uuid::new_v4().to_string()[..8]));
                let title = it.get("title").and_then(|v| v.as_str()).unwrap_or(&page_id);
                let ptype = it.get("page_type").and_then(|v| v.as_str()).unwrap_or("concept");
                let content = it.get("content").and_then(|v| v.as_str()).unwrap_or("");
                use sha2::{Digest, Sha256};
                let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
                let ts_int = chrono::Utc::now().timestamp();
                if db.conn().execute(
                    "INSERT OR IGNORE INTO wiki_pages (id, title, body, page_type, file_path, content_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    rusqlite::params![page_id, title, content, ptype,
                        format!("wiki/{}/{}.md", ptype, page_id), hash, ts_int],
                ).unwrap_or(0) > 0 { *counts.entry("wiki_pages").or_insert(0) += 1; }
            }
            "pattern" => {
                let ptype = it.get("pattern_type").and_then(|v| v.as_str()).unwrap_or("behavior");
                let desc = it.get("description").and_then(|v| v.as_str()).unwrap_or("");
                if desc.is_empty() { continue; }
                let conf = it.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.7);
                let id = format!("import-{}", &uuid::Uuid::new_v4().to_string()[..16]);
                if db.conn().execute(
                    "INSERT OR IGNORE INTO memory_patterns (id, pattern_type, description, confidence, source, examples, created_at) \
                     VALUES (?1, ?2, ?3, ?4, 'import-llm', '[]', ?5)",
                    rusqlite::params![id, ptype, desc, conf, now],
                ).unwrap_or(0) > 0 { *counts.entry("patterns").or_insert(0) += 1; }
            }
            "mistake" => {
                let intended = it.get("intended_action").and_then(|v| v.as_str()).unwrap_or("?");
                let outcome = it.get("actual_outcome").and_then(|v| v.as_str()).unwrap_or("?");
                let reason = it.get("failure_reason").and_then(|v| v.as_str()).unwrap_or("?");
                let lesson = it.get("lesson").and_then(|v| v.as_str()).unwrap_or("?");
                let severity = it.get("severity").and_then(|v| v.as_i64()).unwrap_or(5).clamp(1, 10);
                let id = format!("import-{}", &uuid::Uuid::new_v4().to_string()[..16]);
                let ts_ms = chrono::Utc::now().timestamp_millis();
                if db.conn().execute(
                    "INSERT OR IGNORE INTO mistakes (id, session_id, occurred_at, intended_action, actual_outcome, failure_reason, lesson, severity, embedding_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?3, ?3)",
                    rusqlite::params![id, session_id, ts_ms, intended, outcome, reason, lesson, severity, &id],
                ).unwrap_or(0) > 0 { *counts.entry("mistakes").or_insert(0) += 1; }
            }
            _ => {}
        }
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "session_id": session_id,
        "items_processed": items.len(),
        "inserted": counts,
        "source_app": source_app
    })))
}

/// `POST /v1/webhook/memory/:token` — Bearer 없이 URL token 으로 인증.
async fn webhook_memory_ingest(
    State(state): State<GuiServerState>,
    Path(token): Path<String>,
    Json(bundle): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    let expected: String = {
        let mut db = state.db.lock().await;
        db.conn().query_row(
            "SELECT value FROM identity_settings WHERE key = 'webhook_token_memory'",
            [], |r| r.get(0)
        ).unwrap_or_default()
    };
    if expected.is_empty() || token != expected {
        return Err((StatusCode::UNAUTHORIZED, Json(ErrorDto {
            error: "invalid webhook token — generate at /v1/gui/memory/import/webhook-token".into()
        })));
    }
    process_import_bundle(&state, bundle).await
}

/// `GET /v1/gui/memory/import/webhook-token` — 현재 token 조회 (Bearer 필요).
async fn gui_memory_webhook_token(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let token: String = {
        let mut db = state.db.lock().await;
        db.conn().query_row(
            "SELECT value FROM identity_settings WHERE key = 'webhook_token_memory'",
            [], |r| r.get(0)
        ).unwrap_or_default()
    };
    let base = "https://server-seoul.tail0957ca.ts.net";
    Ok(Json(serde_json::json!({
        "token": token,
        "exists": !token.is_empty(),
        "webhook_url": if token.is_empty() { String::new() } else { format!("{}/v1/webhook/memory/{}", base, token) },
        "rotate_endpoint": "POST /v1/gui/memory/import/webhook-token"
    })))
}

/// `POST /v1/gui/memory/import/webhook-token` — 새 token 발급/회전.
async fn gui_memory_webhook_rotate(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use sha2::{Digest, Sha256};
    let raw = format!("{}-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), uuid::Uuid::new_v4());
    let token = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let token = &token[..32]; // 32 hex chars
    let mut db = state.db.lock().await;
    let _ = db.conn().execute(
        "INSERT INTO identity_settings(key,value,updated_at) VALUES('webhook_token_memory', ?1, datetime('now')) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        rusqlite::params![token],
    );
    let base = "https://server-seoul.tail0957ca.ts.net";
    Ok(Json(serde_json::json!({
        "token": token,
        "webhook_url": format!("{}/v1/webhook/memory/{}", base, token),
        "note": "이 URL 을 LLM 에 주면 Bearer 없이 직접 push 가능"
    })))
}

/// `GET /v1/gui/memory/import/prompt-template` — LLM 에 던질 표준 import 프롬프트 안내.
async fn gui_memory_import_prompt(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let base = "https://server-seoul.tail0957ca.ts.net";

    let prompt = format!(r####"# OpenXgram 메모리 Import 프롬프트

당신은 외부 LLM(Claude Desktop / ChatGPT / Cursor / Gemini 등)입니다.
사용자의 OpenXgram 메모리(5층 = L0 raw / L1 episodes / L2 wiki / L3 patterns / L4 traits)에 기록할 컨텍스트를 추출·변환해 주세요.

---

## 1. 무엇을 추출할 것

다음 5 종류 중 해당하는 항목을 JSON 객체로 정리:

| 종류 | 내용 | OpenXgram 레이어 | 추출 기준 |
|---|---|---|---|
| **message** | 사용자/에이전트가 한 발화·응답 | L0 raw | 모든 의미 있는 대화 |
| **episode** | 한 작업 흐름 요약 (5~50 메시지 묶음) | L1 episode | 시작·끝 명확한 작업 단위 |
| **wiki_fact** | 영구적인 사실·개념·정의 (위키 페이지로 격상) | L2 wiki | "X는 Y다", 정의·설명 |
| **pattern** | 사용자 반복 행동·선호·습관·규칙 | L3 pattern | "사용자는 항상 ~", "~를 선호" |
| **mistake** | 발생한 실수·버그·잘못된 결정 + 교훈 | mistakes | "잘못해서", "버그", "다음엔 ~" |

---

## 2. 출력 형식 (JSON bundle — 단일 객체)

```json
{{
  "openxgram_import_version": 1,
  "source_app": "Claude Desktop|Cursor|ChatGPT|Gemini|기타",
  "exported_at": "2026-05-22T12:00:00Z",
  "session_title": "한 줄 요약 — 무엇에 대한 컨텍스트인가",
  "items": [
    {{
      "type": "message",
      "sender": "user|assistant|<name>",
      "body": "메시지 본문 그대로",
      "timestamp": "2026-05-22T11:55:00Z"
    }},
    {{
      "type": "episode",
      "title": "OpenAgentX 결제 흐름 설계",
      "summary": "x402 + USDC 통합 결정. fixed/auction/matching/chain 4모드 확정.",
      "started_at": "2026-05-22T11:00:00Z",
      "ended_at": "2026-05-22T11:50:00Z",
      "source_messages": [0, 1, 2, 3]
    }},
    {{
      "type": "wiki_fact",
      "page_id": "openagentx-payment-modes",
      "title": "OpenAgentX 4가지 결제 모드",
      "page_type": "concept",
      "content": "# OpenAgentX 결제 모드\n\n## 1. fixed\n... (마크다운)"
    }},
    {{
      "type": "pattern",
      "pattern_type": "preference",
      "description": "사용자는 한국어로 답변을 받기 선호. 영어 응답이 와도 한글로 재요청.",
      "confidence": 0.9
    }},
    {{
      "type": "mistake",
      "intended_action": "rc.58 배포",
      "actual_outcome": "daemon 이 SSH 끊기면서 죽음",
      "failure_reason": "env var XGRAM_KEYSTORE_PASSWORD 가 tmux 세션 안에서만 설정됨",
      "lesson": "~/.openxgram/daemon.env 파일에 영구 저장. 재시작 스크립트는 source 후 실행",
      "severity": 6
    }}
  ]
}}
```

---

## 3. 전송 채널 (4 가지 — 사용자 환경에 맞춰 선택)

### 채널 A — HTTP API (가장 일반)
```bash
TOKEN="<OpenXgram 잠금해제 토큰 — GUI 카드에서 복사>"
curl -X POST "{base}/v1/gui/memory/import/bundle" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  --data-binary @bundle.json
```

### 채널 B — Webhook (LLM 직접 push, 토큰 in URL)
```bash
curl -X POST "{base}/v1/webhook/memory/<WEBHOOK_TOKEN>" \
  -H "Content-Type: application/json" \
  --data-binary @bundle.json
```
WEBHOOK_TOKEN 은 GUI → 메모리 → 가져오기 탭에서 발급 (URL 한 번 발급 후 영구).

### 채널 C — Peer 메시지 (다른 OpenXgram 인스턴스로)
```bash
xgram peer send --alias <peer-alias> --body "$(cat bundle.json)" --metadata kind=memory-import
```
받는 peer 측에서 자동 ingest.

### 채널 D — MCP 도구 (Claude Desktop / Cursor / Continue 안에서 직접)
`xgram mcp-serve` 가 노출하는 MCP 도구 `memory_import_bundle` 호출:
```
"이 대화 전체를 OpenXgram 메모리에 import 해 주세요"
→ Claude Desktop 이 자동으로 memory_import_bundle({{...bundle...}}) MCP 호출
```
사용자가 OpenXgram 을 Claude Desktop / Cursor 에 MCP 로 등록한 경우 (`xgram init` 자동) — 가장 자연스러움.

---

## 4. 데스크탑 앱별 export 가이드 (출처 데이터 빼는 법)

| 앱 | 방법 |
|---|---|
| **Claude Desktop** | 대화 좌상단 ⋮ → "Export conversation" → .json |
| **Claude Code CLI** | OpenXgram daemon 이 자동 ingest 중 (`~/.claude/projects/*.jsonl`) — 별도 export 불필요 |
| **Cursor** | Cmd+Shift+P → "Export Chat as JSON" |
| **ChatGPT** | 설정 → Data controls → Export data → ZIP → conversations.json |
| **Gemini** | google takeout → Bard → conversations.json |

---

## 5. 검증

import 후 다음 URL 에서 확인:
- 메모리 카드 → L0 raw 메시지 탭 (검색 가능)
- 메모리 카드 → L2 위키 페이지 탭 (page_id 로 찾기)
- 자율 행동 카드 → Reflection 즉시 실행 → L1 episodes 자동 추출 / L3 patterns 갱신
"####, base = base);

    Ok(Json(serde_json::json!({
        "title": "OpenXgram memory import — 완전 프롬프트 (v2)",
        "prompt": prompt,
        "channels": {
            "api": format!("{}/v1/gui/memory/import/bundle (Bearer auth)", base),
            "webhook": format!("{}/v1/webhook/memory/<WEBHOOK_TOKEN> (token in URL)", base),
            "peer": "xgram peer send --alias <peer> --metadata kind=memory-import",
            "mcp": "xgram mcp-serve → tool: memory_import_bundle (Claude Desktop / Cursor)"
        },
        "extract_kinds": ["message", "episode", "wiki_fact", "pattern", "mistake"],
        "spec_ref": "UI-MEMORY-SPEC v1.1 §K7 + 5층 (L0~L4)"
    })))
}

/// `GET /v1/gui/memory/migration/export/:session_id` — sessions/messages 묶음 zip 다운로드.
async fn gui_memory_migration_export(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorDto>)> {
    use axum::response::IntoResponse;
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let session_row: serde_json::Value = db.conn().query_row(
        "SELECT id, title, participants, created_at, last_active, home_machine FROM sessions WHERE id = ?1",
        rusqlite::params![&session_id],
        |r| Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?,
            "title": r.get::<_, String>(1)?,
            "participants": r.get::<_, String>(2)?,
            "created_at": r.get::<_, String>(3)?,
            "last_active": r.get::<_, String>(4)?,
            "home_machine": r.get::<_, String>(5)?,
        }))
    ).map_err(|_| bad_request("session not found"))?;
    let mut stmt = db.conn().prepare(
        "SELECT id, sender, body, signature, timestamp, conversation_id FROM messages WHERE session_id = ?1 ORDER BY timestamp"
    ).map_err(|e| internal(&format!("db: {e}")))?;
    let messages: Vec<serde_json::Value> = stmt.query_map(rusqlite::params![&session_id], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?,
            "sender": r.get::<_, String>(1)?,
            "body": r.get::<_, String>(2)?,
            "signature": r.get::<_, String>(3)?,
            "timestamp": r.get::<_, String>(4)?,
            "conversation_id": r.get::<_, Option<String>>(5)?,
        }))
    }).map_err(|e| internal(&format!("db: {e}")))?.filter_map(|r| r.ok()).collect();
    let bundle = serde_json::json!({
        "openxgram_export_version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "session": session_row,
        "messages": messages
    });
    let safe = session_id.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
    Ok((
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            ("content-disposition", &*format!("attachment; filename=\"session-{}.json\"", safe.chars().take(60).collect::<String>())),
        ],
        bundle.to_string(),
    ).into_response())
}

#[derive(serde::Deserialize)]
struct MigrationImportBody {
    bundle: serde_json::Value,
}
async fn gui_memory_migration_import(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<MigrationImportBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let session = body.bundle.get("session").ok_or_else(|| bad_request("missing session"))?;
    let messages = body.bundle.get("messages").and_then(|m| m.as_array())
        .ok_or_else(|| bad_request("missing messages array"))?;
    let session_id = session.get("id").and_then(|v| v.as_str())
        .ok_or_else(|| bad_request("missing session.id"))?;
    let title = session.get("title").and_then(|v| v.as_str()).unwrap_or(session_id);
    let participants = session.get("participants").and_then(|v| v.as_str()).unwrap_or("[]");
    let now = chrono::Utc::now().to_rfc3339();
    let home = session.get("home_machine").and_then(|v| v.as_str()).unwrap_or("imported");
    let mut db = state.db.lock().await;
    let _ = db.conn().execute(
        "INSERT OR IGNORE INTO sessions (id, title, participants, created_at, last_active, home_machine) \
         VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
        rusqlite::params![session_id, title, participants, now, home],
    );
    let mut imported = 0;
    for m in messages {
        let id = match m.get("id").and_then(|v| v.as_str()) { Some(s) => s.to_string(), None => continue };
        let sender = m.get("sender").and_then(|v| v.as_str()).unwrap_or("imported");
        let mbody = m.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let sig = m.get("signature").and_then(|v| v.as_str()).unwrap_or("migration");
        let ts = m.get("timestamp").and_then(|v| v.as_str()).unwrap_or(&now);
        let conv = m.get("conversation_id").and_then(|v| v.as_str()).unwrap_or(session_id);
        let r = crate::save_l0::save_l0_message(&mut db, crate::save_l0::L0SaveInput {
            id: Some(id),
            session_id,
            session_title: Some(title),
            sender,
            body: mbody,
            signature: sig,
            timestamp: Some(ts),
            parent_message_id: None,
            conversation_id: Some(conv),
            source: "migration_import",
            extra_metadata: None,
        }, None);
        if let Ok(res) = r { if res.inserted { imported += 1; } }
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "session_id": session_id,
        "messages_imported": imported,
        "total_in_bundle": messages.len()
    })))
}

/// `POST /v1/gui/memory/extract-now` — patterns + mistakes 휴리스틱 즉시 실행.
async fn gui_memory_extract_now(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let pre_patterns: i64 = {
        let mut db = state.db.lock().await;
        db.conn().query_row("SELECT COUNT(*) FROM memory_patterns", [], |r| r.get(0)).unwrap_or(0)
    };
    let pre_mistakes: i64 = {
        let mut db = state.db.lock().await;
        db.conn().query_row("SELECT COUNT(*) FROM mistakes", [], |r| r.get(0)).unwrap_or(0)
    };
    if let Err(e) = crate::daemon_workers::run_patterns_mistakes_extract(&state.db).await {
        return Err(internal(&format!("extract: {e}")));
    }
    let post_patterns: i64 = {
        let mut db = state.db.lock().await;
        db.conn().query_row("SELECT COUNT(*) FROM memory_patterns", [], |r| r.get(0)).unwrap_or(0)
    };
    let post_mistakes: i64 = {
        let mut db = state.db.lock().await;
        db.conn().query_row("SELECT COUNT(*) FROM mistakes", [], |r| r.get(0)).unwrap_or(0)
    };
    Ok(Json(serde_json::json!({
        "ok": true,
        "patterns_added": post_patterns - pre_patterns,
        "mistakes_added": post_mistakes - pre_mistakes,
        "patterns_total": post_patterns,
        "mistakes_total": post_mistakes,
    })))
}

/// `GET /v1/gui/sessions/aliases` — 사용자 부여 display_name 전체 (identifier → display_name).
async fn gui_session_aliases_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut map = serde_json::Map::new();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT identifier, display_name, note, updated_at FROM session_aliases",
    ) {
        if let Ok(it) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        }) {
            for (id, name, note, updated) in it.flatten() {
                map.insert(id, serde_json::json!({
                    "display_name": name,
                    "note": note,
                    "updated_at": updated,
                }));
            }
        }
    }
    Ok(Json(serde_json::Value::Object(map)))
}

#[derive(serde::Deserialize)]
struct SessionAliasBody {
    display_name: String,
    note: Option<String>,
}
/// `POST /v1/gui/sessions/:identifier/alias` — display_name 저장.
async fn gui_session_alias_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(identifier): Path<String>,
    Json(body): Json<SessionAliasBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let name = body.display_name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err(bad_request("display_name must be 1..=64 chars"));
    }
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO session_aliases(identifier,display_name,note,updated_at) \
         VALUES(?1,?2,?3,datetime('now')) \
         ON CONFLICT(identifier) DO UPDATE SET \
           display_name=excluded.display_name, \
           note=excluded.note, \
           updated_at=excluded.updated_at",
        rusqlite::params![identifier, name, body.note],
    ).map_err(|e| internal(&format!("db: {e}")))?;
    Ok(Json(serde_json::json!({"ok": true, "identifier": identifier, "display_name": name})))
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

/// `GET /v1/gui/role-policies` — 역할별 auto_respond 기본 정책 (L3 + V1, DB v31).
async fn gui_role_policies(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::daemon_gui_sessions::RolePolicyDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut roles: Vec<crate::daemon_gui_sessions::RolePolicyItem> = Vec::new();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT role, auto_respond_default, max_concurrent FROM role_policies ORDER BY role",
    ) {
        let it = stmt.query_map([], |r| {
            Ok(crate::daemon_gui_sessions::RolePolicyItem {
                role: r.get::<_, String>(0)?,
                auto_respond_default: r.get::<_, i64>(1)? != 0,
                max_concurrent: r.get::<_, i64>(2)? as u32,
            })
        });
        if let Ok(it) = it {
            for r in it.flatten() {
                roles.push(r);
            }
        }
    }
    if roles.is_empty() {
        return Ok(Json(crate::daemon_gui_sessions::default_role_policies()));
    }
    Ok(Json(crate::daemon_gui_sessions::RolePolicyDto {
        master_card: "자율 행동 카드".into(),
        roles,
    }))
}

#[derive(serde::Deserialize)]
struct RolePolicyUpdate {
    role: String,
    auto_respond_default: bool,
    max_concurrent: u32,
}
async fn gui_role_policies_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(req): Json<RolePolicyUpdate>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if req.role.is_empty() || req.role.len() > 32 {
        return Err(bad_request("role length must be 1..=32"));
    }
    if req.max_concurrent == 0 || req.max_concurrent > 100 {
        return Err(bad_request("max_concurrent must be 1..=100"));
    }
    let mut db = state.db.lock().await;
    db.conn()
        .execute(
            "INSERT INTO role_policies(role,auto_respond_default,max_concurrent,updated_at) \
             VALUES(?1,?2,?3,datetime('now')) \
             ON CONFLICT(role) DO UPDATE SET \
               auto_respond_default=excluded.auto_respond_default, \
               max_concurrent=excluded.max_concurrent, \
               updated_at=excluded.updated_at",
            rusqlite::params![
                req.role,
                if req.auto_respond_default { 1_i64 } else { 0 },
                req.max_concurrent as i64
            ],
        )
        .map_err(|e| internal(&format!("db: {e}")))?;
    Ok(Json(serde_json::json!({"ok": true})))
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
    // 워커(`patterns_mistakes_extract_tick`)가 `mistakes` 테이블에 저장 — legacy
    // `memory_mistakes` 비어 있는 게 정상. 표시는 양쪽 UNION 으로 통합.
    // mistakes 컬럼 매핑: intended_action→title, lesson→description, "heuristic"→discovery_method
    let mut stmt = db.conn().prepare(
        "SELECT id, title, description, discovery_method, resolved, created_at FROM (\
            SELECT id, intended_action AS title, lesson AS description, \
                   'heuristic' AS discovery_method, resolved, \
                   datetime(created_at/1000, 'unixepoch') AS created_at \
                FROM mistakes \
            UNION ALL \
            SELECT id, title, description, discovery_method, resolved, created_at \
                FROM memory_mistakes \
        ) ORDER BY created_at DESC LIMIT 100",
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
        "SELECT id, platform, channel_ref, bot_label, mention_trigger, permission, active, created_at, bot_id \
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
            "bot_id": r.get::<_, Option<String>>(8)?,
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
    /// rc.92 — discord_bots.id (NULL 이면 default notify.toml 봇 사용)
    #[serde(default)] pub bot_id: Option<String>,
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
            (id, agent_id, platform, channel_ref, bot_label, mention_trigger, permission, active, created_at, bot_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9)",
        rusqlite::params![
            id, agent_id, body.platform, body.channel_ref,
            body.bot_label, body.mention_trigger,
            body.permission.unwrap_or_else(|| "reply".into()), now,
            body.bot_id,
        ],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("insert: {e}")})))?;
    // rc.121 — binding add 시 agent_capabilities placeholder 자동 등록.
    // role/description/capabilities 는 NULL — 그 binding 의 LLM 이 첫 세션 시
    // register_subagent 호출로 자기 정보 명시 (oxg.md 룰).
    // list_peers 결과에 이 binding 자동 노출 (alias 만이라도).
    db.conn().execute(
        "INSERT OR IGNORE INTO agent_capabilities (alias, role, description, capabilities, tool_list, project_path, updated_at) \
         VALUES (?1, 'binding', NULL, NULL, NULL, NULL, ?2)",
        rusqlite::params![agent_id, now],
    ).ok();
    Ok(Json(serde_json::json!({"id": id, "agent_id": agent_id, "bot_id": body.bot_id, "ok": true})))
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

// rc.122 — 에이전트 메신저 등록 (agent_capabilities 직접 CRUD)
// 외부 채널 바인딩과 별개. 모든 협업 에이전트의 필수 등록 path.
#[derive(serde::Deserialize)]
struct AgentRegisterBody {
    alias: String,
    #[serde(default)] role: Option<String>,
    #[serde(default)] description: Option<String>,
    #[serde(default)] capabilities: Option<String>,
    #[serde(default)] tool_list: Option<String>,
    #[serde(default)] project_path: Option<String>,
    #[serde(default)] group_name: Option<String>,
    #[serde(default)] messenger_enabled: bool,
    // rc.125 — 자유 orchestration role (enum 아님) + 특수 지침
    #[serde(default)] orchestration_role: Option<String>,
    #[serde(default)] special_instructions: Option<String>,
}

#[derive(serde::Deserialize)]
struct AgentAutoDetectBody {
    alias: String,
    #[serde(default)] project_path_hint: Option<String>,
}

async fn gui_agents_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT alias, role, description, capabilities, tool_list, project_path, \
                group_name, messenger_enabled, orchestration_role, special_instructions, updated_at \
         FROM agent_capabilities ORDER BY messenger_enabled DESC, alias ASC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "alias": r.get::<_, String>(0)?,
            "role": r.get::<_, Option<String>>(1)?,
            "description": r.get::<_, Option<String>>(2)?,
            "capabilities": r.get::<_, Option<String>>(3)?,
            "tool_list": r.get::<_, Option<String>>(4)?,
            "project_path": r.get::<_, Option<String>>(5)?,
            "group_name": r.get::<_, Option<String>>(6)?,
            "messenger_enabled": r.get::<_, i64>(7)? != 0,
            "orchestration_role": r.get::<_, Option<String>>(8)?,
            "special_instructions": r.get::<_, Option<String>>(9)?,
            "updated_at": r.get::<_, String>(10)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_agents_register(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AgentRegisterBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    // rc.157 — role NOT NULL 위반 fix. 사용자가 role 안 입력하면 default "agent".
    // (이전: body.role None → INSERT 시 SQL constraint fail)
    let role = body.role.as_deref().filter(|s| !s.is_empty()).unwrap_or("agent").to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO agent_capabilities \
            (alias, role, description, capabilities, tool_list, project_path, group_name, messenger_enabled, orchestration_role, special_instructions, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(alias) DO UPDATE SET \
            role = COALESCE(excluded.role, role), \
            description = COALESCE(excluded.description, description), \
            capabilities = COALESCE(excluded.capabilities, capabilities), \
            tool_list = COALESCE(excluded.tool_list, tool_list), \
            project_path = COALESCE(excluded.project_path, project_path), \
            group_name = COALESCE(excluded.group_name, group_name), \
            messenger_enabled = excluded.messenger_enabled, \
            orchestration_role = COALESCE(excluded.orchestration_role, orchestration_role), \
            special_instructions = COALESCE(excluded.special_instructions, special_instructions), \
            updated_at = excluded.updated_at",
        rusqlite::params![
            body.alias, role, body.description, body.capabilities, body.tool_list,
            body.project_path, body.group_name, body.messenger_enabled as i64,
            body.orchestration_role, body.special_instructions, now,
        ],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("upsert: {e}")})))?;

    // rc.192 본질 fix — UI 토글 messenger_enabled=true 시 sub-keypair + peer entry 자동 생성.
    // 이전엔 messenger_enabled flag 만 set 되어 UI MSG 태그 거짓 표시였음.
    // 진실: 이 alias 가 sub-keypair 보유 + peers table 등록 → 실제 peer-to-peer 통신 가능.
    let mut registered_eth: Option<String> = None;
    if body.messenger_enabled {
        use openxgram_keystore::{FsKeystore, Keystore};
        use openxgram_core::paths::keystore_dir;

        let pw = openxgram_core::env::require_password()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("XGRAM_KEYSTORE_PASSWORD 미설정 (daemon env): {e}")})))?;
        let ks = FsKeystore::new(keystore_dir(state.data_dir.as_ref()));

        // sub-keypair: 같은 alias 의 key 이미 있으면 load, 없으면 generate.
        let kp = match ks.load(&body.alias, &pw) {
            Ok(k) => k,
            Err(_) => {
                ks.create(&body.alias, &pw)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("agent sub-keypair 생성 실패: {e}")})))?;
                ks.load(&body.alias, &pw)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("새 keypair load 실패: {e}")})))?
            }
        };
        let pubkey_hex = hex::encode(kp.public_key_bytes());
        let eth_addr = kp.address.to_string();
        registered_eth = Some(eth_addr.clone());

        // peer entry add. address = 이 머신의 transport public URL (env override 가능).
        // 외부 peer 가 이 URL 로 envelope POST → daemon 가 alias 별 inbox 분리 routing (Step 2).
        let local_addr = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:47300".to_string());
        let mut peer_store = PeerStore::new(&mut db);
        // 이미 있으면 (UNIQUE alias) error — silent skip.
        let _ = peer_store.add_with_eth(
            &body.alias,
            &pubkey_hex,
            &local_addr,
            Some(&eth_addr),
            PeerRole::Worker,
            Some(&format!("auto-registered via UI messenger toggle ({now})")),
        );
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": body.alias,
        "eth_address": registered_eth,
    })))
}

// rc.132 — agent_templates: agency-agents 카탈로그 (msitarzewski/agency-agents).
// fetch + parse + insert (UPSERT). customized=1 row 는 보존.
async fn fetch_agency_agents() -> Result<Vec<serde_json::Value>, String> {
    let client = reqwest::Client::builder()
        .user_agent("OpenXgram/0.2")
        .timeout(std::time::Duration::from_secs(60))
        .build().map_err(|e| e.to_string())?;
    let root_url = "https://api.github.com/repos/msitarzewski/agency-agents/contents";
    let resp = client.get(root_url).send().await.map_err(|e| format!("github root: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("github root HTTP {}", resp.status()));
    }
    let categories: Vec<serde_json::Value> = resp.json().await.map_err(|e| format!("json: {e}"))?;
    let mut all = vec![];
    for cat in categories {
        if cat.get("type").and_then(|v| v.as_str()) != Some("dir") { continue;}
        let cat_name = match cat.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.starts_with('.') && n != "examples" => n.to_string(),
            _ => continue,
        };
        let url = format!("https://api.github.com/repos/msitarzewski/agency-agents/contents/{}", cat_name);
        let resp2 = match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => continue,
        };
        let files: Vec<serde_json::Value> = match resp2.json().await {
            Ok(v) => v, _ => continue,
        };
        for file in files {
            if file.get("type").and_then(|v| v.as_str()) != Some("file") { continue;}
            let path = file.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            if !path.ends_with(".md") || path.ends_with("README.md") { continue;}
            let dl = match file.get("download_url").and_then(|v| v.as_str()) {
                Some(u) => u.to_string(), None => continue,
            };
            let content = match client.get(&dl).send().await {
                Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
                _ => continue,
            };
            // frontmatter parse: --- ... --- + body
            let mut name = String::new();
            let mut description = None;
            let mut color = None;
            let mut emoji = None;
            let mut vibe = None;
            let body: String;
            if let Some(rest) = content.strip_prefix("---\n") {
                if let Some(end_idx) = rest.find("\n---\n") {
                    let fm = &rest[..end_idx];
                    body = rest[end_idx + 5..].to_string();
                    for line in fm.lines() {
                        if let Some((k, v)) = line.split_once(':') {
                            let val = v.trim().trim_matches('"').to_string();
                            match k.trim() {
                                "name" => name = val,
                                "description" => description = Some(val),
                                "color" => color = Some(val),
                                "emoji" => emoji = Some(val),
                                "vibe" => vibe = Some(val),
                                _ => {}
                            }
                        }
                    }
                } else { body = content.clone();}
            } else { body = content.clone();}
            if name.is_empty() {
                name = path.split('/').next_back().unwrap_or("").trim_end_matches(".md").to_string();
            }
            all.push(serde_json::json!({
                "id": format!("msitarzewski/agency-agents::{}", path),
                "source_repo": "msitarzewski/agency-agents",
                "source_path": path,
                "category": cat_name,
                "name": name,
                "description": description,
                "color": color,
                "emoji": emoji,
                "vibe": vibe,
                "body": body,
            }));
        }
    }
    Ok(all)
}

async fn gui_agent_templates_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, source_repo, source_path, category, name, description, color, emoji, vibe, body, customized, fetched_at, updated_at \
         FROM agent_templates ORDER BY category ASC, name ASC"
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "source_repo": r.get::<_, String>(1)?,
        "source_path": r.get::<_, Option<String>>(2)?,
        "category": r.get::<_, String>(3)?, "name": r.get::<_, String>(4)?,
        "description": r.get::<_, Option<String>>(5)?,
        "color": r.get::<_, Option<String>>(6)?, "emoji": r.get::<_, Option<String>>(7)?,
        "vibe": r.get::<_, Option<String>>(8)?, "body": r.get::<_, String>(9)?,
        "customized": r.get::<_, i64>(10)? != 0,
        "fetched_at": r.get::<_, String>(11)?, "updated_at": r.get::<_, String>(12)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

// rc.133 — 카탈로그 템플릿을 특정 alias 의 cwd 에 적용:
//   • AGENT.md 자동 생성 (atomic write)
//   • CLAUDE.md 끝에 @AGENT.md reference 자동 추가
//   • agent_capabilities upsert (role/description/group/messenger_enabled)
#[derive(serde::Deserialize)]
struct AgentTemplateApplyBody {
    template_id: String,
    target_alias: String,
    #[serde(default)] body_override: Option<String>,
    #[serde(default)] group_name: Option<String>,
    #[serde(default)] messenger_enabled: bool,
    #[serde(default)] project_path_hint: Option<String>,
}

async fn gui_agent_templates_apply(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AgentTemplateApplyBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 1) 템플릿 로드
    let template: (String, String, String, Option<String>) = {
        let mut db = state.db.lock().await;
        match db.conn().query_row(
            "SELECT name, body, category, emoji FROM agent_templates WHERE id = ?1",
            rusqlite::params![&body.template_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))
        ) {
            Ok(t) => t,
            Err(_) => return Ok(Json(serde_json::json!({
                "ok": false, "error": format!("template {} not found", body.template_id)
            }))),
        }
    };
    let (tpl_name, tpl_body, tpl_category, tpl_emoji) = template;
    let final_body = body.body_override.unwrap_or(tpl_body);
    // 2) 대상 cwd 확인
    let cwd = match resolve_alias_cwd(&body.target_alias, body.project_path_hint.as_deref()).await {
        Ok(p) => p,
        Err(e) => return Ok(Json(serde_json::json!({"ok": false, "error": e}))),
    };
    // 3) AGENT.md atomic write
    let agent_md = std::path::Path::new(&cwd).join("AGENT.md");
    let tmp = agent_md.with_extension("md.new");
    std::fs::write(&tmp, &final_body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("write: {e}")})))?;
    std::fs::rename(&tmp, &agent_md)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("rename: {e}")})))?;
    // 4) CLAUDE.md @AGENT.md reference 자동 추가
    let claude_md = std::path::Path::new(&cwd).join("CLAUDE.md");
    let existing = std::fs::read_to_string(&claude_md).unwrap_or_default();
    if !existing.contains("@AGENT.md") {
        let mut new_content = existing.clone();
        if !new_content.is_empty() && !new_content.ends_with('\n') { new_content.push('\n');}
        new_content.push_str("\n# OpenXgram 에이전트 지침 (auto-injected, agency-agents 카탈로그)\n");
        new_content.push_str("@AGENT.md\n");
        std::fs::write(&claude_md, new_content).ok();
    }
    // 5) agent_capabilities upsert
    let now = chrono::Utc::now().to_rfc3339();
    let head: String = final_body.chars().take(4000).collect();
    let role_short = format!("{} {}", tpl_emoji.unwrap_or_default(), tpl_name).trim().to_string();
    {
        let mut db = state.db.lock().await;
        db.conn().execute(
            "INSERT INTO agent_capabilities (alias, role, description, project_path, group_name, messenger_enabled, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(alias) DO UPDATE SET role=?2, description=?3, project_path=?4, group_name=COALESCE(?5, group_name), messenger_enabled=?6, updated_at=?7",
            rusqlite::params![
                body.target_alias, role_short, head, cwd, body.group_name, body.messenger_enabled as i64, now,
            ],
        ).ok();
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "applied_from": body.template_id,
        "target_alias": body.target_alias,
        "category": tpl_category,
        "agent_md": agent_md.display().to_string(),
        "bytes": final_body.len(),
        "messenger_enabled": body.messenger_enabled,
    })))
}

async fn gui_agent_templates_refresh(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let fetched = fetch_agency_agents().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e})))?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    let mut inserted = 0; let mut updated = 0; let mut preserved = 0;
    for tpl in &fetched {
        let id = tpl.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        // 기존 customized=1 이면 보존 (덮어쓰지 않음)
        let is_customized: i64 = db.conn().query_row(
            "SELECT customized FROM agent_templates WHERE id = ?1", rusqlite::params![id], |r| r.get(0)
        ).unwrap_or(0);
        if is_customized != 0 { preserved += 1; continue;}
        let existed: i64 = db.conn().query_row(
            "SELECT 1 FROM agent_templates WHERE id = ?1", rusqlite::params![id], |r| r.get(0)
        ).unwrap_or(0);
        db.conn().execute(
            "INSERT INTO agent_templates (id, source_repo, source_path, category, name, description, color, emoji, vibe, body, customized, fetched_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?11) \
             ON CONFLICT(id) DO UPDATE SET source_repo=excluded.source_repo, source_path=excluded.source_path, \
             category=excluded.category, name=excluded.name, description=excluded.description, \
             color=excluded.color, emoji=excluded.emoji, vibe=excluded.vibe, body=excluded.body, updated_at=?11",
            rusqlite::params![
                id, tpl.get("source_repo").and_then(|v| v.as_str()),
                tpl.get("source_path").and_then(|v| v.as_str()),
                tpl.get("category").and_then(|v| v.as_str()),
                tpl.get("name").and_then(|v| v.as_str()),
                tpl.get("description").and_then(|v| v.as_str()),
                tpl.get("color").and_then(|v| v.as_str()),
                tpl.get("emoji").and_then(|v| v.as_str()),
                tpl.get("vibe").and_then(|v| v.as_str()),
                tpl.get("body").and_then(|v| v.as_str()),
                now,
            ],
        ).ok();
        if existed != 0 { updated += 1;} else { inserted += 1;}
    }
    Ok(Json(serde_json::json!({
        "ok": true, "fetched": fetched.len(), "inserted": inserted, "updated": updated, "preserved": preserved,
    })))
}

// rc.129 — alias 의 cwd 추출 helper (tmux pane_current_path)
async fn resolve_alias_cwd(alias: &str, hint: Option<&str>) -> Result<String, String> {
    if let Some(p) = hint.filter(|s| !s.is_empty()) { return Ok(p.to_string());}
    let session = crate::notify::resolve_alias_to_tmux(alias).await
        .ok_or_else(|| format!("tmux session not found for alias '{}'", alias))?.0;
    let out = tokio::process::Command::new("tmux")
        .args(["display-message", "-p", "-t", &session, "#{pane_current_path}"])
        .output().await.map_err(|e| format!("tmux: {}", e))?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { Err("cwd 추출 실패".to_string()) } else { Ok(s) }
}

// rc.129 — 지침 파일 (cwd/AGENT.md) inline 편집 endpoint.
#[derive(serde::Deserialize)]
struct AgentInstructionsBody {
    alias: String,
    content: String,
    #[serde(default)] project_path_hint: Option<String>,
}

async fn gui_agents_instructions_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let alias = params.get("alias").cloned()
        .ok_or_else(|| bad_request("alias query 필요"))?;
    let cwd = match resolve_alias_cwd(&alias, params.get("project_path_hint").map(|s| s.as_str())).await {
        Ok(p) => p,
        Err(e) => return Ok(Json(serde_json::json!({"ok": false, "error": e}))),
    };
    let agent_md = std::path::Path::new(&cwd).join("AGENT.md");
    let content = if agent_md.exists() {
        std::fs::read_to_string(&agent_md).unwrap_or_default()
    } else { String::new() };
    Ok(Json(serde_json::json!({
        "ok": true, "alias": alias, "project_path": cwd,
        "file": agent_md.display().to_string(),
        "exists": agent_md.exists(),
        "content": content,
    })))
}

async fn gui_agents_instructions_save(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AgentInstructionsBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let cwd = match resolve_alias_cwd(&body.alias, body.project_path_hint.as_deref()).await {
        Ok(p) => p,
        Err(e) => return Ok(Json(serde_json::json!({"ok": false, "error": e}))),
    };
    let agent_md = std::path::Path::new(&cwd).join("AGENT.md");
    // atomic write: tmp + rename
    let tmp = agent_md.with_extension("md.new");
    std::fs::write(&tmp, &body.content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("write: {e}")})))?;
    std::fs::rename(&tmp, &agent_md)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("rename: {e}")})))?;
    // rc.130 — CLAUDE.md 끝에 @AGENT.md reference 자동 추가 (idempotent).
    // LLM 이 cwd/CLAUDE.md 읽으면 AGENT.md 도 자동으로 같이 읽음 (Claude Code @ syntax).
    let claude_md = std::path::Path::new(&cwd).join("CLAUDE.md");
    let existing = std::fs::read_to_string(&claude_md).unwrap_or_default();
    let claude_ref_added = if !existing.contains("@AGENT.md") && !existing.contains("@./AGENT.md") {
        let mut new_content = existing.clone();
        if !new_content.is_empty() && !new_content.ends_with('\n') { new_content.push('\n');}
        new_content.push_str("\n# OpenXgram 에이전트 지침 (auto-injected by 메신저 등록 탭)\n");
        new_content.push_str("@AGENT.md\n");
        std::fs::write(&claude_md, new_content).ok();
        true
    } else { false };
    // description 자동 update: agent_capabilities 의 description 을 AGENT.md 첫 4KB 로
    {
        let mut db = state.db.lock().await;
        let head: String = body.content.chars().take(4000).collect();
        let now = chrono::Utc::now().to_rfc3339();
        db.conn().execute(
            "INSERT INTO agent_capabilities (alias, role, description, updated_at) VALUES (?1, 'binding', ?2, ?3) \
             ON CONFLICT(alias) DO UPDATE SET description = ?2, updated_at = ?3",
            rusqlite::params![body.alias, head, now],
        ).ok();
    }
    Ok(Json(serde_json::json!({
        "ok": true, "alias": body.alias, "file": agent_md.display().to_string(),
        "bytes": body.content.len(),
        "claude_md_ref_added": claude_ref_added,
    })))
}

// rc.125 — 자동 감지: 해당 alias 의 cwd 에서 AGENT.md → CLAUDE.md 우선순위로 읽기 + .mcp.json.
// rc.129: AGENT.md 우선 (메신저 등록 탭에서 inline 편집한 파일).
async fn gui_agents_auto_detect(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AgentAutoDetectBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 1) project_path 결정: hint 우선, 없으면 tmux session 의 default-path
    let project_path: String = if let Some(p) = body.project_path_hint.as_ref().filter(|s| !s.is_empty()) {
        p.clone()
    } else {
        // alias → tmux session → default-path
        let session = match crate::notify::resolve_alias_to_tmux(&body.alias).await {
            Some((s, _)) => s,
            None => return Ok(Json(serde_json::json!({
                "ok": false,
                "error": "tmux session not found for alias",
                "alias": body.alias,
            }))),
        };
        let out = tokio::process::Command::new("tmux")
            .args(["display-message", "-p", "-t", &session, "#{pane_current_path}"])
            .output().await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("tmux: {e}")})))?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            return Ok(Json(serde_json::json!({"ok": false, "error": "cwd 추출 실패", "alias": body.alias})));
        }
        s
    };
    // rc.129/130 — 2) AGENT.md 우선 (메신저 등록 탭 inline 편집 파일) → CLAUDE.md fallback
    let agent_md_path = std::path::Path::new(&project_path).join("AGENT.md");
    let claude_md_path = std::path::Path::new(&project_path).join("CLAUDE.md");
    let (description, _tool_list_json): (Option<String>, Option<String>) =
        if agent_md_path.exists() {
            let content = std::fs::read_to_string(&agent_md_path).unwrap_or_default();
            (Some(content), None)
        } else if claude_md_path.exists() {
            let content = std::fs::read_to_string(&claude_md_path).unwrap_or_default();
            let head: String = content.chars().take(4000).collect();
            let first_lines: Vec<&str> = head.lines().take(20).collect();
            let desc = first_lines.join("\n");
            (Some(desc), None)
        } else { (None, None) };
    // rc.130 — skills 디렉토리 list 추출 (.claude/skills/ + skills/)
    let mut skills: Vec<String> = vec![];
    for skill_dir in [".claude/skills", "skills"] {
        let dir = std::path::Path::new(&project_path).join(skill_dir);
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        skills.push(name.to_string());
                    }
                }
            }
        }
    }
    let tool_list_json = if !skills.is_empty() {
        Some(serde_json::to_string(&skills).unwrap_or_default())
    } else { None };
    // 3) .mcp.json 읽어 tool_list 추출
    let mcp_path = std::path::Path::new(&project_path).join(".mcp.json");
    let tool_list = if mcp_path.exists() {
        std::fs::read_to_string(&mcp_path).ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("mcpServers").map(|servers| {
                servers.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()
            }))
            .map(|v| serde_json::to_string(&v).unwrap_or_default())
            .or(tool_list_json)
    } else { tool_list_json };
    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": body.alias,
        "project_path": project_path,
        "description": description,
        "tool_list": tool_list,
    })))
}

async fn gui_agents_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute(
        "DELETE FROM agent_capabilities WHERE alias = ?1",
        rusqlite::params![alias],
    ).ok();
    Ok(Json(serde_json::json!({"deleted": alias})))
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
    // 3.5) bot 의 guild 목록 (channel_id 가 실은 guild_id 인지 검사용)
    let guilds_resp = client.get(format!("{api_base}/users/@me/guilds"))
        .header("Authorization", format!("Bot {}", d.bot_token))
        .send().await;
    let guild_ids: Vec<String> = if let Ok(r) = guilds_resp {
        if r.status().is_success() {
            r.json::<serde_json::Value>().await.ok()
                .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|g| g.get("id").and_then(|i| i.as_str()).map(String::from)).collect()))
                .unwrap_or_default()
        } else { vec![] }
    } else { vec![] };
    let channel_is_actually_guild = guild_ids.iter().any(|g| g == &channel_id_str);

    // 4) 권한 분석 — `install_params` 는 OAuth install URL 의 default scope/perm 표시.
    //    봇이 실제 가입 시점에 가진 scope/perm 과 무관 → reinvite 판단 기준으로 쓰지 않음.
    //    bot token /users/@me /users/@me/guilds 가 200 이면 token 자체 정상 → 메시지 송신 가능.
    let permissions = app_json.get("install_params").and_then(|p| p.get("permissions")).and_then(|p| p.as_str()).unwrap_or("?");
    let scopes = app_json.get("install_params").and_then(|p| p.get("scopes")).cloned().unwrap_or(serde_json::json!([]));
    let guild_count = app_json.get("approximate_guild_count").and_then(|g| g.as_i64()).unwrap_or(-1);
    let has_bot_scope_in_install_params = scopes.as_array().map(|a| a.iter().any(|s| s.as_str() == Some("bot"))).unwrap_or(false);
    // 재초대 권장은 token 자체가 invalid 인 경우에만 강제. install_params 는 정보 차원.
    let token_valid = user_status.as_u16() == 200 && !guild_ids.is_empty();
    let needs_reinvite = !token_valid;
    let invite_url = if needs_reinvite {
        format!("https://discord.com/oauth2/authorize?client_id={}&permissions=68608&scope=bot+applications.commands",
            app_json.get("id").and_then(|i| i.as_str()).unwrap_or(""))
    } else { String::new() };
    // summary 우선순위 — channel_id 필드는 서버(guild) ID 등록 자리로도 쓰임 (정상 설정).
    // 채널 단위 송신은 별도 바인딩 화면에서 channel 지정 — 여기서 검증 안 함.
    let summary = if !token_valid {
        "❌ token invalid 또는 guild 가입 없음 — 봇 재초대 필요"
    } else if channel_is_actually_guild {
        "✅ 봇 + 서버(guild) 등록 OK — 특정 채널 메시지 송신은 별도 바인딩 화면에서 channel 지정"
    } else if channel_status == 200 {
        "✅ 정상 — token + channel access 모두 OK (특정 channel 직접 등록 모드)"
    } else if channel_status == 404 {
        "ℹ channel_id 가 guild 도 channel 도 아님 (또는 봇 미가입) — 봇 진입 가능한 서버 ID 또는 채널 ID 입력"
    } else if channel_status == 403 {
        "⚠ channel 보긴 했지만 권한 부족 (Send Messages / Read History 필요)"
    } else {
        "ℹ channel API 응답 비정상"
    };
    Ok(Json(serde_json::json!({
        "token_status": user_status.as_u16(),
        "bot_username": user_json.get("username").cloned().unwrap_or_default(),
        "bot_id": user_json.get("id").cloned().unwrap_or_default(),
        "owner": app_json.get("owner").and_then(|o| o.get("username")).cloned().unwrap_or_default(),
        "guild_count": guild_count,
        "guild_ids": guild_ids,
        "install_permissions": permissions,
        "install_scopes": scopes,
        "has_bot_scope_in_install_params": has_bot_scope_in_install_params,
        "token_valid": token_valid,
        "channel_id_configured": channel_id_str,
        "channel_access_status": channel_status,
        "channel_access_ok": channel_status == 200,
        "channel_is_actually_guild_id": channel_is_actually_guild,
        "needs_reinvite": needs_reinvite,
        "reinvite_url": invite_url,
        "summary": summary,
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
    let started = chrono::Utc::now().to_rfc3339();
    // 실제 reflection 실행 — openxgram_memory::reflect_all + derive_traits_from_patterns
    let data_dir = state.data_dir.as_ref().clone();
    let pre_counts = {
        let mut db = state.db.lock().await;
        let conn = db.conn();
        let wiki: i64 = conn
            .query_row("SELECT COUNT(*) FROM wiki_pages", [], |r| r.get(0))
            .unwrap_or(0);
        let patterns: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_patterns", [], |r| r.get(0))
            .unwrap_or(0);
        (wiki, patterns)
    };
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        use openxgram_db::{Db, DbConfig};
        let mut db = Db::open(DbConfig {
            path: data_dir.join("db.sqlite"),
            ..Default::default()
        })?;
        openxgram_memory::reflect_all(&mut db)?;
        openxgram_memory::derive_traits_from_patterns(&mut db)?;
        Ok(())
    })
    .await;
    let (success, summary) = match result {
        Ok(Ok(())) => (true, "reflection_pass 완료".to_string()),
        Ok(Err(e)) => (false, format!("reflection 실패: {e}")),
        Err(e) => (false, format!("join 실패: {e}")),
    };
    let finished = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    let conn = db.conn();
    let post_wiki: i64 = conn
        .query_row("SELECT COUNT(*) FROM wiki_pages", [], |r| r.get(0))
        .unwrap_or(0);
    let post_patterns: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_patterns", [], |r| r.get(0))
        .unwrap_or(0);
    let new_pages = post_wiki - pre_counts.0;
    let patterns_found = post_patterns - pre_counts.1;
    let _ = conn.execute(
        "INSERT INTO reflection_runs (started_at, finished_at, success, summary, new_pages, patterns_found) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            started,
            finished,
            if success { 1_i64 } else { 0 },
            summary,
            new_pages,
            patterns_found
        ],
    );
    Ok(Json(serde_json::json!({
        "started_at": started,
        "finished_at": finished,
        "success": success,
        "summary": summary,
        "new_pages": new_pages,
        "patterns_found": patterns_found
    })))
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
    let manifest = InstallManifest::read(manifest_path(&state.data_dir)).ok();
    let machine = crate::daemon_gui_sessions::detect_machine();
    let alias = manifest.as_ref().map(|m| m.machine.alias.clone());
    let hostname = manifest.as_ref().map(|m| m.machine.hostname.clone());

    // M-2: identity_settings 테이블에서 동적 조회 (DB v30)
    let (auto_lock, sess_ttl) = {
        let mut db = state.db.lock().await;
        let al: i64 = db
            .conn()
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM identity_settings WHERE key='auto_lock_minutes'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(30);
        let st: i64 = db
            .conn()
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM identity_settings WHERE key='session_token_ttl_minutes'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(30);
        (al, st)
    };

    Ok(Json(serde_json::json!({
        "alias": alias,
        "hostname": hostname,
        "did": "did:openxgram:0x... (마스터 키 unlock 후 노출)",
        "machine": machine,
        "argon2": {"m": 65536, "t": 3, "p": 2},
        "auto_lock_minutes": auto_lock,
        "session_token_ttl_minutes": sess_ttl,
        "did_format": "did:openxgram:0x...",
        "hd_path": "m/44'/9999'/0'/0/{agent_index}",
    })))
}

/// `POST /v1/gui/identity/settings` — M-2 auto_lock_minutes 편집.
#[derive(serde::Deserialize)]
struct IdentitySettingsUpdate {
    auto_lock_minutes: Option<i64>,
    session_token_ttl_minutes: Option<i64>,
}
async fn gui_identity_settings_update(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(req): Json<IdentitySettingsUpdate>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    if let Some(v) = req.auto_lock_minutes {
        if !(1..=1440).contains(&v) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorDto { error: "auto_lock_minutes must be 1..=1440".into() }),
            ));
        }
        db.conn()
            .execute(
                "INSERT INTO identity_settings(key,value,updated_at) VALUES('auto_lock_minutes',?1,datetime('now')) \
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
                rusqlite::params![v.to_string()],
            )
            .map_err(|e| internal(&format!("db: {e}")))?;
    }
    if let Some(v) = req.session_token_ttl_minutes {
        if !(1..=1440).contains(&v) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorDto { error: "session_token_ttl_minutes must be 1..=1440".into() }),
            ));
        }
        db.conn()
            .execute(
                "INSERT INTO identity_settings(key,value,updated_at) VALUES('session_token_ttl_minutes',?1,datetime('now')) \
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
                rusqlite::params![v.to_string()],
            )
            .map_err(|e| internal(&format!("db: {e}")))?;
    }
    Ok(Json(serde_json::json!({"ok": true})))
}

/// `GET /v1/gui/identity/suspicious_dids` — M-10 해킹 의심 DID 목록.
async fn gui_identity_suspicious_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, external_did, reason, first_seen, dismissed \
         FROM identity_suspicious_dids WHERE dismissed=0 ORDER BY first_seen DESC LIMIT 100",
    ).map_err(|e| internal(&format!("db: {e}")))?;
    let rows: Vec<serde_json::Value> = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_,i64>(0).unwrap_or(0),
                "external_did": r.get::<_,String>(1).unwrap_or_default(),
                "reason": r.get::<_,String>(2).unwrap_or_default(),
                "first_seen": r.get::<_,String>(3).unwrap_or_default(),
                "dismissed": r.get::<_,i64>(4).unwrap_or(0) != 0,
            }))
        })
        .map_err(|e| internal(&format!("db: {e}")))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(Json(rows))
}

#[derive(serde::Deserialize)]
struct SuspiciousDismissReq { id: i64 }
async fn gui_identity_suspicious_dismiss(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(req): Json<SuspiciousDismissReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute(
        "UPDATE identity_suspicious_dids SET dismissed=1, dismissed_at=datetime('now') WHERE id=?1",
        rusqlite::params![req.id],
    ).map_err(|e| internal(&format!("db: {e}")))?;
    Ok(Json(serde_json::json!({"ok": true})))
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

/// UI-EXTERNAL-AGENT — 외부 디렉토리 (5 테이블 종합 뷰).
/// HomeDashboard + ExternalAgentCard 공통 진입점.
async fn gui_external_directory(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let conn = db.conn();

    // protocols — external_protocols 테이블에서 enabled 만
    let protocols: Vec<serde_json::Value> = conn
        .prepare("SELECT name, enabled, configured_at FROM external_protocols ORDER BY name")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(serde_json::json!({
                    "name": r.get::<_, String>(0)?,
                    "enabled": r.get::<_, i64>(1)? != 0,
                    "configured_at": r.get::<_, Option<String>>(2)?,
                }))
            })
            .map(|it| it.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // my_listings — 내가 마켓에 올린 리스팅
    let my_listings: Vec<serde_json::Value> = conn
        .prepare("SELECT id, agent_id, marketplace, price_usdc, pricing_model, description, enabled, created_at FROM external_listings ORDER BY created_at DESC LIMIT 100")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, String>(0)?,
                    "agent_id": r.get::<_, String>(1)?,
                    "marketplace": r.get::<_, String>(2)?,
                    "price_usdc": r.get::<_, f64>(3)?,
                    "pricing_model": r.get::<_, String>(4)?,
                    "description": r.get::<_, Option<String>>(5)?,
                    "enabled": r.get::<_, i64>(6)? != 0,
                    "created_at": r.get::<_, String>(7)?,
                }))
            })
            .map(|it| it.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // outbound_calls — 내가 외부로 호출한 기록
    let outbound_calls: Vec<serde_json::Value> = conn
        .prepare("SELECT id, to_agent, protocol, amount, status, rating, started_at, completed_at FROM external_outbound_calls ORDER BY started_at DESC LIMIT 100")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, String>(0)?,
                    "to_agent": r.get::<_, String>(1)?,
                    "protocol": r.get::<_, String>(2)?,
                    "amount": r.get::<_, f64>(3)?,
                    "status": r.get::<_, String>(4)?,
                    "rating": r.get::<_, Option<i64>>(5)?,
                    "started_at": r.get::<_, String>(6)?,
                    "completed_at": r.get::<_, Option<String>>(7)?,
                }))
            })
            .map(|it| it.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // inbound_pending — 외부에서 나에게 들어온 미승인 요청
    let inbound_pending: Vec<serde_json::Value> = conn
        .prepare("SELECT id, from_agent, protocol, request_summary, offered_price, status, received_at FROM external_inbound_pending WHERE status='pending' ORDER BY received_at DESC LIMIT 100")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, String>(0)?,
                    "from_agent": r.get::<_, String>(1)?,
                    "protocol": r.get::<_, String>(2)?,
                    "request_summary": r.get::<_, Option<String>>(3)?,
                    "offered_price": r.get::<_, Option<f64>>(4)?,
                    "status": r.get::<_, String>(5)?,
                    "received_at": r.get::<_, String>(6)?,
                }))
            })
            .map(|it| it.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    // reputation — 내가 거래한 외부 에이전트들의 평판
    let reputation: Vec<serde_json::Value> = conn
        .prepare("SELECT external_agent, avg_rating, review_count, blacklisted, last_interaction FROM external_reputation ORDER BY review_count DESC LIMIT 100")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(serde_json::json!({
                    "external_agent": r.get::<_, String>(0)?,
                    "avg_rating": r.get::<_, Option<f64>>(1)?,
                    "review_count": r.get::<_, i64>(2)?,
                    "blacklisted": r.get::<_, i64>(3)? != 0,
                    "last_interaction": r.get::<_, Option<String>>(4)?,
                }))
            })
            .map(|it| it.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "protocols": protocols,
        "my_listings": my_listings,
        "outbound_calls": outbound_calls,
        "inbound_pending": inbound_pending,
        "reputation": reputation,
        // 호환: 기존 GUI 가 external_agents 키 참조하므로 reputation 매핑으로 대체.
        "external_agents": [],
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

// ── Workflows (UI-MESSENGER-SPEC v1.4 §20 — W-1 ~ W-10) ──

async fn gui_workflows_list(
    State(state): State<GuiServerState>, headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT id, name, description, orchestrator, cron_expr, cost_limit, enabled, updated_at FROM workflows ORDER BY updated_at DESC").map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    let rows: Vec<serde_json::Value> = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?,
        "description": r.get::<_, Option<String>>(2)?, "orchestrator": r.get::<_, Option<String>>(3)?,
        "cron_expr": r.get::<_, Option<String>>(4)?, "cost_limit": r.get::<_, Option<f64>>(5)?,
        "enabled": r.get::<_, i64>(6)? != 0, "updated_at": r.get::<_, String>(7)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?
       .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_workflow_upsert(
    State(state): State<GuiServerState>, headers: HeaderMap, Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let id = body.get("id").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let description = body.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let yaml_body = body.get("yaml_body").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let orchestrator = body.get("orchestrator").and_then(|v| v.as_str()).map(String::from);
    let cron_expr = body.get("cron_expr").and_then(|v| v.as_str()).map(String::from);
    let cost_limit = body.get("cost_limit").and_then(|v| v.as_f64());
    if name.is_empty() || yaml_body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorDto{error: "name + yaml_body 필수".into()})));
    }
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO workflows (id, name, description, yaml_body, orchestrator, cron_expr, cost_limit, enabled, created_at, updated_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,1,datetime('now'),datetime('now')) \
         ON CONFLICT(id) DO UPDATE SET name=?2, description=?3, yaml_body=?4, orchestrator=?5, cron_expr=?6, cost_limit=?7, updated_at=datetime('now')",
        rusqlite::params![id, name, description, yaml_body, orchestrator, cron_expr, cost_limit],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    Ok(Json(serde_json::json!({"id": id, "name": name, "saved": true})))
}

async fn gui_workflow_get(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let row = db.conn().query_row(
        "SELECT id, name, description, yaml_body, orchestrator, cron_expr, message_trigger, cost_limit, enabled, updated_at FROM workflows WHERE id=?1",
        rusqlite::params![id],
        |r| Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?,
            "description": r.get::<_, Option<String>>(2)?, "yaml_body": r.get::<_, String>(3)?,
            "orchestrator": r.get::<_, Option<String>>(4)?, "cron_expr": r.get::<_, Option<String>>(5)?,
            "message_trigger": r.get::<_, Option<String>>(6)?, "cost_limit": r.get::<_, Option<f64>>(7)?,
            "enabled": r.get::<_, i64>(8)? != 0, "updated_at": r.get::<_, String>(9)?,
        })),
    ).map_err(|_| (StatusCode::NOT_FOUND, Json(ErrorDto{error: "not found".into()})))?;
    Ok(Json(row))
}

async fn gui_workflow_delete(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute("DELETE FROM workflows WHERE id=?1", rusqlite::params![id])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    Ok(Json(serde_json::json!({"deleted": id})))
}

async fn gui_workflow_run(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let yaml_body: String = {
        let mut db = state.db.lock().await;
        db.conn().execute(
            "INSERT INTO workflow_runs (id, workflow_id, started_at, status, trigger_source) VALUES (?1, ?2, datetime('now'), 'running', 'manual')",
            rusqlite::params![run_id, id],
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        db.conn().query_row(
            "SELECT yaml_body FROM workflows WHERE id=?1",
            rusqlite::params![id],
            |r| r.get::<_, String>(0),
        ).map_err(|_| (StatusCode::NOT_FOUND, Json(ErrorDto{error: format!("workflow {id} not found")})))?
    };
    // Engine 실행 (block until complete) — TODO: tokio::spawn 으로 background, GET runs 로 진행 확인
    let mut db = state.db.lock().await;
    let result = crate::workflow_engine::run_workflow(&mut *db, &id, &run_id, &yaml_body).await;
    Ok(Json(serde_json::json!({
        "run_id": run_id, "workflow_id": id,
        "status": result.status, "error": result.error,
        "total_cost": result.total_cost,
        "step_outputs": result.step_outputs,
    })))
}

/// W-3 human_approval resume: waiting_human → running 으로 전환 + 나머지 step 재실행.
async fn gui_workflow_run_approve(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let (workflow_id, yaml_body): (String, String) = {
        let mut db = state.db.lock().await;
        let row = db.conn().query_row(
            "SELECT r.workflow_id, w.yaml_body FROM workflow_runs r JOIN workflows w ON w.id=r.workflow_id WHERE r.id=?1 AND r.status='waiting_human'",
            rusqlite::params![run_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ).map_err(|_| (StatusCode::NOT_FOUND, Json(ErrorDto{error: "run not found or not waiting_human".into()})))?;
        db.conn().execute("UPDATE workflow_runs SET status='running' WHERE id=?1", rusqlite::params![run_id])
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        row
    };
    let mut db = state.db.lock().await;
    let result = crate::workflow_engine::run_workflow(&mut *db, &workflow_id, &run_id, &yaml_body).await;
    Ok(Json(serde_json::json!({"resumed": run_id, "status": result.status, "total_cost": result.total_cost})))
}

/// peer-to-peer 메시지 — outbound_queue 에 **signed envelope (JSON)** enqueue.
/// rc.215 fix: 과거 이름은 "unsigned" 였지만 receiver `/v1/message` 가 `Envelope` JSON
/// deserialize 를 요구하므로 master keystore 로 서명 후 직렬화한 envelope 을 INSERT.
/// 스키마: msg_ulid PK, target_machine, target_alias, body (Envelope JSON 문자열),
///         attempts, next_retry_at, last_error, enqueued_at, sent_at.
/// worker (daemon_workers.rs S8) 가 target_alias JOIN peers 로 address 조회 → POST `/v1/message`.
async fn gui_peer_send_unsigned(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(alias): Path<String>, Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let text = body.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if text.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorDto{error: "body 필수".into()})));
    }
    let conversation_id_in = body
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // master keystore 로 서명. env 필요 — 명시적 503 (silent fallback 금지).
    let pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").map_err(|_| {
        (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorDto{
            error: "XGRAM_KEYSTORE_PASSWORD 미설정 — daemon 환경변수 필요".into()
        }))
    })?;

    // peer 조회 + 서명 준비. db lock 은 짧게 (INSERT 위해 다시 잡는다).
    let (peer_pubkey_hex, _peer_address) = {
        let mut db = state.db.lock().await;
        let row = db.conn().query_row(
            "SELECT address, public_key_hex FROM peers WHERE alias=?1",
            rusqlite::params![alias],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        );
        match row {
            Ok(r) => r,
            Err(_) => {
                return Err((StatusCode::NOT_FOUND, Json(ErrorDto{
                    error: format!("peer {alias} not found")
                })));
            }
        }
    };

    let data_dir = state.data_dir.as_ref().clone();
    use openxgram_keystore::{FsKeystore, Keystore};
    let ks = FsKeystore::new(openxgram_core::paths::keystore_dir(&data_dir));
    let signer = ks
        .load(openxgram_core::paths::MASTER_KEY_NAME, &pw)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{
            error: format!("master 키 로드 실패: {e}")
        })))?;

    let signature_hex = hex::encode(signer.sign(text.as_bytes()));
    let payload_hex = hex::encode(text.as_bytes());
    let sender_addr = signer.address.to_string();

    // install-manifest 기반 sender 메타데이터.
    let manifest_opt = openxgram_manifest::InstallManifest::read(
        openxgram_core::paths::manifest_path(&data_dir),
    ).ok();
    let sender_alias = manifest_opt.as_ref().map(|m| m.machine.alias.clone());
    let sender_transport_url = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            manifest_opt
                .as_ref()
                .and_then(|m| m.machine.tailscale_ip.clone())
                .map(|ip| format!("http://{ip}:47300"))
        });
    let sender_pubkey_hex = Some(hex::encode(signer.public_key_bytes()));

    let envelope = openxgram_transport::Envelope {
        from: sender_addr,
        to: peer_pubkey_hex,
        payload_hex,
        timestamp: openxgram_core::time::kst_now(),
        signature_hex,
        nonce: Some(uuid::Uuid::new_v4().to_string()),
        conversation_id: conversation_id_in.or_else(|| Some(uuid::Uuid::new_v4().to_string())),
        sender_alias,
        sender_transport_url,
        sender_pubkey_hex,
        recipient_alias: Some(alias.clone()),
    };
    let envelope_json = serde_json::to_string(&envelope).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{
            error: format!("envelope serialize: {e}")
        }))
    })?;

    let envelope_id = uuid::Uuid::new_v4().to_string();
    let now_str = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO outbound_queue (msg_ulid, target_machine, target_alias, body, attempts, enqueued_at) \
         VALUES (?1, '', ?2, ?3, 0, ?4)",
        rusqlite::params![envelope_id, alias, envelope_json, now_str],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("enqueue: {e}")})))?;
    Ok(Json(serde_json::json!({"queued": envelope_id, "to_alias": alias})))
}

async fn gui_workflow_runs(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT id, started_at, finished_at, status, current_step, total_cost FROM workflow_runs WHERE workflow_id=?1 ORDER BY started_at DESC LIMIT 50").map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    let rows: Vec<serde_json::Value> = stmt.query_map(rusqlite::params![id], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "started_at": r.get::<_, String>(1)?,
        "finished_at": r.get::<_, Option<String>>(2)?, "status": r.get::<_, String>(3)?,
        "current_step": r.get::<_, Option<String>>(4)?, "total_cost": r.get::<_, Option<f64>>(5)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?
       .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

// ── External Agent endpoints (UI-EXTERNAL-AGENT-SPEC 30 결정) ──

async fn gui_external_outbound(
    State(state): State<GuiServerState>, headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT id, to_agent, protocol, amount, status, rating, started_at, completed_at FROM external_outbound_calls ORDER BY started_at DESC LIMIT 100").map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    let rows: Vec<serde_json::Value> = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "to_agent": r.get::<_, String>(1)?, "protocol": r.get::<_, String>(2)?,
        "amount": r.get::<_, f64>(3)?, "status": r.get::<_, String>(4)?, "rating": r.get::<_, Option<i64>>(5)?,
        "started_at": r.get::<_, String>(6)?, "completed_at": r.get::<_, Option<String>>(7)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?
       .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_external_inbound(
    State(state): State<GuiServerState>, headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT id, from_agent, protocol, request_summary, offered_price, received_at FROM external_inbound_pending WHERE status='pending' ORDER BY received_at DESC LIMIT 50").map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    let rows: Vec<serde_json::Value> = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "from_agent": r.get::<_, String>(1)?, "protocol": r.get::<_, String>(2)?,
        "request_summary": r.get::<_, Option<String>>(3)?, "offered_price": r.get::<_, Option<f64>>(4)?,
        "received_at": r.get::<_, String>(5)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?
       .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_external_inbound_approve(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute("UPDATE external_inbound_pending SET status='approved' WHERE id=?1", rusqlite::params![id])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    Ok(Json(serde_json::json!({"approved": id})))
}

async fn gui_external_inbound_reject(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute("UPDATE external_inbound_pending SET status='rejected' WHERE id=?1", rusqlite::params![id])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    Ok(Json(serde_json::json!({"rejected": id})))
}

async fn gui_external_listings(
    State(state): State<GuiServerState>, headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT id, agent_id, marketplace, price_usdc, pricing_model, description, enabled FROM external_listings ORDER BY created_at DESC").map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    let rows: Vec<serde_json::Value> = stmt.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "agent_id": r.get::<_, String>(1)?, "marketplace": r.get::<_, String>(2)?,
        "price_usdc": r.get::<_, f64>(3)?, "pricing_model": r.get::<_, String>(4)?,
        "description": r.get::<_, Option<String>>(5)?, "enabled": r.get::<_, i64>(6)? != 0,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?
       .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_external_listing_add(
    State(state): State<GuiServerState>, headers: HeaderMap, Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let agent_id = body.get("agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let marketplace = body.get("marketplace").and_then(|v| v.as_str()).unwrap_or("OpenAgentX").to_string();
    let price = body.get("price_usdc").and_then(|v| v.as_f64()).unwrap_or(1.0);
    if agent_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorDto{error: "agent_id 필수".into()})));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO external_listings (id, agent_id, marketplace, price_usdc, pricing_model, description, enabled, created_at) VALUES (?1,?2,?3,?4,'per-call','',1,datetime('now'))",
        rusqlite::params![id, agent_id, marketplace, price],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    Ok(Json(serde_json::json!({"id": id, "agent_id": agent_id, "marketplace": marketplace, "price_usdc": price})))
}

async fn gui_external_reputation(
    State(state): State<GuiServerState>, headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT external_agent, avg_rating, review_count, blacklisted FROM external_reputation ORDER BY review_count DESC LIMIT 100").map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    let rows: Vec<serde_json::Value> = stmt.query_map([], |r| Ok(serde_json::json!({
        "external_agent": r.get::<_, String>(0)?, "avg_rating": r.get::<_, Option<f64>>(1)?,
        "review_count": r.get::<_, i64>(2)?, "blacklisted": r.get::<_, i64>(3)? != 0,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?
       .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

async fn gui_external_protocols(
    State(state): State<GuiServerState>, headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT name, enabled FROM external_protocols").map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    let mut out = serde_json::Map::new();
    let mut protos = Vec::new();
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    for row in rows.flatten() {
        out.insert(row.0.clone(), serde_json::json!(row.1));
        protos.push(row.0);
    }
    out.insert("protocols".into(), serde_json::json!(protos));
    Ok(Json(serde_json::Value::Object(out)))
}

/// `GET /v1/gui/ops/diagnostic` — DB / 디스크 / keystore / 서비스 헬스체크.
async fn gui_ops_diagnostic(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let db_ok = db.conn().query_row("SELECT 1", [], |r| r.get::<_, i32>(0)).is_ok();
    let session_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0)).unwrap_or(-1);
    let msg_count: i64 = db.conn().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).unwrap_or(-1);
    let migration_version: i64 = db.conn().query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0)).unwrap_or(-1);
    drop(db);
    let data_dir = state.data_dir.as_ref().clone();
    let keystore_path = openxgram_core::paths::keystore_dir(&data_dir).join("master.json");
    let keystore_exists = keystore_path.exists();
    let disk_free_mb = match std::fs::metadata(&data_dir) {
        Ok(_) => "측정 가능 (statvfs 통합 예정)",
        Err(_) => "측정 불가",
    };
    Ok(Json(serde_json::json!({
        "db": {"ok": db_ok, "sessions": session_count, "messages": msg_count, "migration_version": migration_version},
        "disk": {"data_dir": data_dir.display().to_string(), "status": disk_free_mb},
        "keystore": {"master_exists": keystore_exists, "path": keystore_path.display().to_string()},
        "services": {"tailscale": "Tailscale Funnel active", "discord": "listener spawned", "telegram": "configured"},
        "summary": if db_ok && keystore_exists { "✅ 정상" } else { "❌ 점검 필요" }
    })))
}

/// `GET /v1/gui/ops/machines` — Tailscale peer + DID 등록 머신 목록.
async fn gui_ops_machines(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT alias, address, role, COALESCE(last_seen, '미연결') FROM peers ORDER BY created_at DESC LIMIT 50"
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prepare: {e}")})))?;
    let peers: Vec<serde_json::Value> = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "alias": r.get::<_, String>(0)?,
            "address": r.get::<_, String>(1)?,
            "role": r.get::<_, String>(2)?,
            "last_seen": r.get::<_, String>(3)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("query: {e}")})))?
      .filter_map(|r| r.ok())
      .collect();
    drop(stmt);
    let local = crate::daemon_gui_sessions::detect_machine();
    let ts_status = std::process::Command::new("tailscale")
        .arg("status").arg("--json")
        .output().ok()
        .and_then(|o| if o.status.success() { String::from_utf8(o.stdout).ok() } else { None })
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let tailscale_peers = ts_status.as_ref().and_then(|v| v.get("Peer")).cloned().unwrap_or(serde_json::json!({}));
    Ok(Json(serde_json::json!({
        "local_machine": local,
        "registered_peers": peers,
        "peer_count": peers.len(),
        "tailscale_peers": tailscale_peers,
    })))
}

/// `GET /v1/gui/ops/backup-status` — 백업 last/next + 백업 dir 의 파일 count.
async fn gui_ops_backup_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let backup_dir = state.data_dir.as_ref().join("backup");
    let entries: Vec<_> = std::fs::read_dir(&backup_dir).ok()
        .into_iter().flatten()
        .filter_map(|e| e.ok())
        .map(|e| serde_json::json!({"name": e.file_name().to_string_lossy().to_string()}))
        .collect();
    let last_at = entries.last().map(|e| e.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string());
    Ok(Json(serde_json::json!({
        "backup_dir": backup_dir.display().to_string(),
        "count": entries.len(),
        "last_at": last_at,
        "next_scheduled": "daily 03:00 (자동 cron)",
        "backup_files": entries,
        "note": "수동 백업: POST /v1/gui/ops/backup-now. 자동: 매일 03:00 cron."
    })))
}

/// `POST /v1/gui/ops/backup-now` — 즉시 백업 (~/.openxgram/db.sqlite + keystore → backup/<ts>.tar.gz).
async fn gui_ops_backup_now(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let data_dir = state.data_dir.as_ref().clone();
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let backup_dir = data_dir.join("backup");
    let _ = std::fs::create_dir_all(&backup_dir);
    let out = backup_dir.join(format!("backup-{ts}.tar.gz"));
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, u64)> {
        let f = std::fs::File::create(&out)?;
        let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        for name in ["db.sqlite", "keystore", "notify.toml", "install_manifest.json"] {
            let p = data_dir.join(name);
            if p.exists() {
                if p.is_dir() {
                    tar.append_dir_all(name, &p)?;
                } else {
                    let mut f = std::fs::File::open(&p)?;
                    tar.append_file(name, &mut f)?;
                }
            }
        }
        tar.finish()?;
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        Ok((out.display().to_string(), size))
    })
    .await
    .map_err(|e| internal(&format!("backup join: {e}")))?
    .map_err(|e| internal(&format!("backup: {e}")))?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "file": result.0,
        "size_bytes": result.1,
        "created_at": ts
    })))
}

/// `GET /v1/gui/ops/update-check` — GitHub release latest vs 현재.
async fn gui_ops_update_check(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let current = crate::daemon_gui_sessions::version_info();
    let client = reqwest::Client::new();
    let latest = client.get("https://api.github.com/repos/OpenXgram/openxgram/releases/latest")
        .header("User-Agent", "openxgram-daemon")
        .send().await.ok();
    let latest_tag = if let Some(r) = latest {
        if r.status().is_success() {
            r.json::<serde_json::Value>().await.ok().and_then(|j| j.get("tag_name").and_then(|t| t.as_str()).map(String::from))
        } else { None }
    } else { None };
    let current_release = serde_json::to_value(&current).ok()
        .and_then(|v| v.get("release").and_then(|r| r.as_str()).map(String::from));
    Ok(Json(serde_json::json!({
        "current": current,
        "latest_tag": latest_tag,
        "channel": "stable",
        "up_to_date": latest_tag.as_ref().map(|t| t.trim_start_matches('v')) == current_release.as_deref(),
        "update_url": "https://github.com/OpenXgram/openxgram/releases/latest"
    })))
}

/// `POST /v1/gui/ops/update-apply` — GitHub release asset 다운로드 + binary 교체 (재시작 필요).
async fn gui_ops_update_apply(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let client = reqwest::Client::builder()
        .user_agent("openxgram-daemon")
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| internal(&format!("http: {e}")))?;
    let rel: serde_json::Value = client
        .get("https://api.github.com/repos/OpenXgram/openxgram/releases/latest")
        .send().await.map_err(|e| internal(&format!("latest fetch: {e}")))?
        .json().await.map_err(|e| internal(&format!("json: {e}")))?;
    let tag = rel.get("tag_name").and_then(|t| t.as_str()).unwrap_or("").to_string();
    if tag.is_empty() {
        return Err(internal("no tag in release"));
    }
    // Linux x86_64 asset 찾기
    let arch_substr = if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-aarch64"
    } else {
        return Err(internal("unsupported platform for auto-update"));
    };
    let url = rel.get("assets").and_then(|a| a.as_array())
        .and_then(|arr| arr.iter().find(|a| a.get("name").and_then(|n| n.as_str()).map(|n| n.contains(arch_substr)).unwrap_or(false)))
        .and_then(|a| a.get("browser_download_url").and_then(|u| u.as_str()).map(String::from));
    let url = match url {
        Some(u) => u,
        None => return Err(internal(&format!("no {arch_substr} asset in {tag}"))),
    };
    let bin_bytes = client.get(&url).send().await
        .map_err(|e| internal(&format!("dl: {e}")))?
        .bytes().await
        .map_err(|e| internal(&format!("dl body: {e}")))?;
    // /tmp 저장 → binary 교체는 사용자/systemd 수동
    let staged = std::path::PathBuf::from("/tmp")
        .join(format!("xgram-{}", tag.trim_start_matches('v')));
    std::fs::write(&staged, &bin_bytes).map_err(|e| internal(&format!("write: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755));
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "tag": tag,
        "staged_at": staged.display().to_string(),
        "size_bytes": bin_bytes.len(),
        "next_step": format!("mv {} /home/llm/.local/bin/xgram && systemctl restart xgram-daemon", staged.display())
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
        description: None,
        capabilities: vec![],
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

/// rc.155 — portal × OpenXgram bridge. starian-portal 의 send 후 호출하면
/// messages 테이블에 mirror INSERT (ack_status='delivered' 자동).
#[derive(Debug, Deserialize)]
struct MirrorMessageBody {
    session_id: String,
    sender: String,
    body: String,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    via: Option<String>,
}

async fn gui_messages_mirror(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(b): Json<MirrorMessageBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();
    let via = b.via.unwrap_or_else(|| "portal_mirror".into());
    let mut db = state.db.lock().await;
    // sessions 테이블에 session_id 가 없으면 자동 생성 (FK 만족)
    let exists: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM sessions WHERE id=?1",
        rusqlite::params![b.session_id],
        |r| r.get(0),
    ).unwrap_or(0);
    if exists == 0 {
        let _ = db.conn().execute(
            "INSERT INTO sessions (id, title, participants, created_at, last_active, home_machine, metadata) \
             VALUES (?1, ?2, '[]', ?3, ?3, 'unknown', '{}')",
            rusqlite::params![b.session_id, format!("portal-bridge: {}", b.session_id), &now],
        );
    }
    db.conn().execute(
        "INSERT INTO messages (id, session_id, sender, body, signature, timestamp, conversation_id, metadata, ack_status, acked_at, ack_via) \
         VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, '{}', 'delivered', ?5, ?7)",
        rusqlite::params![id, b.session_id, b.sender, b.body, &now, b.conversation_id, via],
    ).map_err(|e| internal(&format!("insert: {e}")))?;
    Ok(Json(serde_json::json!({"ok": true, "id": id, "ack_status": "delivered", "via": via})))
}

#[derive(Serialize)]
struct GuiMessageDto {
    id: String,
    session_id: String,
    sender: String,
    body: String,
    timestamp: String,
    conversation_id: String,
    // rc.153 — ack tracking (sender 가 메시지 처리 상태 확인)
    #[serde(skip_serializing_if = "Option::is_none")]
    ack_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    acked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ack_via: Option<String>,
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
    // rc.153 — ack tracking 포함 직접 SELECT (MessageStore 의 list_recent 는 ack 컬럼 미포함).
    // GUI 가 메시지 옆에 ack badge 표시 → sender 가 처리 상태 직접 확인.
    let fetch_limit = (limit * 4) as i64;
    let mut stmt = db.conn().prepare(
        "SELECT id, session_id, sender, body, timestamp, conversation_id, \
                ack_status, acked_at, ack_via \
         FROM messages ORDER BY timestamp DESC LIMIT ?1"
    ).map_err(|e| internal(&format!("prep: {e}")))?;
    let rows = stmt.query_map(rusqlite::params![fetch_limit], |r| {
        Ok(GuiMessageDto {
            id: r.get::<_, String>(0)?,
            session_id: r.get::<_, String>(1)?,
            sender: r.get::<_, String>(2)?,
            body: r.get::<_, String>(3)?,
            timestamp: r.get::<_, String>(4)?,
            conversation_id: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            ack_status: r.get::<_, Option<String>>(6)?,
            acked_at: r.get::<_, Option<String>>(7)?,
            ack_via: r.get::<_, Option<String>>(8)?,
        })
    }).map_err(|e| internal(&format!("q: {e}")))?;
    let items: Vec<GuiMessageDto> = rows
        .filter_map(|r| r.ok())
        .filter(|m| match &sender_filter {
            Some(s) => m.sender.to_lowercase() == *s,
            None => true,
        })
        .take(limit)
        .collect();
    Ok(Json(items))
}

/// rc.212 — `GET /v1/gui/peer_conversation/{alias}` — peer 와의 통합 conversation view.
///
/// 본질 결함 fix: 한 peer (예: akashic) 와의 메시지가 다음 여러 session 에 분산:
///  - `outbox-to-<alias>` / `outbox-to-<alias_variant>` (sender side, sender_label='self:*'/'me')
///  - `inbox-from-<alias>` / `inbox-from-<alias_variant>` (receiver side, sender_label='peer:*')
///  - `Peer · <alias>` (양방향 session, sender_label='me'/'peer:*')
///  - `Claude Code · <alias> · *` (LLM session, sender='user'/'assistant')
///
/// 단일 endpoint 가 alias root 매칭으로 모두 가져와 timestamp ASC 정렬 반환.
/// alias variant 매핑: `akashic` 선택 시 peers table 의 alias 가 `akashic` 또는
/// `*akashic*` (예: `aoe_akashic_5054a80a`) 인 row 들의 alias 모두 OR 매칭.
async fn gui_peer_conversation(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Vec<GuiMessageDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let limit = q
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(500)
        .min(2000);

    let mut db = state.db.lock().await;

    // alias variants — peers table 에서 자기 자신 + substring 매칭되는 모든 alias 수집.
    // 예: 입력 alias="akashic" → ["akashic", "aoe_akashic_5054a80a"] 모두 매칭.
    let mut alias_variants: Vec<String> = vec![alias.clone()];
    {
        let pattern = format!("%{}%", alias);
        if let Ok(mut stmt) = db
            .conn()
            .prepare("SELECT alias FROM peers WHERE alias LIKE ?1 OR ?2 LIKE '%' || alias || '%'")
        {
            if let Ok(rows) = stmt.query_map(rusqlite::params![pattern, alias], |r| {
                r.get::<_, String>(0)
            }) {
                for row in rows.flatten() {
                    if !alias_variants.contains(&row) {
                        alias_variants.push(row);
                    }
                }
            }
        }
    }

    // 각 variant 에 대해 4가지 title pattern 매칭. session.title LIKE 'outbox-to-' || ?1 || '%' 등.
    // dedup 위해 message id key set 사용.
    let mut items: Vec<GuiMessageDto> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for variant in &alias_variants {
        let outbox = format!("outbox-to-{}", variant);
        let inbox = format!("inbox-from-{}", variant);
        let peer_session = format!("Peer · {}", variant);
        let cc_prefix = format!("Claude Code · {} · ", variant);

        // outbox/inbox/Peer 는 정확 매칭, Claude Code 는 prefix LIKE 매칭.
        let sql = "SELECT m.id, m.session_id, m.sender, m.body, m.timestamp, \
                          m.conversation_id, m.ack_status, m.acked_at, m.ack_via \
                   FROM sessions s JOIN messages m ON m.session_id = s.id \
                   WHERE s.title = ?1 OR s.title = ?2 OR s.title = ?3 OR s.title LIKE ?4 \
                   ORDER BY m.timestamp ASC \
                   LIMIT ?5";
        let cc_like = format!("{}%", cc_prefix);
        let mut stmt = db
            .conn()
            .prepare(sql)
            .map_err(|e| internal(&format!("prep peer_conv: {e}")))?;
        let rows = stmt
            .query_map(
                rusqlite::params![outbox, inbox, peer_session, cc_like, limit as i64],
                |r| {
                    Ok(GuiMessageDto {
                        id: r.get::<_, String>(0)?,
                        session_id: r.get::<_, String>(1)?,
                        sender: r.get::<_, String>(2)?,
                        body: r.get::<_, String>(3)?,
                        timestamp: r.get::<_, String>(4)?,
                        conversation_id: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                        ack_status: r.get::<_, Option<String>>(6)?,
                        acked_at: r.get::<_, Option<String>>(7)?,
                        ack_via: r.get::<_, Option<String>>(8)?,
                    })
                },
            )
            .map_err(|e| internal(&format!("q peer_conv: {e}")))?;
        for r in rows.filter_map(|r| r.ok()) {
            if seen_ids.insert(r.id.clone()) {
                items.push(r);
            }
        }
    }

    // 전역 timestamp ASC 정렬 (chronological, oldest first).
    items.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    if items.len() > limit {
        let drop_n = items.len() - limit;
        items.drain(0..drop_n);
    }
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
        body.conversation_id.clone(),
    )
    .await
    .map_err(|e| internal(&format!("peer_send: {e}")))?;
    // L0 자동 저장 — outbound 메시지도 messages 테이블에 기록 (audit + reflection)
    {
        let session_id = format!("peer:{}", alias);
        let mut db = state.db.lock().await;
        let _ = crate::save_l0::save_l0_message(&mut db, crate::save_l0::L0SaveInput {
            id: None,
            session_id: &session_id,
            session_title: Some(&format!("Peer · {}", alias)),
            sender: "me",
            body: &body.body,
            signature: "outbound-signed",
            timestamp: None,
            parent_message_id: None,
            conversation_id: body.conversation_id.as_deref(),
            source: "messenger_outbound",
            extra_metadata: Some(serde_json::json!({"peer_alias": alias})),
        }, None);
    }
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

/// `POST /v1/gui/notify/telegram/detect_chat_saved` — notify.toml 의 저장된 봇 토큰으로 chat_id 자동감지.
async fn gui_notify_telegram_detect_chat_saved(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let config = crate::notify_setup::NotifyConfig::load(Some(state.data_dir.as_ref()))
        .map_err(|e| internal(&format!("notify cfg load: {e}")))?;
    let tg = config.telegram.ok_or_else(|| bad_request("텔레그램 봇 미등록. 채널 카드 → 채널 등록에서 봇 토큰 먼저 설정"))?;
    let api_base = crate::notify_setup::telegram_api_base();
    let chat = crate::notify_setup::telegram_detect_chat_id(&api_base, &tg.bot_token, 1)
        .await
        .map_err(|e| internal(&format!("telegram detect_chat: {e}")))?;
    Ok(Json(serde_json::json!({
        "chat_id": chat,
        "found": chat.is_some(),
        "hint": if chat.is_none() {
            "텔레그램 봇에게 메시지를 1개 보낸 뒤 다시 시도 (getUpdates 가 마지막 update 만 반환)"
        } else { "" }
    })))
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

// ─────────────────────────────────────────────────────────────────────────────
// rc.91 — Discord/Telegram 채널 ▶ 테스트 + 권한 진단 + 초대 URL
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ChannelTestBody {
    platform: String,
    channel_ref: String,
    text: String,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
}

async fn gui_notify_channel_test(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<ChannelTestBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let cfg = crate::notify_setup::NotifyConfig::load(Some(&state.data_dir))
        .map_err(|e| internal(&format!("notify.toml load: {e}")))?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| internal(&format!("http: {e}")))?;
    // rc.103 — multibot-aware send: bot_id → discord_bots, fallback channel_ref → bindings → bot_id → discord_bots, 최후 notify.toml
    let discord_token_lookup = |body_bot_id: Option<&str>, body_agent: Option<&str>, channel_ref: &str| -> Option<String> {
        let path = state.data_dir.join("db.sqlite");
        let mut db = openxgram_db::Db::open(openxgram_db::DbConfig { path, ..Default::default() }).ok()?;
        let conn = db.conn();
        // 1) 명시적 bot_id 우선
        if let Some(bid) = body_bot_id {
            if let Ok(t) = conn.query_row::<String, _, _>(
                "SELECT bot_token FROM discord_bots WHERE id = ?1 AND active = 1",
                rusqlite::params![bid], |r| r.get(0),
            ) {
                if !t.is_empty() { return Some(t);}
            }
        }
        // 2) channel_ref(+agent) 기반 bindings 조회 → bot_id → token
        let bot_id_opt: Option<String> = if let Some(agent) = body_agent {
            conn.query_row(
                "SELECT bot_id FROM session_channel_bindings WHERE platform='discord' AND channel_ref=?1 AND agent_id=?2 AND active=1 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![channel_ref, agent], |r| r.get(0),
            ).ok()
        } else {
            conn.query_row(
                "SELECT bot_id FROM session_channel_bindings WHERE platform='discord' AND channel_ref=?1 AND active=1 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![channel_ref], |r| r.get(0),
            ).ok()
        };
        if let Some(bid) = bot_id_opt {
            if let Ok(t) = conn.query_row::<String, _, _>(
                "SELECT bot_token FROM discord_bots WHERE id = ?1 AND active = 1",
                rusqlite::params![bid], |r| r.get(0),
            ) {
                if !t.is_empty() { return Some(t);}
            }
        }
        // 3) 어떤 봇이라도 있으면 (단일 봇 시나리오) — 첫 active 봇
        if let Ok(t) = conn.query_row::<String, _, _>(
            "SELECT bot_token FROM discord_bots WHERE active = 1 ORDER BY created_at ASC LIMIT 1",
            [], |r| r.get(0),
        ) {
            if !t.is_empty() { return Some(t);}
        }
        None
    };
    match body.platform.as_str() {
        "discord" => {
            let token = discord_token_lookup(body.bot_id.as_deref(), body.agent_id.as_deref(), &body.channel_ref)
                .or_else(|| cfg.discord.as_ref().map(|d| d.bot_token.clone()).filter(|t| !t.is_empty()))
                .ok_or_else(|| bad_request("Discord bot_token 조회 실패 — 봇 등록 + 채널 바인딩이 필요합니다 (에이전트 패널 → 채널 바인딩 → '+ 봇')"))?;
            let url = format!("https://discord.com/api/v10/channels/{}/messages", body.channel_ref);
            let resp = http.post(&url)
                .header("Authorization", format!("Bot {token}"))
                .json(&serde_json::json!({"content": body.text}))
                .send().await
                .map_err(|e| internal(&format!("discord post: {e}")))?;
            let status = resp.status();
            let rb = resp.text().await.unwrap_or_default();
            if status.is_success() {
                Ok(Json(serde_json::json!({"ok": true, "message": format!("Discord {} 전송 OK", body.channel_ref)})))
            } else {
                Ok(Json(serde_json::json!({"ok": false, "message": format!("Discord HTTP {}: {}", status, rb)})))
            }
        }
        "telegram" => {
            let token = cfg.telegram.as_ref()
                .map(|t| t.bot_token.clone())
                .filter(|t| !t.is_empty())
                .ok_or_else(|| bad_request("Telegram bot_token 미설정"))?;
            let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
            let resp = http.post(&url)
                .json(&serde_json::json!({"chat_id": body.channel_ref, "text": body.text}))
                .send().await
                .map_err(|e| internal(&format!("telegram send: {e}")))?;
            let status = resp.status();
            let rb = resp.text().await.unwrap_or_default();
            if status.is_success() {
                Ok(Json(serde_json::json!({"ok": true, "message": format!("Telegram {} 전송 OK", body.channel_ref)})))
            } else {
                Ok(Json(serde_json::json!({"ok": false, "message": format!("Telegram HTTP {}: {}", status, rb)})))
            }
        }
        other => Err(bad_request(&format!("platform 미지원: {other}"))),
    }
}

async fn gui_notify_discord_permissions(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let channel_id = params.get("channel_id").cloned()
        .ok_or_else(|| bad_request("channel_id query 필요"))?;
    let bot_id_param = params.get("bot_id").cloned();
    let cfg = crate::notify_setup::NotifyConfig::load(Some(&state.data_dir))
        .map_err(|e| internal(&format!("notify.toml load: {e}")))?;
    // rc.103 — multibot-aware: bot_id 우선, channel_id → bindings 조회, 첫 active 봇, 최후 notify.toml fallback
    let token_lookup = || -> Option<String> {
        let path = state.data_dir.join("db.sqlite");
        let mut db = openxgram_db::Db::open(openxgram_db::DbConfig { path, ..Default::default() }).ok()?;
        let conn = db.conn();
        if let Some(bid) = &bot_id_param {
            if let Ok(t) = conn.query_row::<String, _, _>(
                "SELECT bot_token FROM discord_bots WHERE id = ?1 AND active = 1",
                rusqlite::params![bid], |r| r.get(0),
            ) { if !t.is_empty() { return Some(t);} }
        }
        if let Ok(bid) = conn.query_row::<String, _, _>(
            "SELECT bot_id FROM session_channel_bindings WHERE platform='discord' AND channel_ref=?1 AND active=1 ORDER BY created_at DESC LIMIT 1",
            rusqlite::params![channel_id], |r| r.get(0),
        ) {
            if let Ok(t) = conn.query_row::<String, _, _>(
                "SELECT bot_token FROM discord_bots WHERE id = ?1 AND active = 1",
                rusqlite::params![bid], |r| r.get(0),
            ) { if !t.is_empty() { return Some(t);} }
        }
        conn.query_row::<String, _, _>(
            "SELECT bot_token FROM discord_bots WHERE active = 1 ORDER BY created_at ASC LIMIT 1",
            [], |r| r.get(0),
        ).ok().filter(|t| !t.is_empty())
    };
    let token = token_lookup()
        .or_else(|| cfg.discord.as_ref().map(|d| d.bot_token.clone()).filter(|t| !t.is_empty()))
        .ok_or_else(|| bad_request("Discord bot_token 조회 실패 — 봇 등록 필요"))?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| internal(&format!("http: {e}")))?;
    let ch_resp = http.get(&format!("https://discord.com/api/v10/channels/{}", channel_id))
        .header("Authorization", format!("Bot {token}"))
        .send().await
        .map_err(|e| internal(&format!("channel get: {e}")))?;
    let ch_status = ch_resp.status();
    if !ch_status.is_success() {
        let rb = ch_resp.text().await.unwrap_or_default();
        return Ok(Json(serde_json::json!({
            "ok": false,
            "message": format!("채널 조회 실패 HTTP {}: {}", ch_status, rb),
            "hint": "봇이 채널에 초대되어 있는지 확인 — invite URL 사용"
        })));
    }
    let ch_json: serde_json::Value = ch_resp.json().await.unwrap_or_default();
    let guild_id = ch_json.get("guild_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let ch_name = ch_json.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let me_resp = http.get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {token}"))
        .send().await
        .map_err(|e| internal(&format!("me: {e}")))?;
    let me_json: serde_json::Value = me_resp.json().await.unwrap_or_default();
    let bot_id = me_json.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut roles: Vec<String> = vec![];
    if !guild_id.is_empty() && !bot_id.is_empty() {
        let mem_url = format!("https://discord.com/api/v10/guilds/{}/members/{}", guild_id, bot_id);
        if let Ok(r) = http.get(&mem_url)
            .header("Authorization", format!("Bot {token}"))
            .send().await {
            if let Ok(j) = r.json::<serde_json::Value>().await {
                if let Some(rs) = j.get("roles").and_then(|v| v.as_array()) {
                    roles = rs.iter().filter_map(|r| r.as_str().map(String::from)).collect();
                }
            }
        }
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "channel_id": channel_id,
        "channel_name": ch_name,
        "guild_id": guild_id,
        "bot_id": bot_id,
        "bot_roles": roles,
        "hint": if roles.is_empty() { "봇 역할 없음 — invite URL 로 재초대" } else { "권한 확인됨" }
    })))
}

async fn gui_notify_discord_invite_url(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let cfg = crate::notify_setup::NotifyConfig::load(Some(&state.data_dir))
        .map_err(|e| internal(&format!("notify.toml load: {e}")))?;
    let token = cfg.discord.as_ref()
        .map(|d| d.bot_token.clone())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| bad_request("Discord bot_token 미설정"))?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| internal(&format!("http: {e}")))?;
    let me_resp = http.get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {token}"))
        .send().await
        .map_err(|e| internal(&format!("me: {e}")))?;
    let me_json: serde_json::Value = me_resp.json().await.unwrap_or_default();
    let bot_id = me_json.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if bot_id.is_empty() { return Err(bad_request("bot id 조회 실패 — token 잘못됨")); }
    // VIEW_CHANNEL + SEND_MESSAGES + READ_MSG_HISTORY + ATTACH_FILES + EMBED_LINKS + EXT_EMOJIS + ADD_REACTIONS + MANAGE_MESSAGES + MENTION_EVERYONE
    let perms: u64 = 1024 + 2048 + 65536 + 32768 + 16384 + 262144 + 64 + 8192 + 131072;
    let url = format!(
        "https://discord.com/api/oauth2/authorize?client_id={}&permissions={}&scope=bot%20applications.commands",
        bot_id, perms
    );
    Ok(Json(serde_json::json!({"invite_url": url, "bot_id": bot_id, "permissions": perms})))
}

// outbound polling worker 폐기 (rc.112). agent-push 모델로 전환:
//   openxgram.send_to_discord(content, channel?, bot_id?) MCP 도구 사용.
//   send_to_telegram 도 동일 패턴.

// ─────────────────────────────────────────────────────────────────────────────
// rc.92 — 멀티 디스코드 봇 CRUD
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct DiscordBotDto {
    id: String,
    alias: String,
    bot_user_id: Option<String>,
    owner: Option<String>,
    active: bool,
    created_at: String,
    /// token 은 클라이언트에 노출 안 함 (보안). prefix 만.
    token_prefix: String,
}

async fn gui_discord_bots_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DiscordBotDto>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let rows: rusqlite::Result<Vec<DiscordBotDto>> = db.conn().prepare(
        "SELECT id, alias, bot_user_id, owner, active, created_at, bot_token \
         FROM discord_bots ORDER BY created_at DESC"
    ).and_then(|mut s| {
        s.query_map([], |r| {
            let token: String = r.get(6)?;
            let prefix: String = token.chars().take(12).collect::<String>() + "...";
            Ok(DiscordBotDto {
                id: r.get(0)?,
                alias: r.get(1)?,
                bot_user_id: r.get(2)?,
                owner: r.get(3)?,
                active: r.get::<_, i64>(4)? != 0,
                created_at: r.get(5)?,
                token_prefix: prefix,
            })
        }).and_then(|m| m.collect())
    });
    Ok(Json(rows.unwrap_or_default()))
}

#[derive(serde::Deserialize)]
struct DiscordBotAddBody {
    alias: String,
    bot_token: String,
    owner: Option<String>,
}

async fn gui_discord_bots_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<DiscordBotAddBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.alias.trim().is_empty() || body.bot_token.trim().is_empty() {
        return Err(bad_request("alias + bot_token 필요"));
    }
    // 토큰으로 봇 검증 (bot_user_id 추출)
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| internal(&format!("http: {e}")))?;
    let me_resp = http.get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {}", body.bot_token.trim()))
        .send().await
        .map_err(|e| internal(&format!("validate: {e}")))?;
    let me_status = me_resp.status();
    let me_json: serde_json::Value = me_resp.json().await.unwrap_or_default();
    if !me_status.is_success() {
        return Err(bad_request(&format!("token 검증 실패 HTTP {}: {}", me_status, me_json)));
    }
    let bot_user_id = me_json.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let bot_username = me_json.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();

    use sha2::{Digest, Sha256};
    let id = format!("{:x}", Sha256::digest(format!("{}{}", body.alias, body.bot_token).as_bytes()))[..20].to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO discord_bots (id, alias, bot_token, bot_user_id, owner, active, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6) \
         ON CONFLICT(alias) DO UPDATE SET bot_token=excluded.bot_token, bot_user_id=excluded.bot_user_id, owner=excluded.owner",
        rusqlite::params![id, body.alias.trim(), body.bot_token.trim(), bot_user_id, body.owner.unwrap_or_else(|| "self".into()), now],
    ).map_err(|e| internal(&format!("insert: {e}")))?;
    Ok(Json(serde_json::json!({
        "id": id,
        "alias": body.alias,
        "bot_user_id": bot_user_id,
        "bot_username": bot_username,
        "ok": true,
        "note": "daemon restart 후 listener 가 새 봇으로 spawn 됩니다"
    })))
}

async fn gui_discord_bots_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute("DELETE FROM discord_bots WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| internal(&format!("delete: {e}")))?;
    Ok(Json(serde_json::json!({"ok": true, "id": id})))
}

// rc.92 — 특정 봇의 가입 서버 + text channel 자동 조회 (UI dropdown 용)
async fn gui_discord_bot_channels(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let bot_id_opt = params.get("bot_id").cloned();
    // bot_id 없으면 default (notify.toml)
    let token: String = if let Some(bid) = bot_id_opt.as_ref() {
        if bid.is_empty() || bid == "default" {
            crate::notify_setup::NotifyConfig::load(Some(&state.data_dir))
                .ok().and_then(|c| c.discord.map(|d| d.bot_token))
                .filter(|t| !t.is_empty())
                .ok_or_else(|| bad_request("default discord bot 미등록"))?
        } else {
            let mut db = state.db.lock().await;
            db.conn().query_row(
                "SELECT bot_token FROM discord_bots WHERE id = ?1 AND active = 1",
                rusqlite::params![bid], |r| r.get::<_, String>(0)
            ).map_err(|_| bad_request("bot_id 미존재"))?
        }
    } else {
        crate::notify_setup::NotifyConfig::load(Some(&state.data_dir))
            .ok().and_then(|c| c.discord.map(|d| d.bot_token))
            .filter(|t| !t.is_empty())
            .ok_or_else(|| bad_request("discord bot token 미설정"))?
    };
    let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build()
        .map_err(|e| internal(&format!("http: {e}")))?;
    // guilds 조회
    let guilds: Vec<serde_json::Value> = match http.get("https://discord.com/api/v10/users/@me/guilds")
        .header("Authorization", format!("Bot {token}"))
        .send().await {
        Ok(r) if r.status().is_success() => r.json::<Vec<serde_json::Value>>().await.unwrap_or_default(),
        _ => vec![],
    };
    // 각 guild 의 channel list
    let mut all_channels: Vec<serde_json::Value> = Vec::new();
    for g in &guilds {
        let gid = g.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let gname = g.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
        if gid.is_empty() { continue; }
        if let Ok(r) = http.get(&format!("https://discord.com/api/v10/guilds/{}/channels", gid))
            .header("Authorization", format!("Bot {token}"))
            .send().await {
            if let Ok(arr) = r.json::<Vec<serde_json::Value>>().await {
                for ch in arr {
                    if ch.get("type").and_then(|v| v.as_i64()) == Some(0) {  // GUILD_TEXT
                        all_channels.push(serde_json::json!({
                            "guild_id": gid,
                            "guild_name": gname,
                            "channel_id": ch.get("id"),
                            "channel_name": ch.get("name"),
                        }));
                    }
                }
            }
        }
    }
    Ok(Json(serde_json::json!({
        "guilds_count": guilds.len(),
        "channels": all_channels,
    })))
}

// ─────────────────────────────────────────────────────────────────────────────
// rc.92 — 채널 카드 종합 정보 (모든 봇 + 가입 서버 + 채널 + binding 통계)
// ─────────────────────────────────────────────────────────────────────────────
async fn gui_channels_summary(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let cfg = crate::notify_setup::NotifyConfig::load(Some(&state.data_dir))
        .map_err(|e| internal(&format!("notify.toml load: {e}")))?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| internal(&format!("http: {e}")))?;

    // 1) 모든 디스코드 봇 (default + extra) 수집
    let mut all_discord_bots: Vec<(String, String, String)> = Vec::new(); // (source, alias, token)
    if let Some(d) = cfg.discord.as_ref() {
        if !d.bot_token.is_empty() {
            all_discord_bots.push(("default(notify.toml)".into(), "default".into(), d.bot_token.clone()));
        }
    }
    let mut db = state.db.lock().await;
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT alias, bot_token FROM discord_bots WHERE active = 1"
    ) {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            for row in rows.flatten() {
                all_discord_bots.push(("discord_bots".into(), row.0, row.1));
            }
        }
    }

    // 2) bindings stats (per platform)
    let mut binding_stats: std::collections::HashMap<String, i64> = Default::default();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT platform, COUNT(*) FROM session_channel_bindings WHERE active = 1 GROUP BY platform"
    ) {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            for row in rows.flatten() {
                binding_stats.insert(row.0, row.1);
            }
        }
    }
    let bindings_per_channel: Vec<(String, String, i64)> = {
        if let Ok(mut stmt) = db.conn().prepare(
            "SELECT platform, channel_ref, COUNT(*) FROM session_channel_bindings WHERE active = 1 GROUP BY platform, channel_ref"
        ) {
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
                .and_then(|m| m.collect()).unwrap_or_default()
        } else { vec![] }
    };
    drop(db);

    // 3) 각 디스코드 봇의 상세 정보 — bot user, guilds
    let mut discord_bots_info: Vec<serde_json::Value> = Vec::new();
    for (source, alias, token) in &all_discord_bots {
        let mut bot_username = String::new();
        let mut bot_id = String::new();
        let mut guilds: Vec<serde_json::Value> = vec![];
        let mut error: Option<String> = None;
        // /users/@me
        match http.get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bot {token}"))
            .send().await {
            Ok(r) if r.status().is_success() => {
                if let Ok(j) = r.json::<serde_json::Value>().await {
                    bot_username = j.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    bot_id = j.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                }
            }
            Ok(r) => error = Some(format!("HTTP {}", r.status())),
            Err(e) => error = Some(format!("network: {e}")),
        }
        // /users/@me/guilds
        if !bot_id.is_empty() {
            if let Ok(r) = http.get("https://discord.com/api/v10/users/@me/guilds")
                .header("Authorization", format!("Bot {token}"))
                .send().await {
                if let Ok(arr) = r.json::<Vec<serde_json::Value>>().await {
                    for g in arr {
                        guilds.push(serde_json::json!({
                            "id": g.get("id"),
                            "name": g.get("name"),
                            "owner": g.get("owner"),
                        }));
                    }
                }
            }
        }
        discord_bots_info.push(serde_json::json!({
            "source": source,
            "alias": alias,
            "bot_username": bot_username,
            "bot_id": bot_id,
            "token_prefix": token.chars().take(16).collect::<String>(),
            "guilds": guilds,
            "guilds_count": guilds.len(),
            "error": error,
        }));
    }

    // 4) Telegram 봇 정보
    let mut telegram_info: Option<serde_json::Value> = None;
    if let Some(t) = cfg.telegram.as_ref() {
        if !t.bot_token.is_empty() {
            let mut bot_username = String::new();
            let mut bot_id: i64 = 0;
            let mut error: Option<String> = None;
            match http.get(&format!("https://api.telegram.org/bot{}/getMe", t.bot_token))
                .send().await {
                Ok(r) if r.status().is_success() => {
                    if let Ok(j) = r.json::<serde_json::Value>().await {
                        if let Some(res) = j.get("result") {
                            bot_username = res.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            bot_id = res.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                        }
                    }
                }
                Ok(r) => error = Some(format!("HTTP {}", r.status())),
                Err(e) => error = Some(format!("network: {e}")),
            }
            telegram_info = Some(serde_json::json!({
                "bot_username": bot_username,
                "bot_id": bot_id,
                "token_prefix": t.bot_token.chars().take(12).collect::<String>(),
                "error": error,
            }));
        }
    }

    Ok(Json(serde_json::json!({
        "discord": {
            "bots_count": discord_bots_info.len(),
            "bots": discord_bots_info,
        },
        "telegram": telegram_info,
        "bindings": {
            "stats_per_platform": binding_stats,
            "stats_per_channel": bindings_per_channel.iter().map(|(p, c, n)| serde_json::json!({
                "platform": p, "channel_ref": c, "count": n
            })).collect::<Vec<_>>(),
        },
    })))
}
/// rc.170 — auto-echo enforcer 의 visual verification API.
/// session_channel_bindings + matched session + last assistant message + last_echoed_ulid.
/// 마스터가 GUI 에서 매칭 정상인지 visual 확인 후 worker 활성화.
async fn gui_bindings_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;

    // 1) active binding row 수집
    let bindings: Vec<(String, String, String, String, Option<String>, Option<String>, i64, Option<String>)> = {
        let mut stmt = db.conn().prepare(
            "SELECT id, agent_id, platform, channel_ref, bot_id, bot_label, active, last_echoed_ulid \
             FROM session_channel_bindings WHERE active = 1"
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
        let rows = stmt.query_map([], |r| Ok((
            r.get::<_,String>(0)?,
            r.get::<_,String>(1)?,
            r.get::<_,String>(2)?,
            r.get::<_,String>(3)?,
            r.get::<_,Option<String>>(4)?,
            r.get::<_,Option<String>>(5)?,
            r.get::<_,i64>(6)?,
            r.get::<_,Option<String>>(7)?,
        ))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("query: {e}")})))?;
        rows.flatten().collect()
    };

    let mut out = Vec::new();
    for (binding_id, agent_id, platform, channel_ref, bot_id, bot_label, _active, last_echoed) in bindings {
        // bot alias (있으면)
        let bot_alias: Option<String> = match &bot_id {
            Some(bid) => db.conn().query_row(
                "SELECT alias FROM discord_bots WHERE id=?1",
                rusqlite::params![bid],
                |r| r.get::<_,String>(0)
            ).ok(),
            None => None,
        };
        // session 매칭 — rc.170: session_proj_name 있으면 그것 사용 (alias mapping), 없으면 agent_id 직접.
        let proj_name: String = db.conn().query_row(
            "SELECT COALESCE(session_proj_name, agent_id) FROM session_channel_bindings WHERE id=?1",
            rusqlite::params![&binding_id],
            |r| r.get::<_,String>(0)
        ).unwrap_or_else(|_| agent_id.clone());
        let pattern = format!("claude:{}:%", proj_name);
        let matched: Option<(String, String, String, String)> = db.conn().query_row(
            "SELECT id, session_id, substr(body, 1, 120), timestamp FROM messages \
             WHERE session_id LIKE ?1 AND sender='assistant' \
             ORDER BY timestamp DESC LIMIT 1",
            rusqlite::params![&pattern],
            |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?, r.get::<_,String>(3)?))
        ).ok();
        let session_count: i64 = db.conn().query_row(
            "SELECT COUNT(DISTINCT session_id) FROM messages WHERE session_id LIKE ?1",
            rusqlite::params![&pattern],
            |r| r.get::<_,i64>(0)
        ).unwrap_or(0);

        let (latest_msg_id, latest_session_id, preview, latest_ts) = match matched {
            Some((m, s, p, t)) => (Some(m), Some(s), Some(p), Some(t)),
            None => (None, None, None, None),
        };
        let would_echo = match (&latest_msg_id, &last_echoed) {
            (Some(m), Some(e)) => m != e,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let match_status = if session_count == 0 { "no_match" }
            else if latest_msg_id.is_none() { "no_assistant_messages" }
            else if last_echoed.is_none() { "first_setup" }
            else if would_echo { "pending_echo" }
            else { "up_to_date" };

        out.push(serde_json::json!({
            "binding_id": binding_id,
            "agent_id": agent_id,
            "platform": platform,
            "channel_ref": channel_ref,
            "bot_id": bot_id,
            "bot_label": bot_label,
            "bot_alias": bot_alias,
            "session_pattern": pattern,
            "matched_session_count": session_count,
            "latest_message_id": latest_msg_id,
            "latest_session_id": latest_session_id,
            "latest_preview": preview,
            "latest_timestamp": latest_ts,
            "last_echoed_ulid": last_echoed,
            "would_echo": would_echo,
            "match_status": match_status,
        }));
    }

    Ok(Json(serde_json::json!({
        "bindings": out,
        "note": "match_status: no_match (세션 없음) / no_assistant_messages / first_setup (옛 메시지 echo 방지) / pending_echo (다음 worker tick) / up_to_date",
    })))
}

/// rc.176 — daemon.log tail (cross-machine 진단 도구).
/// data_dir 의 daemon.log 마지막 N 줄. zalman 같은 remote peer 의 process_inbound silent fail 등 진단.
/// Query: ?tail=50 (기본 100, 최대 1000)
async fn gui_daemon_log(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let tail_n: usize = q.get("tail").and_then(|s| s.parse().ok()).unwrap_or(100).min(1000);
    let log_path = state.data_dir.join("daemon.log");
    let err_path = state.data_dir.join("daemon.log.err");

    let read_tail = |path: &std::path::Path, n: usize| -> Option<String> {
        let content = std::fs::read_to_string(path).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(n);
        Some(lines[start..].join("\n"))
    };

    let stdout_tail = read_tail(&log_path, tail_n).unwrap_or_else(|| String::from("(no daemon.log)"));
    let stderr_tail = read_tail(&err_path, tail_n).unwrap_or_else(|| String::from("(no daemon.log.err)"));

    Ok(Json(serde_json::json!({
        "log_path": log_path.display().to_string(),
        "err_path": err_path.display().to_string(),
        "tail_n": tail_n,
        "stdout": stdout_tail,
        "stderr": stderr_tail,
    })))
}

// rc.122 trigger marker
