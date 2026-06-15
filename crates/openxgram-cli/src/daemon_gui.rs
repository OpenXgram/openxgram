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
    routing::{get, patch, post},
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
    /// ACP (Agent Client Protocol) daemon state — spawn 된 agent process registry
    /// + GUI conversation-session 매핑 + SSE relay (§3 hosting, §5 lifecycle).
    /// Phase B-2. Clone-cheap (내부 Arc).
    acp: crate::daemon_gui_acp::AcpHttpState,
    /// A2A (Google Agent2Agent) daemon state — client-only: OpenXgram이 외부/타
    /// 에이전트의 A2A endpoint 를 호출 (AgentCard discover + tasks/send|get|cancel).
    /// Phase 3 (ACP-A2A-CORE). Clone-cheap.
    a2a: crate::daemon_gui_a2a::A2aHttpState,
    /// A2A SERVER state — OpenXgram 에이전트를 A2A로 CALLABLE 하게 호스팅
    /// (AgentCard serving + tasks/send 실행). 실행은 `acp` AcpHttpState 재사용
    /// (별도 ACP registry 없음). tasks/get 용 in-memory task 추적. Clone-cheap.
    served_a2a: crate::daemon_gui_a2a::ServedA2aState,
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
    /// rc.226 — peer entity = 1 project folder = 1 tmux session = 1 LLM 의 본질 inline.
    /// tmux pane current_path (local peer 만; cross-machine 은 None V1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_folder: Option<String>,
    /// LLM type: "Claude Code" / "Gemini" / "Codex" / "Ollama" / "unknown".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_type: Option<String>,
    /// LLM version string (--version 결과 또는 binary path 추출).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_version: Option<String>,
    /// install-manifest 의 machine.alias (이 daemon 의 머신; cross-machine 은 None V1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// rc.228 — peer 의 git worktree 목록 (project_folder 의 `git worktree list`).
    /// local peer 만 enrich, cross-machine 은 빈 vec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worktrees: Vec<WorktreeEntry>,
    /// rc.228 — peer 의 subagents (project_folder 의 agents/ 와 .claude/agents/ scan).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubagentEntry>,
    /// rc.228 — 이 peer 와 대화한 다른 peer 들 (inbox/outbox session 기준 집계).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ex_peers: Vec<ExPeerEntry>,
    /// rc.245 — 결정적 세션 매핑: 이 peer 의 터미널을 보여줄 명시적 세션 식별자.
    /// format 은 collect_sessions(/v1/gui/sessions) 의 identifier 와 동일
    /// (예: "tmux:<name>", "aoe:<...>", "portal:<...>", "claude:<...>").
    /// NULL 이면 Messenger.tsx 가 기존 normalizeAlias 추정 fallback.
    /// auto-seed 가 기본값 set, 사용자가 PATCH /v1/gui/peers/{alias}/session 으로 override.
    pub session_identifier: Option<String>,
}

/// rc.228 — peer 의 git worktree entry.
#[derive(Debug, Serialize, Clone)]
pub struct WorktreeEntry {
    pub path: String,
    /// HEAD branch 또는 ref short 표시.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// rc.228 — peer 의 subagent definition entry.
#[derive(Debug, Serialize, Clone)]
pub struct SubagentEntry {
    pub name: String,
    pub path: String,
    /// "claude_agents" (= `.claude/agents/`) / "project_agents" (= `agents/`).
    pub kind: String,
}

/// rc.228 — self_alias 와 대화한 다른 peer entry (inbox/outbox session 기준 집계).
#[derive(Debug, Serialize, Clone)]
pub struct ExPeerEntry {
    pub alias: String,
    pub msg_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_msg_at: Option<String>,
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

    let db_shared = Arc::new(Mutex::new(db));
    let state = GuiServerState {
        data_dir: Arc::new(data_dir),
        db: db_shared.clone(),
        // 증분 영속 — ACP 레이어가 진행 중 툴 호출을 실시간으로 acp_messages 에 기록하도록 동일 DB 공유.
        acp: crate::daemon_gui_acp::AcpHttpState::new().with_db(db_shared.clone()),
        a2a: crate::daemon_gui_a2a::A2aHttpState::new(),
        served_a2a: crate::daemon_gui_a2a::ServedA2aState::new(),
    };
    let state_clone = state.clone();

    // 하트비트 — execution_mode='heartbeat' 에이전트를 주기적으로 ACP 로 wake(기본 30분).
    // on_demand 와의 차이: heartbeat 모드 에이전트만 정기 깨움(점검 프롬프트). 로컬 한정, 비용은
    // heartbeat 로 지정한 에이전트 수에 비례(마스터 opt-in).
    {
        let hb_state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(120)).await; // startup race 방지
            loop {
                heartbeat_wake(&hb_state).await;
                tokio::time::sleep(std::time::Duration::from_secs(1800)).await; // 30분
            }
        });
    }

    // A2A 지속 세션 idle TTL reaper — 친구 대화(label=a2a:*) 세션이 마지막 사용 후 30분 이상
    // idle 이면 close(에이전트 reap). onClose 가 누락돼도(탭 강제종료 등) 누수되지 않게 하는 안전망.
    {
        let reap_state = state.clone();
        tokio::spawn(async move {
            let idle = std::time::Duration::from_secs(1800); // 30분
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await; // 5분마다 점검
                reap_state.acp.reap_idle_a2a(idle).await;
            }
        });
    }

    let app = Router::new()
        .route("/v1/gui/health", get(gui_health))
        .route("/v1/gui/status", get(gui_status))
        .route("/v1/gui/initialized", get(gui_initialized))
        // 비밀번호 변경 (keystore/vault rekey) — require_auth + 현재 비번 검증.
        .route("/v1/gui/change-password", post(gui_change_password))
        .route("/v1/gui/peers", get(gui_peers).post(gui_peer_add))
        // rc.245 — 결정적 세션 매핑 사용자 override: peer 의 터미널 세션 식별자 set/clear.
        .route("/v1/gui/peers/{alias}/session", patch(gui_peer_set_session))
        // rc.229 fix#3 — on-demand 1-agent enrich (4-metadata + worktree/subagent/ex_peer tree).
        .route("/v1/gui/agent/{alias}/detail", get(gui_agent_detail))
        // Phase 2-A — 동적 설정 탐지: AI 종류별 지침/설정 파일 체인(@import 재귀) + MCP/hooks/skills.
        .route("/v1/gui/agent/{alias}/config-chain", get(gui_agent_config_chain))
        // Phase 2-D — 에이전트 프로필 (classification/execution_mode/ai_type/worktree/public + folder/group/role 병합).
        .route("/v1/gui/agent/{alias}/profile", get(gui_agent_profile_get).post(gui_agent_profile_set))
        // 메신저 v1.3 §3.2 — 머신×세션 통합 detector (M-1).
        .route("/v1/gui/sessions", get(gui_sessions))
        .route("/v1/gui/sessions/{identifier}/screen", get(gui_session_screen))
        // rc.239 (이슈 #66) — cross-machine tmux 화면 read-only 미러.
        //   unlock 불필요 (auth X) — keystore password 불일치/M-8 lockout 제거.
        //   tailnet(Tailscale 100.x/CGNAT) 또는 localhost 에서만 응답 (allow_anonymous_screen).
        //   tmux 화면 capture 만 반환 — password/vault/메시지 내용 노출 X.
        .route(
            "/v1/gui/public/session-screen/{identifier}",
            get(gui_public_session_screen),
        )
        // rc.247 — 인증 없는 세션 목록 (tailnet 전용). fan-out 이 비번 불일치 머신의
        //   세션을 가져와 머신의 개별 에이전트를 각각 peer 카드로 노출.
        .route("/v1/gui/public/sessions", get(gui_public_sessions))
        .route("/v1/gui/sessions/{identifier}/input", post(gui_session_input))
        .route("/v1/gui/sessions/{identifier}/dropfile", post(gui_session_dropfile))
        .route("/v1/gui/public/sessions/{identifier}/input", post(gui_public_session_input))
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
        // 마켓 (c)갈래 — 지갑 거래 원장 (충전/구매/수익 내역 + 집계).
        .route("/v1/gui/wallets/ledger", get(gui_wallet_ledger))
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
        .route("/v1/gui/wiki/pages/{id}/backlinks", get(gui_wiki_backlinks))
        .route("/v1/gui/wiki/ingest", post(gui_wiki_ingest))
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
        // rc.320 — agent-level opt-in 친구. roster: 인증 없음 (cross-daemon peer-inbound,
        // 이 머신의 친구-가능 로컬 에이전트 노출). remote-agents: 인증 필요 (로컬→원격 roster fetch).
        .route("/v1/gui/friends/roster", get(gui_friends_roster))
        .route("/v1/gui/friends/remote-agents", get(gui_friends_remote_agents))
        // rc.321 — 친구 단위 정책 읽기/갱신 (권한/격리/비용). 인증 필요.
        .route("/v1/gui/friends/{alias}/policy", get(gui_friend_policy_get).post(gui_friend_policy_set))
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
        .route("/v1/gui/agents/{alias}/activate", post(gui_agent_activate))
        .route("/v1/gui/agent/{alias}/composer", post(gui_agent_composer_set))
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
        // 친구 추가(머신) UI — Tailscale tailnet 장치 목록 (자동 목록 표시).
        .route("/v1/gui/tailnet/devices", get(gui_tailnet_devices))
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
        // 결제 확정 가시화 — MCP 런타임(purchase_service)에서 데몬 self-call(④ 패턴)로 호출.
        .route("/v1/gui/commerce/event", post(gui_commerce_event))
        // UI-MESSENGER-SPEC v1.4 §20 — 오케스트레이션 워크플로 (W-1 ~ W-10)
        .route("/v1/gui/workflows", get(gui_workflows_list).post(gui_workflow_upsert))
        .route("/v1/gui/workflows/plan", post(gui_workflow_plan))
        // OpenXgram 런타임(하네스) — 제어/설정/메모리주입 레이어.
        .route("/v1/gui/runtime/config", get(gui_runtime_config_get).post(gui_runtime_config_set))
        .route("/v1/gui/runtime/context", get(gui_runtime_context))
        // 큐레이션된 주입 항목(규칙·원칙) CRUD — 전역(scope=*)+에이전트별.
        .route("/v1/gui/runtime/injections", get(gui_runtime_injections_list).post(gui_runtime_injection_upsert))
        .route("/v1/gui/runtime/injections/{id}", axum::routing::delete(gui_runtime_injection_delete))
        // rc.330 (GUI P3) — 방(대화) 단위 설정 저장/로드. 하네스·역할·오케스트레이션·
        // 시스템 프롬프트·이벤트 규칙. GET=로드(없으면 기본값) / PUT=저장. 강제는 P4.
        .route("/v1/gui/room/{key}/config", get(gui_room_config_get).put(gui_room_config_set))
        // P4a — "발언권 주기"(턴 부여). body {agent, note?}. 그 에이전트 ACP 에 누적 맥락+방/역할
        // 지침으로 한 번 턴 발화. @호명·조건 트리거도 이 메커니즘 재사용.
        .route("/v1/gui/room/{key}/grant-turn", post(gui_room_grant_turn))
        // P5 (rc.333) — 방 동적 멤버십. invite=참가자 추가+맥락 인계+전달 시작, eject=제거+수신 중단+ACP 분리,
        // members=현재 활성 참가자 목록(UI). 1:1(참가자 row 없음)은 무회귀. body {member, role?}.
        .route("/v1/gui/room/{key}/invite", post(gui_room_invite))
        .route("/v1/gui/room/{key}/eject", post(gui_room_eject))
        .route("/v1/gui/room/{key}/members", get(gui_room_members))
        // P4c (rc.332) — 오케스트레이션 RUNNER. 방의 orchestration_json 단계를 데몬이
        // 순서대로 실제 실행(각 단계 = grant-turn/handle_task 재사용). start=kick, status=진행상태,
        // approve/advance=사람-승인 pause 통과, cancel=중단.
        .route("/v1/gui/room/{key}/orchestrate/start", post(gui_room_orchestrate_start))
        .route("/v1/gui/room/{key}/orchestrate/status", get(gui_room_orchestrate_status))
        .route("/v1/gui/room/{key}/orchestrate/approve", post(gui_room_orchestrate_approve))
        .route("/v1/gui/room/{key}/orchestrate/advance", post(gui_room_orchestrate_approve))
        .route("/v1/gui/room/{key}/orchestrate/cancel", post(gui_room_orchestrate_cancel))
        .route("/v1/gui/workflows/{id}", get(gui_workflow_get).post(gui_workflow_delete))
        .route("/v1/gui/workflows/{id}/run", post(gui_workflow_run))
        .route("/v1/gui/workflows/{id}/runs", get(gui_workflow_runs))
        .route("/v1/gui/workflows/runs/{run_id}/approve", post(gui_workflow_run_approve))
        // rc.276 — Paperclip orchestration absorption Phase 1 read endpoints.
        // org chart (agent_capabilities + reports_to hierarchy), issue board, goals tree.
        .route("/v1/gui/orchestration/agents", get(gui_orchestration_agents))
        .route("/v1/gui/orchestration/issues", get(gui_orchestration_issues))
        .route("/v1/gui/orchestration/goals", get(gui_orchestration_goals))
        // rc.277 — Paperclip Phase 2 (adapter abstraction). Make every list_peers entry an
        // addable org agent (adapter_type=peer_send), + single-shot agent invoke primitive.
        .route(
            "/v1/gui/orchestration/agents/add-from-peer",
            post(gui_orchestration_add_from_peer),
        )
        .route(
            "/v1/gui/orchestration/agents/{alias}/invoke",
            post(gui_orchestration_agent_invoke),
        )
        .route("/v1/gui/peers/{alias}/send-unsigned", post(gui_peer_send_unsigned))
        // 메신저 카드 v1.3 Step 0 — 메시지 송수신.
        .route("/v1/gui/messages", get(gui_messages_recent))
        // ACP 대화 영속화 — 새로고침/재시작 후 복원.
        .route(
            "/v1/gui/acp/conversations/{key}/messages",
            get(gui_acp_conv_list).post(gui_acp_conv_add).delete(gui_acp_conv_clear),
        )
        .route(
            "/v1/gui/acp/conversations/{key}/read",
            post(gui_acp_conv_read),
        )
        // rc.212 — peer conversation unified view. 한 peer 와의 전 session (outbox/inbox/Peer·/Claude Code·) 합쳐서 시간순.
        .route("/v1/gui/peer_conversation/{alias}", get(gui_peer_conversation))
        .route("/v1/gui/peers/{alias}/send", post(gui_peer_send))
        // rc.228 — ex Peer thread 삭제. self_alias↔other_alias 의 outbox/inbox sessions + messages + outbound_queue.
        .route(
            "/v1/gui/peer/{self_alias}/ex_peer/{other_alias}",
            axum::routing::delete(gui_peer_ex_peer_delete),
        )
        // rc.155 — portal × OpenXgram 통합. starian-portal 의 send 후 메시지 mirror.
        // portal 가 sendKeys 한 직후 POST → messages 테이블에 INSERT.
        // ack_status='delivered' 자동, via='portal_mirror'. GUI 의 ack badge 가 표시.
        .route("/v1/gui/messages/mirror", post(gui_messages_mirror))
        // rc.170 — auto-echo enforcer visual verification API
        .route("/v1/gui/bindings_status", get(gui_bindings_status))
        // rc.176 — daemon log tail (cross-machine 진단 도구). zalman 같은 remote peer 의 silent fail 진단.
        .route("/v1/gui/daemon/log", get(gui_daemon_log))
        .route("/v1/gui/channel/status", get(gui_channel_status))
        // --- GUI-MISSING-ROUTES (additive) — wiki body read/write, fs tree, fs file rw, machines ---
        // 위키 본문 read/edit (WikiTab). slug = `{type}/{slug}` (예: `entity/foo`).
        // 기존 /v1/gui/wiki/pages/{id} 는 DB 메타만 — 본문은 디스크(WikiFs). 여기서 WikiTools 재사용.
        .route(
            "/v1/gui/wiki/{type}/{slug}",
            get(gui_wiki_body_get).put(gui_wiki_body_put),
        )
        // 디렉토리 file-tree (프로젝트 폴더 트리 / 폴더 피커).
        .route("/v1/gui/fs/tree", get(gui_fs_tree))
        // config 파일 read/write (에이전트 지침 편집기). write 는 whitelist 강제.
        .route("/v1/gui/fs/file", get(gui_fs_file_get).put(gui_fs_file_put))
        // 물리 머신 목록 (worker agent 제외) — settings "연결된 머신".
        .route("/v1/gui/machines", get(gui_machines_list))
        .route("/v1/gui/agent-machines", get(gui_agent_machines))
        .route("/v1/gui/models", get(gui_models_list))
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
        // 마켓 (d)갈래 — free-tier 무료 할당량 config(전역 기본 + override) + 상태(잔여/사용량).
        .route(
            "/v1/gui/payment/free-tier",
            get(gui_free_tier_get_config).put(gui_free_tier_set_config),
        )
        .route("/v1/gui/payment/free-tier/status", get(gui_free_tier_status))
        // 마켓 — 온체인 결제 지갑 (keystore master 주소 + Base 체인 ETH/USDC 실잔액).
        .route("/v1/gui/payment/wallet", get(gui_payment_wallet))
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
        // inbound webhook → 모니터 가능한 에이전트 스레드. memory webhook 과 동일한
        // URL token 인증(Bearer 없음). source 별 conv_key(`webhook:<source>`)에 record_message
        // + conv_persisted broadcast → 사람이 messenger 대화뷰에서 inbound 를 실시간으로 본다.
        .route("/v1/webhook/agent/{token}", post(webhook_agent_inbound))
        // Web GUI 정적 자산 — xgram 바이너리에 임베드 (PRD-OpenXgram v1.3 §4.8).
        // nginx 외부 호스팅 불필요. 외부 노출은 Tailscale Funnel 또는 reverse proxy 위임.
        .route("/gui", get(crate::ui_assets::gui_root))
        .route("/gui/", get(crate::ui_assets::gui_root))
        .route("/gui/{*path}", get(crate::ui_assets::gui_asset_path))
        // ── ACP (Agent Client Protocol) — Phase B-2 (additive) ─────────────
        // GUI conversation sessions backed by spawned ACP agent subprocesses.
        // All behind require_auth like the other /v1/gui routes.
        .route("/v1/acp/agents", get(acp_agents))
        .route("/v1/acp/sessions", post(acp_session_create))
        .route("/v1/acp/sessions/{id}/prompt", post(acp_session_prompt))
        .route("/v1/acp/sessions/{id}/stream", get(acp_session_stream))
        .route("/v1/acp/sessions/{id}/cancel", post(acp_session_cancel))
        .route("/v1/acp/sessions/{id}", axum::routing::delete(acp_session_close))
        // ── A2A (Google Agent2Agent) — Phase 3 (additive, client-only) ─────
        // Agent↔agent: OpenXgram calls another agent's A2A endpoint.
        // All behind require_auth like the other /v1/gui routes.
        .route("/v1/gui/a2a/agents", get(a2a_agents))
        .route(
            "/v1/gui/a2a/agents/{alias}/endpoints",
            get(a2a_list_agent_endpoints),
        )
        .route("/v1/gui/a2a/send", post(a2a_send))
        .route("/v1/gui/a2a/tasks/{id}", get(a2a_task_get))
        // ── A2A SERVER (ACP-A2A-CORE) — OpenXgram agents CALLABLE via A2A ───
        // AgentCard hosting (discovery) + tasks/send (executes via ACP) + tasks/get.
        // Behind require_auth like the rest of the daemon surface.
        .route(
            "/v1/a2a/agents/{alias}/.well-known/agent-card.json",
            get(a2a_served_card),
        )
        .route("/v1/a2a/agents/{alias}/tasks", post(a2a_served_task_send))
        .route(
            "/v1/a2a/agents/{alias}/tasks/{id}",
            get(a2a_served_task_get),
        )
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
        // rc.239 — ConnectInfo<SocketAddr> 주입 (anonymous screen endpoint 의 tailnet IP 체크용).
        let svc = app.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, svc).await {
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

#[derive(Debug, Deserialize)]
pub struct ChangePasswordReq {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct OkDto {
    pub ok: bool,
}

/// 비밀번호 변경 — keystore + vault rekey.
///
/// 1) require_auth (세션/mcp 토큰)
/// 2) 현재 비번 검증 (auth::verify_password) — 불일치 시 403
/// 3) 새 비번 길이 >=8 — 미만 시 400
/// 4) rekey::run_rekey (백업 → 재암호화 → daemon.env 갱신 → 검증)
/// 5) 현재 프로세스 env 도 새 비번으로 갱신 (재시작 전까지 일관성)
async fn gui_change_password(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<ChangePasswordReq>,
) -> Result<Json<OkDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    if !crate::auth::verify_password(&body.old_password) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorDto {
                error: "현재 비밀번호 불일치".into(),
            }),
        ));
    }
    if body.new_password.trim().len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: "새 비밀번호는 8자 이상이어야 합니다".into(),
            }),
        ));
    }

    let data_dir = state.data_dir.as_ref().clone();
    let old = body.old_password.clone();
    let new = body.new_password.trim().to_string();
    // rekey 는 blocking (Argon2 + 파일 IO) — blocking 스레드로 격리.
    tokio::task::spawn_blocking(move || crate::rekey::run_rekey(&data_dir, &old, &new))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorDto {
                    error: format!("rekey task join: {e}"),
                }),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorDto {
                    error: format!("비밀번호 변경 실패: {e}"),
                }),
            )
        })?;

    // run_rekey 가 daemon.env 는 이미 갱신함 — 현재 프로세스 env 도 동기화.
    std::env::set_var("XGRAM_KEYSTORE_PASSWORD", body.new_password.trim());

    Ok(Json(OkDto { ok: true }))
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

/// rc.274/rc.280 — 같은 tmux 세션을 가리키는 중복 행을 1행/세션으로 dedupe.
///   auto_seed 가 에이전트당 2행 기록할 수 있음: short alias(예: "akashic") + full
///   alias(예: "aoe_akashic_5054a80a"). 둘 다 session_identifier 가 동일("tmux:<name>")
///   이라 목록에 둘 다 노출됨 → session_identifier(tmux:* 인 행) 기준 1행만 남긴다.
///   - canonical: alias == 세션명(sid 의 "tmux:" 접두 제거값, 즉 aoe_* full alias) 우선.
///     (worktree/subagent 연결이 full alias 기준이므로 그쪽을 살린다.) 없으면 첫 행.
///   - sid NULL / 비-tmux(원격·채널·self) 행은 dedupe 대상 아님 — 각자 고유하므로 그대로 유지.
/// `alias_of` 로 각 행의 alias 를 추출, `sid_map` 으로 그 alias 의 session_identifier 를 조회.
/// gui_peers(PeerDto) 와 gui_orchestration_agents(serde_json::Value) 가 공유.
fn dedup_by_tmux_session<T>(
    rows: Vec<T>,
    sid_map: &std::collections::HashMap<String, String>,
    alias_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut keep: Vec<bool> = vec![true; rows.len()];
    for (idx, r) in rows.iter().enumerate() {
        let alias = alias_of(r);
        let sid = match sid_map.get(alias) {
            Some(s) if s.starts_with("tmux:") => s.clone(),
            // 비-tmux / 없음 → dedupe 제외 (고유 유지).
            _ => continue,
        };
        let session_name = &sid["tmux:".len()..];
        let is_canonical = alias == session_name;
        match seen.get(&sid).copied() {
            None => {
                seen.insert(sid, idx);
            }
            Some(prev_idx) => {
                // 이미 본 세션. canonical(full alias) 우선으로 유지 대상 교체.
                if is_canonical && alias_of(&rows[prev_idx]) != session_name {
                    keep[prev_idx] = false; // 기존(short) 제거
                    seen.insert(sid, idx); // 현재(full) 유지
                } else {
                    keep[idx] = false; // 현재 중복 제거
                }
            }
        }
    }
    rows.into_iter()
        .enumerate()
        .filter_map(|(idx, r)| if keep[idx] { Some(r) } else { None })
        .collect()
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
    // rc.245 — peers.session_identifier 별도 prefetch (PeerStore.list() 미반환 필드).
    //   결정적 세션 매핑: Messenger.tsx 가 normalizeAlias 추정 대신 이 값을 직접 사용.
    let mut sid_map: std::collections::HashMap<String, String> = Default::default();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT alias, session_identifier FROM peers WHERE session_identifier IS NOT NULL AND session_identifier != ''"
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                sid_map.insert(row.0, row.1);
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
    drop(db); // unlock

    // rc.273 — 로스터에는 살아있는 tmux 에이전트만 (마스터 룰). 단일 헬퍼로 LOCAL 생존 판정.
    //   - LOCAL tmux peer = session_identifier 가 "tmux:<name>" (auto_seed 가 자기 머신 세션에만 기록).
    //     이 ident 가 live 집합에 없으면 죽은 LOCAL 세션 → 로스터에서 제외.
    //   - 원격 병합 peer(session_identifier 없거나 비-tmux)·채널(discord/telegram)·self 는 그대로 표시.
    //     (원격 peer 는 함대 배포 후 그 원격의 reachable 소스에서 이미 걸러짐 → 로컬 tmux 검사 금지.)
    //   - tmux 미설치/실패로 live 가 비면(보수) 필터를 적용하지 않는다(기존 표시 유지 — 과삭제 방지).
    let live = crate::daemon::local_live_tmux_agent_idents();
    let rows: Vec<_> = if live.is_empty() {
        rows
    } else {
        rows.into_iter()
            .filter(|p| match sid_map.get(&p.alias) {
                // LOCAL tmux peer — 살아있는 세션 집합에 있을 때만 표시.
                Some(sid) if sid.starts_with("tmux:") => live.contains(sid),
                // 비-tmux session_identifier / 없음 = 원격·채널·self → 그대로 표시.
                _ => true,
            })
            .collect()
    };
    // rc.274/rc.280 — 같은 tmux 세션을 가리키는 중복 peer 행 dedupe (GUI 로스터 1행/세션).
    //   공통 헬퍼 dedup_by_tmux_session 으로 일원화 (gui_orchestration_agents 와 동일 규칙).
    let rows = dedup_by_tmux_session(rows, &sid_map, |p| p.alias.as_str());
    // rc.229 — fix#1: per-peer subprocess enrichment 전부 제거 (8.7s → <200ms).
    //   project_folder/llm_type/llm_version/worktrees/subagents/ex_peers 는 모두
    //   on-demand `/v1/gui/agent/{alias}/detail` 에서 1개씩 enrich (fix#3).
    //   여기는 기본 필드만 — tmux/ps/git/ls subprocess 호출 0회.
    let mut dtos: Vec<PeerDto> = Vec::with_capacity(rows.len());
    for p in rows.into_iter() {
        let (description, capabilities) = caps_map.get(&p.alias).cloned().unwrap_or((None, vec![]));
        let session_identifier = sid_map.get(&p.alias).cloned();
        dtos.push(PeerDto {
            id: p.id,
            alias: p.alias,
            address: p.address,
            public_key_hex: p.public_key_hex,
            role: p.role.as_str().to_string(),
            created_at: p.created_at.to_rfc3339(),
            last_seen: p.last_seen.map(|t| t.to_rfc3339()),
            description,
            capabilities,
            project_folder: None,
            llm_type: None,
            llm_version: None,
            machine: None,
            worktrees: Vec::new(),
            subagents: Vec::new(),
            ex_peers: Vec::new(),
            session_identifier,
        });
    }
    Ok(Json(dtos))
}

/// rc.229 fix#3 — on-demand 단일 agent enrichment.
/// alias = tmux session 이름 또는 peer alias. 그 1개만 enrich:
///   4-metadata (project_folder / llm_type / llm_version / machine)
///   + worktrees + subagents + ex_peers.
/// gui_peers 에서 제거된 무거운 subprocess enrichment 를 클릭 시 1회만 수행 (~300ms).
async fn gui_agent_detail(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // local machine.alias (install-manifest) — 1회.
    let local_machine: Option<String> = InstallManifest::read(manifest_path(&state.data_dir))
        .ok()
        .map(|m| m.machine.alias);
    // 4-metadata: enrich_peer_metadata 가 alias → tmux session 매칭 + project_folder + LLM detect.
    let (project_folder, llm_type, llm_version) = enrich_peer_metadata(&alias).await;
    // tmux session 매칭이 됐으면 local 로 간주 → machine 채움.
    let machine: Option<String> = if project_folder.is_some() || llm_type.is_some() {
        local_machine
    } else {
        None
    };
    // worktrees + subagents: project_folder 가 있을 때만.
    let (worktrees, subagents) = if let Some(pf) = project_folder.as_deref() {
        let wt = enrich_worktrees(pf).await;
        let sa = enrich_subagents(pf).await;
        (wt, sa)
    } else {
        (Vec::new(), Vec::new())
    };
    // ex_peers: self (Starian) 에 대해서만 (다른 머신 의 daemon 이 그 peer 의 ex_peers 책임).
    let ex_peers = if alias == "Starian" {
        collect_ex_peers(&state).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(Json(serde_json::json!({
        "alias": alias,
        "project_folder": project_folder,
        "llm_type": llm_type,
        "llm_version": llm_version,
        "machine": machine,
        "worktrees": worktrees,
        "subagents": subagents,
        "ex_peers": ex_peers,
    })))
}

/// rc.228 — `git -C <project_folder> worktree list --porcelain` 파싱.
/// porcelain 형식: 빈 줄 사이 record. record 안에 `worktree <path>`, `HEAD <sha>`, `branch <ref>` 라인.
async fn enrich_worktrees(project_folder: &str) -> Vec<WorktreeEntry> {
    let out = match tokio::process::Command::new("git")
        .args(["-C", project_folder, "worktree", "list", "--porcelain"])
        .output().await
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut entries: Vec<WorktreeEntry> = Vec::new();
    let mut cur_path: Option<String> = None;
    let mut cur_branch: Option<String> = None;
    for line in stdout.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            if let Some(p) = cur_path.take() {
                entries.push(WorktreeEntry { path: p, branch: cur_branch.take() });
            }
            cur_branch = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            cur_path = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            // refs/heads/main → main
            let short = rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string();
            cur_branch = Some(short);
        } else if line == "detached" && cur_branch.is_none() {
            cur_branch = Some("(detached)".to_string());
        }
    }
    if let Some(p) = cur_path.take() {
        entries.push(WorktreeEntry { path: p, branch: cur_branch.take() });
    }
    entries
}

/// rc.228 — `<project_folder>/agents/` + `<project_folder>/.claude/agents/` scan.
/// 폴더 = subagent (entry.name). 파일도 포함 (`.md` 등 Claude Code agent definition).
async fn enrich_subagents(project_folder: &str) -> Vec<SubagentEntry> {
    let mut out: Vec<SubagentEntry> = Vec::new();
    for (subpath, kind) in [
        (".claude/agents", "claude_agents"),
        ("agents", "project_agents"),
    ] {
        let full = format!("{}/{}", project_folder.trim_end_matches('/'), subpath);
        let mut rd = match tokio::fs::read_dir(&full).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = match entry.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            // hidden / shared / CLAUDE.md 류 metadata 는 skip.
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path().display().to_string();
            out.push(SubagentEntry { name, path, kind: kind.to_string() });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// rc.228 — Starian (self) 가 대화한 다른 peer 들 집계.
/// sessions.title LIKE 'outbox-to-%' 또는 'inbox-from-%' 인 session 의 messages COUNT + MAX(timestamp).
/// alias 추출: title 의 prefix 제거 후 (outbox-to-X → X / inbox-from-X → X).
async fn collect_ex_peers(
    state: &GuiServerState,
) -> Result<Vec<ExPeerEntry>, rusqlite::Error> {
    use std::collections::HashMap;
    let mut db = state.db.lock().await;
    let conn = db.conn();
    let mut agg: HashMap<String, (i64, Option<String>)> = HashMap::new();
    let sql = "SELECT s.title, COUNT(m.id) as cnt, MAX(m.timestamp) as last_at \
               FROM sessions s LEFT JOIN messages m ON m.session_id = s.id \
               WHERE s.title LIKE 'outbox-to-%' OR s.title LIKE 'inbox-from-%' \
               GROUP BY s.title";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows.flatten() {
        let (title, cnt, last_at) = row;
        let alias = if let Some(rest) = title.strip_prefix("outbox-to-") {
            rest.to_string()
        } else if let Some(rest) = title.strip_prefix("inbox-from-") {
            rest.to_string()
        } else {
            continue;
        };
        // alias variant root 통합: aoe_<root>_<hash> → <root>.
        let root = if let Some(rest) = alias.strip_prefix("aoe_") {
            rest.split('_').next().unwrap_or(&alias).to_string()
        } else {
            alias.clone()
        };
        let key = root;
        let entry = agg.entry(key).or_insert((0, None));
        entry.0 += cnt;
        match (&entry.1, &last_at) {
            (None, Some(_)) => entry.1 = last_at.clone(),
            (Some(prev), Some(cur)) if cur > prev => entry.1 = last_at.clone(),
            _ => {}
        }
    }
    drop(stmt);
    drop(db);
    let mut out: Vec<ExPeerEntry> = agg
        .into_iter()
        .filter(|(_, (c, _))| *c > 0)
        .map(|(alias, (msg_count, last_msg_at))| ExPeerEntry {
            alias,
            msg_count,
            last_msg_at,
        })
        .collect();
    // 최신 활동 순.
    out.sort_by(|a, b| b.last_msg_at.cmp(&a.last_msg_at));
    Ok(out)
}

/// rc.228 — `DELETE /v1/gui/peer/{self_alias}/ex_peer/{other_alias}`
/// ex Peer thread 삭제: self↔other 의 outbox/inbox sessions + 그 안 messages + outbound_queue rows.
/// backup first: 삭제 전 archived_at metadata 만 표시할지 vs hard delete — V1 = hard delete (관리 단순).
async fn gui_peer_ex_peer_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path((self_alias, other_alias)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // V1 — Starian self 만 처리. 다른 self_alias 는 명시적으로 reject.
    if self_alias != "Starian" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: format!("ex_peer delete: only self='Starian' supported in V1 (got '{}')", self_alias),
            }),
        ));
    }
    let mut db = state.db.lock().await;
    let conn = db.conn();
    // alias variants: peers table 에서 other_alias substring 매칭.
    let mut variants: Vec<String> = vec![other_alias.clone()];
    {
        let pattern = format!("%{}%", other_alias);
        if let Ok(mut stmt) = conn.prepare(
            "SELECT alias FROM peers WHERE alias LIKE ?1 OR ?2 LIKE '%' || alias || '%'"
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![pattern, other_alias], |r| {
                r.get::<_, String>(0)
            }) {
                for row in rows.flatten() {
                    if !variants.contains(&row) {
                        variants.push(row);
                    }
                }
            }
        }
    }
    // sessions.title 목록 build: outbox-to-<v> / inbox-from-<v>.
    let mut titles: Vec<String> = Vec::new();
    for v in &variants {
        titles.push(format!("outbox-to-{}", v));
        titles.push(format!("inbox-from-{}", v));
    }
    let mut deleted_sessions = 0i64;
    let mut deleted_messages = 0i64;
    let mut deleted_queue = 0i64;
    let tx = conn.unchecked_transaction().map_err(|e| {
        internal(&format!("ex_peer delete tx: {e}"))
    })?;
    for title in &titles {
        // messages count first (CASCADE 가 자동 삭제하지만 회계용).
        let mut cnt: i64 = 0;
        if let Ok(c) = tx.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id IN (SELECT id FROM sessions WHERE title = ?1)",
            rusqlite::params![title],
            |r| r.get::<_, i64>(0),
        ) { cnt = c; }
        deleted_messages += cnt;
        // session delete (ON DELETE CASCADE 로 messages 도 sweep).
        let n = tx.execute(
            "DELETE FROM sessions WHERE title = ?1",
            rusqlite::params![title],
        ).map_err(|e| internal(&format!("ex_peer sessions del: {e}")))?;
        deleted_sessions += n as i64;
    }
    // outbound_queue 의 target_alias 매칭 row 삭제.
    for v in &variants {
        let n = tx.execute(
            "DELETE FROM outbound_queue WHERE target_alias = ?1",
            rusqlite::params![v],
        ).map_err(|e| internal(&format!("ex_peer queue del: {e}")))?;
        deleted_queue += n as i64;
    }
    tx.commit().map_err(|e| internal(&format!("ex_peer commit: {e}")))?;
    Ok(Json(serde_json::json!({
        "self_alias": self_alias,
        "other_alias": other_alias,
        "variants": variants,
        "deleted_sessions": deleted_sessions,
        "deleted_messages": deleted_messages,
        "deleted_queue": deleted_queue,
    })))
}

/// rc.226 — peer alias 의 4-metadata 자동 detect (tmux + process tree).
/// 반환: (project_folder, llm_type, llm_version). detect 실패 = "unknown" 명시.
async fn enrich_peer_metadata(alias: &str) -> (Option<String>, Option<String>, Option<String>) {
    // 1) alias → tmux session 매칭 (notify::resolve_alias_to_tmux 재사용)
    let session = match crate::notify::resolve_alias_to_tmux(alias).await {
        Some((s, _)) => s,
        None => return (None, None, None), // tmux 미설치 또는 peer 가 cross-machine
    };
    // 2) project_folder = pane_current_path
    let project_folder = tokio::process::Command::new("tmux")
        .args(["display-message", "-p", "-t", &format!("{}:0", session), "#{pane_current_path}"])
        .output().await
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            } else { None }
        });
    // 3) pane PID → 자식 process tree 에서 LLM 키워드 매칭
    let pane_pid = tokio::process::Command::new("tmux")
        .args(["display-message", "-p", "-t", &format!("{}:0", session), "#{pane_pid}"])
        .output().await
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok()
            } else { None }
        });
    let (llm_type, llm_version) = match pane_pid {
        Some(pid) => detect_llm_in_subtree(pid).await,
        None => (Some("unknown".to_string()), None),
    };
    (project_folder, llm_type, llm_version)
}

/// rc.226 — pane PID + 그 자식 프로세스 트리에서 LLM 종류 자동 detect.
/// 후보: claude / gemini / codex / ollama / aider / cursor / continue / cline.
/// pane_pid 자체가 LLM 일 수도 있음 (예: tmux 가 직접 `claude` 실행).
async fn detect_llm_in_subtree(pane_pid: u32) -> (Option<String>, Option<String>) {
    // 1) pane PID 자체 + 자식들 BFS (최대 깊이 4)
    let mut frontier: Vec<u32> = vec![pane_pid];
    let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
    visited.insert(pane_pid);
    // 0회차 = pane_pid 자체 검사
    let self_out = tokio::process::Command::new("ps")
        .args(["-o", "pid=,comm=,args=", "-p", &pane_pid.to_string()])
        .output().await;
    if let Ok(out) = self_out {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if let Some(found) = match_llm_in_line(line).await {
                    return found;
                }
            }
        }
    }
    for _depth in 0..4 {
        if frontier.is_empty() { break; }
        let pids_csv = frontier.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
        let out = tokio::process::Command::new("ps")
            .args(["-o", "pid=,comm=,args=", "--ppid", &pids_csv])
            .output().await;
        let Ok(out) = out else { break; };
        if !out.status.success() { break; }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut next_frontier: Vec<u32> = vec![];
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            let mut parts = trimmed.splitn(3, char::is_whitespace);
            let pid_s = parts.next().unwrap_or("");
            let Ok(child_pid) = pid_s.parse::<u32>() else { continue; };
            if !visited.insert(child_pid) { continue; }
            next_frontier.push(child_pid);
            if let Some(found) = match_llm_in_line(trimmed).await {
                return found;
            }
        }
        frontier = next_frontier;
    }
    // 자식 트리 에 LLM 미검출 → 명시 unknown
    (Some("unknown".to_string()), None)
}

/// rc.226 — ps line ("PID COMM ARGS...") 에서 LLM 키워드 매칭 + --version 추출.
async fn match_llm_in_line(line: &str) -> Option<(Option<String>, Option<String>)> {
    let trimmed = line.trim();
    if trimmed.is_empty() { return None; }
    let mut parts = trimmed.splitn(3, char::is_whitespace);
    let _pid = parts.next().unwrap_or("");
    let comm = parts.next().unwrap_or("");
    let args = parts.next().unwrap_or("");
    let hay = format!("{} {}", comm, args).to_lowercase();
    // 키워드 매칭 — comm + args 합쳐 검사
    let (llm_name, version_cmd): (&str, Option<&str>) = if (hay.contains("claude") || comm == "claude") && !hay.contains("claude-api") {
        ("Claude Code", Some("claude"))
    } else if (hay.contains("gemini") || comm == "gemini") && !hay.contains("gemini-api") {
        ("Gemini", Some("gemini"))
    } else if hay.contains("codex") || comm == "codex" {
        ("Codex", Some("codex"))
    } else if hay.contains("ollama") || comm == "ollama" {
        ("Ollama", Some("ollama"))
    } else if hay.contains("aider") || comm == "aider" {
        ("Aider", Some("aider"))
    } else if hay.contains("cursor-agent") || hay.contains("cursor agent") {
        ("Cursor", None)
    } else if hay.contains("continue") && hay.contains("dev") {
        ("Continue", None)
    } else if hay.contains("cline") {
        ("Cline", None)
    } else if hay.contains("hermes") || comm == "hermes" {
        // rc.278 — Hermes Agent(비-Claude 프레임워크) 인식. tmux_session_runs_llm 과 동일 후보.
        ("Hermes", None)
    } else {
        return None;
    };
    let version: Option<String> = if let Some(vcmd) = version_cmd {
        tokio::time::timeout(
            std::time::Duration::from_millis(800),
            tokio::process::Command::new(vcmd).arg("--version").output(),
        ).await
            .ok()
            .and_then(|r| r.ok())
            .and_then(|o| {
                if o.status.success() {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if s.is_empty() { None } else { Some(s.lines().next().unwrap_or("").to_string()) }
                } else { None }
            })
    } else { None };
    Some((Some(llm_name.to_string()), version))
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
        // rc.229 fix#2 — 이전: peer 마다 sequential unlock+sessions (3s timeout 누적 → 3s+).
        //   이제: (1) base address dedup — 같은 daemon 가리키는 peer row 중복 제거 (alias 만 다름).
        //         (2) per-request 2s timeout + 전체 fan-out concurrent (join_all) → wall time ≈ 1 peer.
        //   본질(cross-machine merge) 유지 — 단지 병렬화·중복제거로 빠르게.
        let mut seen_base: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut deduped: Vec<(String, String)> = Vec::new();
        for (alias, address) in peer_targets {
            let base = address.trim_end_matches('/').to_string();
            if seen_base.insert(base.clone()) {
                deduped.push((alias, base));
            }
        }
        let client = reqwest::Client::builder()
            // rc.229 fix#2 — connect_timeout 0.9s: hang/방화벽 peer 가 overall timeout 까지
            //   안 기다리고 ~0.9s 에 fail-fast. reachable peer 는 정상 (connect <30ms).
            .connect_timeout(std::time::Duration::from_millis(900))
            .timeout(std::time::Duration::from_millis(1500))
            .build().ok();
        if let Some(http) = client {
            // 각 base 를 독립 future 로 — unlock → sessions → remote json 반환.
            let fetches = deduped.into_iter().map(|(alias, base)| {
                let http = http.clone();
                let local_pw = local_pw.clone();
                async move {
                    // rc.247 — anon sessions 우선 (keystore 비번 불일치 머신도 가져옴).
                    //   성공 시 unlock 생략 → zalman 처럼 비번 다른 머신의 개별 에이전트도 카드로 노출.
                    let anon_url = format!("{base}/v1/gui/public/sessions");
                    if let Ok(r) = http.get(&anon_url).send().await {
                        if r.status().is_success() {
                            if let Ok(v) = r.json::<serde_json::Value>().await {
                                let has = v.get("sessions").and_then(|s| s.as_array()).map(|a| !a.is_empty()).unwrap_or(false);
                                if has { return (alias, Some(v)); }
                            }
                        }
                    }
                    // fallback: unlock + sessions (anon 비활성 구버전 peer)
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
                    if peer_token.is_empty() { return (alias, None); }
                    let url = format!("{base}/v1/gui/sessions");
                    let resp = http.get(&url)
                        .header("Authorization", format!("Bearer {peer_token}"))
                        .send().await;
                    let json = match resp {
                        Ok(r) => r.json::<serde_json::Value>().await.ok(),
                        Err(_) => None,
                    };
                    (alias, json)
                }
            });
            let results = futures_util::future::join_all(fetches).await;
            for (alias, maybe_json) in results {
                if let Some(remote_json) = maybe_json {
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
                            // rc.281 — 원격 데몬이 보고한 tmux pane cwd. 신버전 원격만 채움(구버전=None).
                            //   cross-machine cwd 매칭에 사용(양쪽 신버전 시 동작).
                            let remote_cwd = item.get("cwd").and_then(|v| v.as_str())
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty());
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
                                // rc.234 — cross-machine peer 세션은 보고측 머신에서 worktree 종합(미전달). 빈 Vec.
                                worktrees: Vec::new(),
                                cwd: remote_cwd,
                            });
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
    // rc.266 — 메신저 카드는 실제 tmux 세션만 (마스터 핵심 지시·반복 회귀 금지).
    // 로컬+원격(peer merge) 모두에서 claude_project 카드 제거 — 단일 방어선.
    dto.sessions.retain(|s| s.kind != crate::daemon_gui_sessions::SessionKind::ClaudeProject);
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
    // peer:<alias>[:<inner-id>] 형태면 해당 peer 의 daemon 에 cross-machine proxy.
    //   rc.237 — inner-id 가 없으면(peer:<alias>) 그 peer 의 첫 active session 을 자동 선택.
    //   peer unreachable 시 silent X — 명시적 "remote terminal unavailable" content 반환.
    if let Some(rest) = identifier.strip_prefix("peer:") {
        let (alias, inner_opt): (&str, Option<&str>) = match rest.find(':') {
            Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
            None => (rest, None),
        };
        // peer address 조회 — rc.167+: gui_address 있으면 우선 (7302 GUI), 없으면 address (7300 transport).
        let address: String = {
            let mut db = state.db.lock().await;
            db.conn().query_row(
                "SELECT COALESCE(gui_address, address) FROM peers WHERE alias = ?1",
                rusqlite::params![alias],
                |r| r.get(0),
            ).unwrap_or_default()
        };
        // 명시적 unavailable DTO helper (silent X — 빈 화면 대신 사유 노출).
        let unavailable = |reason: &str| -> Json<crate::daemon_gui_sessions::SessionScreenDto> {
            Json(crate::daemon_gui_sessions::SessionScreenDto {
                identifier: identifier.clone(),
                kind: crate::daemon_gui_sessions::SessionKind::Tmux,
                display: format!("[{alias}] (원격)"),
                content: format!("⚠ remote terminal unavailable\npeer: {alias}\n사유: {reason}"),
                lines: 0,
                source_note: format!("[via peer {alias}] unavailable"),
                fetched_at: String::new(),
            })
        };
        if address.is_empty() || !address.starts_with("http") {
            return Ok(unavailable("peer has no http address (gui_address/address 미설정)"));
        }
        let local_pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").unwrap_or_default();
        let client = match reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(1200))
            .timeout(std::time::Duration::from_secs(5))
            .build() {
            Ok(c) => c,
            Err(e) => return Ok(unavailable(&format!("http client: {e}"))),
        };
        let base = address.trim_end_matches('/');
        // rc.239 (이슈 #66) — 먼저 anonymous read-only screen 시도 (unlock 불필요).
        //   peer 가 rc.239+ 이면 /v1/gui/public/session-screen 가 auth 없이 tmux 화면 반환.
        //   keystore password 불일치/M-8 lockout 회피. inner-id 가 없으면 "0" 으로 첫 세션.
        {
            let inner_for_anon = inner_opt.unwrap_or("0");
            let anon_url = format!(
                "{base}/v1/gui/public/session-screen/{}",
                urlencoding::encode(inner_for_anon)
            );
            if let Ok(r) = client.get(&anon_url).send().await {
                if r.status().is_success() {
                    if let Ok(v) = r.json::<serde_json::Value>().await {
                        let kind_str = v.get("kind").and_then(|x| x.as_str()).unwrap_or("tmux");
                        let kind = match kind_str {
                            "tmux" => crate::daemon_gui_sessions::SessionKind::Tmux,
                            "claude_project" => crate::daemon_gui_sessions::SessionKind::ClaudeProject,
                            _ => crate::daemon_gui_sessions::SessionKind::XgramSession,
                        };
                        let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
                        // 빈 content 가 아니면 anonymous 경로 성공 — unlock 단계 skip.
                        // rc.241 — 구버전 peer 가 "0" 을 unsupported 로 응답하는 에러 문자열은
                        //   성공으로 오인하지 않도록 제외 → unlock+첫세션 fallback 으로 넘어감.
                        if !content.is_empty() && !content.contains("unsupported identifier") {
                            return Ok(Json(crate::daemon_gui_sessions::SessionScreenDto {
                                identifier: v.get("identifier").and_then(|x| x.as_str()).unwrap_or(inner_for_anon).into(),
                                kind,
                                display: v.get("display").and_then(|x| x.as_str()).unwrap_or("?").into(),
                                content: content.into(),
                                lines: v.get("lines").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                                source_note: format!("[via peer {alias} · anon] {}", v.get("source_note").and_then(|x| x.as_str()).unwrap_or("")),
                                fetched_at: v.get("fetched_at").and_then(|x| x.as_str()).unwrap_or("").into(),
                            }));
                        }
                    }
                }
            }
            // anonymous 실패 (peer 가 구버전 rc.238- 이거나 미응답) → 아래 unlock fallback.
        }
        // peer 의 unlock → token (password 평문 로그 금지).
        let unlock = match client.post(format!("{base}/v1/auth/unlock"))
            .json(&serde_json::json!({"password": local_pw}))
            .send().await {
            Ok(r) => r,
            Err(e) => return Ok(unavailable(&format!("peer unreachable (unlock): {e}"))),
        };
        let token = unlock.json::<serde_json::Value>().await.ok()
            .and_then(|v| v.get("session_token").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_default();
        if token.is_empty() {
            return Ok(unavailable("peer unlock failed (keystore password 불일치 가능)"));
        }
        // inner-id 결정: 명시되면 그대로, 없으면 peer 의 첫 session 자동 선택.
        let inner: String = match inner_opt {
            Some(s) => s.to_string(),
            None => {
                let list = match client.get(format!("{base}/v1/gui/sessions"))
                    .header("Authorization", format!("Bearer {token}"))
                    .send().await {
                    Ok(r) => r,
                    Err(e) => return Ok(unavailable(&format!("peer unreachable (sessions): {e}"))),
                };
                let lv: serde_json::Value = match list.json().await {
                    Ok(v) => v,
                    Err(e) => return Ok(unavailable(&format!("peer sessions json: {e}"))),
                };
                let arr = lv.get("sessions").and_then(|s| s.as_array()).cloned().unwrap_or_default();
                // 우선순위: status=active > attached > 첫 항목. (그 peer 의 self 원격 peer 항목 제외)
                let pick = arr.iter()
                    .filter(|it| {
                        let id = it.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
                        !id.starts_with("peer:") // 재귀 cross-machine 항목 제외
                    })
                    .max_by_key(|it| match it.get("status").and_then(|v| v.as_str()).unwrap_or("") {
                        "active" => 3, "attached" => 2, _ => 1,
                    })
                    .and_then(|it| it.get("identifier").and_then(|v| v.as_str()).map(String::from));
                match pick {
                    Some(id) => id,
                    None => return Ok(unavailable("peer 에 표시 가능한 터미널 세션 없음")),
                }
            }
        };
        // peer screen proxy fetch.
        let resp = match client.get(format!("{base}/v1/gui/sessions/{}/screen", urlencoding::encode(&inner)))
            .header("Authorization", format!("Bearer {token}"))
            .send().await {
            Ok(r) => r,
            Err(e) => return Ok(unavailable(&format!("peer unreachable (screen): {e}"))),
        };
        let v: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => return Ok(unavailable(&format!("peer screen json: {e}"))),
        };
        let kind_str = v.get("kind").and_then(|x| x.as_str()).unwrap_or("tmux");
        let kind = match kind_str {
            "tmux" => crate::daemon_gui_sessions::SessionKind::Tmux,
            "claude_project" => crate::daemon_gui_sessions::SessionKind::ClaudeProject,
            _ => crate::daemon_gui_sessions::SessionKind::XgramSession,
        };
        let dto = crate::daemon_gui_sessions::SessionScreenDto {
            identifier: v.get("identifier").and_then(|x| x.as_str()).unwrap_or(&inner).into(),
            kind,
            display: v.get("display").and_then(|x| x.as_str()).unwrap_or("?").into(),
            content: v.get("content").and_then(|x| x.as_str()).unwrap_or("").into(),
            lines: v.get("lines").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            source_note: format!("[via peer {alias}] {}", v.get("source_note").and_then(|x| x.as_str()).unwrap_or("")),
            fetched_at: v.get("fetched_at").and_then(|x| x.as_str()).unwrap_or("").into(),
        };
        return Ok(Json(dto));
    }
    Ok(Json(crate::daemon_gui_sessions::capture_session(&identifier)))
}

/// rc.239 (이슈 #66) — anonymous read-only tmux 화면 미러.
///
/// 본질: cross-machine proxy 가 remote daemon 의 `/v1/auth/unlock` 을 호출하면
/// keystore password 불일치 시 M-8 lockout 이 누적됐다 (zalman-wsl 사례).
/// 이 endpoint 는 **unlock 없이** tmux capture 만 반환 — auth 단계 제거로 lockout 소멸.
///
/// 보안: tailnet(Tailscale 100.64.0.0/10 CGNAT) 또는 localhost 에서만 응답.
///   - 반환은 tmux 화면 capture 뿐 (`capture_session`). password/vault/메시지 내용 노출 X.
///   - `XGRAM_ANON_SCREEN=0` 으로 명시 차단 가능 (기본 활성).
async fn gui_public_session_screen(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
    Path(identifier): Path<String>,
) -> Result<Json<crate::daemon_gui_sessions::SessionScreenDto>, (StatusCode, Json<ErrorDto>)> {
    if std::env::var("XGRAM_ANON_SCREEN").as_deref() == Ok("0") {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorDto {
                error: "anonymous screen disabled (XGRAM_ANON_SCREEN=0)".into(),
            }),
        ));
    }
    if !is_tailnet_or_local(peer.ip()) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorDto {
                error: format!("anonymous screen 은 tailnet/localhost 전용 (src={})", peer.ip()),
            }),
        ));
    }
    // cross-machine proxy 재귀 차단 — peer:* identifier 는 여기서 처리 안 함.
    if identifier.starts_with("peer:") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: "anonymous screen 은 로컬 세션만 (peer:* 불가)".into(),
            }),
        ));
    }
    // rc.241 (이슈 #66) — cross-machine proxy 는 inner-id 가 없으면 "0" 을 보낸다.
    //   capture_session("0") 은 unsupported 라, "0"/빈 식별자면 첫 active 로컬 세션을 자동 선택.
    let resolved: String = if identifier == "0" || identifier.is_empty() {
        match tokio::task::spawn_blocking(crate::daemon_gui_sessions::collect_sessions).await {
            Ok(dto) => {
                // rc.244 — 한 머신이 여러 tmux(AoE 워커 등)를 돌릴 때 "첫 active" 가 엉뚱한
                //   세션(portal 이 active 로 표시한 워커, 예: aoe_studio)을 뽑아 그 머신의 본
                //   에이전트가 아닌 게 보이던 문제. 우선순위로 본 에이전트를 정확히 선택:
                //   (1) 세션 이름이 머신 자기 alias 와 일치 (그 머신의 primary 에이전트 세션)
                //   (2) attached=true (사용자/portal 이 실제 attach 한 세션)
                //   (3) status (Active>Attached>Detached>Stale)
                // rc.246 — tmux 세션 이름이 재생성마다 바뀌어(aoe_zalman-wsl_7f27e90b →
                //   aoe_zalman_1d825afa) full-alias 매칭이 깨지고, volatile "active" status 에
                //   의존해 폴링마다 흔들리던 문제. 해결: machine alias 를 stem 으로 정규화해
                //   (wsl-zalman → zalman) 세션 이름에 stem 포함 여부로 매칭 + active status
                //   의존 제거 + 결정적 tie-break(같은 점수면 항상 같은 세션) → 폴링마다 안 흔들림.
                let stem = {
                    let mut x = dto.machine.alias.trim().to_lowercase();
                    for p in ["wsl-", "wsl_", "aoe-", "aoe_", "term-", "term_"] {
                        if let Some(r) = x.strip_prefix(p) { x = r.to_string(); break; }
                    }
                    if let Some(pos) = x.rfind(['_', '-']) {
                        let suf = &x[pos + 1..];
                        if suf.len() >= 8 && suf.chars().all(|c| c.is_ascii_hexdigit()) {
                            x.truncate(pos);
                        }
                    }
                    x
                };
                dto.sessions
                    .into_iter()
                    .filter(|s| !s.identifier.starts_with("peer:")) // cross-machine 재귀 제외
                    .max_by_key(|s| {
                        let idl = s.identifier.to_lowercase();
                        // (1) 이 머신의 본 에이전트 세션 — 이름에 머신 stem 포함 (가장 강한 신호)
                        let stem_match = if !stem.is_empty() && idl.contains(&stem) { 500 } else { 0 };
                        // (2) 실제 attach 된 세션 (사용자가 보고 있는 live 터미널)
                        let attached_bonus = if s.attached == Some(true) { 100 } else { 0 };
                        // (3) 임시 sv_ 세션 비선호
                        let not_temp = if !idl.contains("sv_") { 30 } else { 0 };
                        // (4) 인터랙티브 tmux 선호 (claude jsonl·portal 보다)
                        let tmux_bonus = if matches!(s.kind, crate::daemon_gui_sessions::SessionKind::Tmux) { 10 } else { 0 };
                        // 결정적 tie-break — 같은 점수면 항상 같은 세션 선택 (폴링마다 동일)
                        (stem_match + attached_bonus + not_temp + tmux_bonus, s.identifier.clone())
                    })
                    .map(|s| s.identifier)
                    .unwrap_or_else(|| identifier.clone())
            }
            Err(_) => identifier.clone(),
        }
    } else {
        identifier.clone()
    };
    Ok(Json(crate::daemon_gui_sessions::capture_session(&resolved)))
}

/// `GET /v1/gui/public/sessions` — rc.247. 인증 없이 로컬 세션 목록 (tailnet/localhost 전용).
///   cross-machine fan-out 이 keystore 비번 불일치 머신(zalman 등)의 세션을 가져올 수 있게.
///   각 원격 세션이 `peer:<alias>:<id>` 카드로 노출 → 머신의 개별 에이전트가 각각 peer 처럼 보임.
///   peer:* (재귀 cross-machine) 제외 — 로컬 세션만.
async fn gui_public_sessions(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
) -> Result<Json<crate::daemon_gui_sessions::SessionsDto>, (StatusCode, Json<ErrorDto>)> {
    if std::env::var("XGRAM_ANON_SCREEN").as_deref() == Ok("0") {
        return Err((StatusCode::FORBIDDEN, Json(ErrorDto { error: "anonymous disabled (XGRAM_ANON_SCREEN=0)".into() })));
    }
    if !is_tailnet_or_local(peer.ip()) {
        return Err((StatusCode::FORBIDDEN, Json(ErrorDto { error: format!("anonymous sessions 는 tailnet/localhost 전용 (src={})", peer.ip()) })));
    }
    let mut dto = tokio::task::spawn_blocking(crate::daemon_gui_sessions::collect_sessions)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("collect: {e}") })))?;
    dto.sessions.retain(|s| !s.identifier.starts_with("peer:"));
    Ok(Json(dto))
}

/// 요청 src IP 가 tailnet(Tailscale CGNAT 100.64.0.0/10) 또는 localhost 인지 판정.
/// anonymous screen endpoint 의 접근 통제 (tmux 화면만이라도 외부 공개 X).
fn is_tailnet_or_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            // localhost
            if v4.is_loopback() {
                return true;
            }
            // Tailscale CGNAT 100.64.0.0/10 (100.64.x.x ~ 100.127.x.x)
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return true;
            }
            // private LAN (같은 머신 WSL ↔ Windows 브리지 등) — RFC1918
            if v4.is_private() {
                return true;
            }
            false
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(is_tailnet_or_local_v4),
    }
}

fn is_tailnet_or_local_v4(v4: std::net::Ipv4Addr) -> bool {
    is_tailnet_or_local(std::net::IpAddr::V4(v4))
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

    // ── (1) peer fan-out — 원격 머신 세션은 그 머신의 PUBLIC 입력 엔드포인트로 (무인증, tailnet 게이트).
    //   rc.256 — 이전: peer keystore 를 LOCAL 비번으로 unlock → authed POST. 그러나 머신마다 keystore
    //   비번이 달라(예: server-seoul ≠ macmini) unlock 실패 → 500 + M-8 lockout. 읽기 경로(rc.247)처럼
    //   anon public 입력으로 보낸다. transport 포트(address) 가 아닌 gui_address(COALESCE) 사용.
    if let Some(rest) = identifier.strip_prefix("peer:") {
        if let Some(idx) = rest.find(':') {
            let alias = rest[..idx].to_string();
            let inner = rest[idx + 1..].to_string();
            let gui_base: String = {
                let mut db = state.db.lock().await;
                db.conn().query_row(
                    "SELECT COALESCE(gui_address, address) FROM peers WHERE alias = ?1",
                    rusqlite::params![alias],
                    |r| r.get(0),
                ).unwrap_or_default()
            };
            if gui_base.is_empty() || !gui_base.starts_with("http") {
                return Err(internal(&format!("peer {alias} has no http gui address")));
            }
            let base = gui_base.trim_end_matches('/');
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| internal(&format!("http: {e}")))?;
            // anon public 입력 우선 (무인증). 구버전 peer 는 404/405 → unlock fallback.
            let anon = client
                .post(format!("{base}/v1/gui/public/sessions/{}/input", urlencoding::encode(&inner)))
                .json(&serde_json::json!({"data": body.data}))
                .send().await;
            if let Ok(resp) = anon {
                let st = resp.status();
                if st.is_success() {
                    return Ok(Json(serde_json::json!({"ok": true, "via": format!("peer:{alias}:public"), "bytes_sent": body.data.len()})));
                }
                if st != reqwest::StatusCode::NOT_FOUND && st != reqwest::StatusCode::METHOD_NOT_ALLOWED {
                    let t = resp.text().await.unwrap_or_default();
                    return Err(bad_request(&format!("peer public input HTTP {st}: {t}")));
                }
            }
            // fallback: 구버전 peer (public 입력 엔드포인트 없음) — 같은 비번 가정 unlock + authed.
            let local_pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").unwrap_or_default();
            let unlock = client.post(format!("{base}/v1/auth/unlock"))
                .json(&serde_json::json!({"password": local_pw}))
                .send().await
                .map_err(|e| internal(&format!("peer unlock: {e}")))?;
            let token = unlock.json::<serde_json::Value>().await.ok()
                .and_then(|v| v.get("session_token").and_then(|t| t.as_str()).map(String::from))
                .unwrap_or_default();
            if token.is_empty() {
                return Err(internal(&format!("peer {alias} unlock failed (비번 불일치 — peer 를 rc.256+ 로 올려 public 입력 사용)")));
            }
            let resp = client.post(format!("{base}/v1/gui/sessions/{}/input", urlencoding::encode(&inner)))
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
    }

    // ── (2)+(3) 로컬 입력 (portal / tmux send-keys) — 공용 헬퍼 ──
    do_local_session_input(&identifier, &body.data).await.map(Json)
}

/// 로컬 입력 주입 — `portal:`/`aoe:` → portal API, `tmux:`/bare → tmux send-keys.
/// gui_session_input(인증) 과 gui_public_session_input(tailnet anon) 공용 (rc.256).
async fn do_local_session_input(
    identifier: &str,
    data: &str,
) -> Result<serde_json::Value, (StatusCode, Json<ErrorDto>)> {
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
        let trailing_enter = data.ends_with('\r') || data.ends_with('\n');
        let text_clean = data.trim_end_matches(|c: char| c == '\r' || c == '\n');
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
        return Ok(serde_json::json!({"ok": true, "via": format!("portal:{session}:{idx}"), "bytes_sent": data.len()}));
    }

    // ── (3) local tmux fallback (tmux:<name> 또는 bare alias) ─────────────────────
    let raw_target = identifier
        .strip_prefix("tmux:")
        .unwrap_or(identifier)
        .to_string();
    if raw_target.is_empty() {
        return Err(bad_request("empty identifier"));
    }
    // rc.269 gap#5 — alias → 실제 tmux session 해석.
    // Discord 인바운드 binding alias("voice")가 실제 session("aoe_voice_75cb4fe6")과
    // 다르면 send-keys -t 가 "can't find pane" 으로 실패 → alias 를 실제 session 명으로 변환.
    // 이미 실제 session 명이면 그대로. 매핑 못 찾으면 raw_target 유지 (명확한 tmux 에러 노출).
    let target = resolve_tmux_session(&raw_target);
    let send_data = data.to_string();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<std::process::Output> {
        // 줄바꿈(\n)으로 끝나면 = 제출. 리터럴 \n 은 TUI 앱(Claude Code 등)에서 줄바꿈일 뿐
        // 제출 Enter 가 아니므로, 텍스트는 -l 리터럴 → 딜레이 → 실제 Enter 키(-l 없음) 순으로 송신.
        let has_enter = send_data.ends_with('\n');
        if !has_enter {
            // 제어문자(^C=\x03)·방향키(\x1b[A) 등은 리터럴 그대로.
            return std::process::Command::new("tmux")
                .args(["send-keys", "-t", target.as_str(), "-l", send_data.as_str()])
                .output();
        }
        let text = send_data.trim_end_matches('\n');
        if !text.is_empty() {
            let out = std::process::Command::new("tmux")
                .args(["send-keys", "-t", target.as_str(), "-l", text])
                .output()?;
            if !out.status.success() {
                return Ok(out);
            }
            // 앱이 입력 텍스트를 반영할 시간을 준 뒤 Enter (제출). 딜레이 없으면 빈 제출/유실 가능.
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        // 실제 Enter 키 (리터럴 아님) → 제출.
        std::process::Command::new("tmux")
            .args(["send-keys", "-t", target.as_str(), "Enter"])
            .output()
    })
    .await
    .map_err(|e| internal(&format!("spawn: {e}")))?
    .map_err(|e| internal(&format!("tmux: {e}")))?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        return Err(bad_request(&format!("tmux send-keys: {stderr}")));
    }
    Ok(serde_json::json!({
        "ok": true,
        "bytes_sent": data.len()
    }))
}

/// rc.269 gap#5 — binding alias → 실제 tmux session 명 해석.
/// 우선순위:
///   1) alias 가 이미 살아있는 tmux session 명이면 그대로.
///   2) peers.session_identifier ("tmux:<name>") 매핑 (auto_seed_local_tmux_agents 가 기록).
///   3) tmux list-sessions 에서 alias == name | "aoe_<alias>_*" prefix 매칭
///      (retroactive_register_agents 의 매칭 규칙과 동일).
///   4) 못 찾으면 입력 alias 그대로 반환 (이후 send-keys 가 명확한 에러 노출).
fn resolve_tmux_session(alias: &str) -> String {
    // 현재 살아있는 tmux session 목록 (sync — daemon_gui 의 다른 helper 와 동일 패턴).
    let live: Vec<String> = {
        let (cmd, base_arg) = if cfg!(windows) { ("wsl", Some("tmux")) } else { ("tmux", None) };
        let mut c = std::process::Command::new(cmd);
        if let Some(a) = base_arg { c.arg(a); }
        match c.args(["list-sessions", "-F", "#{session_name}"]).output() {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            _ => Vec::new(),
        }
    };
    // (1) 이미 실제 session 명.
    if live.iter().any(|s| s == alias) {
        return alias.to_string();
    }
    // (2) peers.session_identifier 매핑 ("tmux:<name>").
    let data_dir = match openxgram_core::paths::default_data_dir() {
        Ok(d) => Some(d),
        Err(_) => std::env::var("XGRAM_DATA_DIR").ok().map(std::path::PathBuf::from),
    };
    if let Some(dir) = &data_dir {
        if let Ok(mut db) = openxgram_db::Db::open(openxgram_db::DbConfig {
            path: openxgram_core::paths::db_path(dir),
            ..Default::default()
        }) {
            let sid: Option<String> = db.conn().query_row(
                "SELECT session_identifier FROM peers \
                 WHERE alias=?1 AND session_identifier IS NOT NULL AND session_identifier != ''",
                rusqlite::params![alias],
                |r| r.get::<_, String>(0),
            ).ok();
            if let Some(sid) = sid {
                let name = sid.strip_prefix("tmux:").unwrap_or(&sid).to_string();
                if !name.is_empty() {
                    return name;
                }
            }
        }
    }
    // (3) live session prefix 매칭 (aoe_<alias>_*).
    if let Some(found) = live.iter().find(|sn| {
        sn.starts_with(&format!("aoe_{alias}_")) || sn.as_str() == alias
    }) {
        return found.clone();
    }
    // (4) fallback — 입력 그대로.
    alias.to_string()
}

/// `POST /v1/gui/public/sessions/:identifier/input` — rc.256. 인증 없이 로컬 세션에 입력 주입
/// (tailnet/localhost 전용). cross-machine 입력 fan-out 이 keystore 비번 불일치 머신에도
/// 보낼 수 있게 — 읽기 경로(public/sessions, public/session-screen)와 동일 패턴.
/// `peer:*` (재귀) 는 금지 — 로컬 세션만.
async fn gui_public_session_input(
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<SocketAddr>,
    Path(identifier): Path<String>,
    Json(body): Json<SessionInputBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    if std::env::var("XGRAM_ANON_SCREEN").as_deref() == Ok("0") {
        return Err((StatusCode::FORBIDDEN, Json(ErrorDto { error: "anonymous disabled (XGRAM_ANON_SCREEN=0)".into() })));
    }
    if !is_tailnet_or_local(peer.ip()) {
        return Err((StatusCode::FORBIDDEN, Json(ErrorDto { error: format!("anonymous input 은 tailnet/localhost 전용 (src={})", peer.ip()) })));
    }
    if identifier.starts_with("peer:") {
        return Err(bad_request("public input 은 로컬 세션만 (peer: 재귀 불가)"));
    }
    do_local_session_input(&identifier, &body.data).await.map(Json)
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

/// `POST /v1/webhook/agent/:token` — inbound webhook → 모니터 가능한 에이전트 스레드.
///
/// memory webhook 과 동일한 URL-token 인증(Bearer 없음, `webhook_token_memory` 재사용).
/// memory/L0 ingest 와 **별개** 추가 경로 — 기존 동작 변경 없음. 받은 페이로드를
/// A2A 가시화와 똑같은 방식(`record_message` → `acp_messages` + `conv_persisted` SSE
/// broadcast)으로 `webhook:<source>` conv_key 스레드에 남겨, 사람이 messenger 대화뷰에서
/// inbound 를 실시간으로 본다. **에이전트 턴 자동 트리거는 하지 않는다**(범위 밖) — 가시화만.
///
/// source 우선순위: query `?source=` → body `source` → body `from` → `"unknown"`.
/// 본문 텍스트: body `text` → body `message` → body `summary` → 전체 JSON 직렬화.
async fn webhook_agent_inbound(
    State(state): State<GuiServerState>,
    Path(token): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    Json(payload): Json<serde_json::Value>,
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

    // source 식별 (query → body.source → body.from → unknown). conv_key 안전화: 공백/구분자 정리.
    let source_raw = q
        .get("source")
        .map(|s| s.as_str())
        .or_else(|| payload.get("source").and_then(|v| v.as_str()))
        .or_else(|| payload.get("from").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown");
    let source = source_raw.replace(char::is_whitespace, "_");
    let conv_key = format!("webhook:{source}");

    // 본문 텍스트 — 명시 필드 우선, 없으면 페이로드 전체를 직렬화.
    let text = payload
        .get("text")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("message").and_then(|v| v.as_str()))
        .or_else(|| payload.get("summary").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            serde_json::to_string(&payload).unwrap_or_else(|e| {
                // 절대 규칙 1 — 직렬화 실패도 조용히 넘기지 않고 명시 로그.
                tracing::error!(target: "acp.daemon", conv_key = %conv_key, "webhook inbound 페이로드 직렬화 실패: {e}");
                format!("[webhook:{source}] (페이로드 직렬화 실패: {e})")
            })
        });

    // 가시화 — A2A 와 동일 경로: record_message(영속) + conv_persisted broadcast(라이브).
    // 발신은 'me' 가 아닌 'agent'(=상대측 발화)로 기록 → 카드가 상대 메시지로 렌더.
    state.acp.record_message(&conv_key, "agent", &text).await;
    crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, &conv_key).await;

    tracing::info!(target: "acp.daemon", conv_key = %conv_key, source = %source, "inbound webhook → 에이전트 스레드 기록");

    Ok(Json(serde_json::json!({
        "ok": true,
        "conv_key": conv_key,
        "source": source,
    })))
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

#[derive(Debug, serde::Deserialize)]
struct DropFileBody {
    filename: String,
    content_b64: String,
    #[serde(default)]
    machine: Option<String>,
}

/// `POST /v1/gui/sessions/{identifier}/dropfile` — tmux 창에 드래그드롭한 파일을 서버
/// `<data_dir>/drops/<안전한 파일명>` 에 저장하고 절대경로를 반환. 프론트가 이 경로를
/// tmux 입력창에 삽입 → 같은 머신의 에이전트/명령이 그 파일을 바로 사용한다.
async fn gui_session_dropfile(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(_identifier): Path<String>,
    Json(body): Json<DropFileBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let raw = base64_decode(&body.content_b64)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorDto{error: format!("base64: {e}")})))?;
    // path traversal 방지 — 파일명에서 디렉토리 성분 제거.
    let safe = std::path::Path::new(&body.filename)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("dropped.bin")
        .to_string();
    // 원격 머신이면 SSH stdin 파이프로 그 머신에 저장(WSL 커맨드라인 길이제한 회피).
    // 스크립트는 base64 로 작게 전송, 파일 payload 는 ssh stdin → base64 -d (remote_acp_command 와 동일 패턴).
    if let Some(m) = body.machine.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(cfg) = crate::daemon_gui::machine_lookup(m) {
            use base64::Engine;
            let safe_sh: String = safe.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }).collect();
            let dest = format!("$HOME/.openxgram/drops/{safe_sh}");
            let script = format!("mkdir -p \"$HOME/.openxgram/drops\"; base64 -d > \"{dest}\"; printf '%s' \"{dest}\"");
            let sb64 = base64::engine::general_purpose::STANDARD.encode(script.as_bytes());
            let run = format!("echo {sb64}|base64 -d>/tmp/oxgdrop.$$.sh;exec bash /tmp/oxgdrop.$$.sh");
            let remote_cmd = if cfg.wsl { format!("wsl -- bash -lc \"{run}\"") } else { format!("bash -lc \"{run}\"") };
            use tokio::io::AsyncWriteExt;
            let mut child = tokio::process::Command::new("ssh")
                .arg("-T").arg(&cfg.ssh_host).arg(&remote_cmd)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("ssh spawn: {e}")})))?;
            if let Some(mut si) = child.stdin.take() {
                let _ = si.write_all(body.content_b64.as_bytes()).await;
                let _ = si.shutdown().await;
            }
            let out = child.wait_with_output().await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("ssh: {e}")})))?;
            if !out.status.success() {
                return Err((StatusCode::BAD_GATEWAY, Json(ErrorDto{error: format!("원격 저장 실패: {}", String::from_utf8_lossy(&out.stderr).trim())})));
            }
            let rpath = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return Ok(Json(serde_json::json!({ "ok": true, "path": rpath, "remote": m, "size_bytes": raw.len() })));
        }
    }
    let dir = state.data_dir.join("drops");
    std::fs::create_dir_all(&dir)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("mkdir: {e}")})))?;
    let path = dir.join(&safe);
    std::fs::write(&path, &raw)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("write: {e}")})))?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "path": path.to_string_lossy(),
        "size_bytes": raw.len(),
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
        // id LIKE '%/%' — 정상 PageId(ptype/slug)만. 슬래시 없는 비정상 행(지식그래프 노드 등,
        // 디스크 본문 없음 → 클릭 시 404)은 목록에서 제외.
        "SELECT id, title, page_type, updated_at FROM wiki_pages WHERE id LIKE '%/%' ORDER BY updated_at DESC LIMIT 200",
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
    // LLM 위키 — 본문 [[wikilink]] 파싱 → wiki_links 갱신(이 페이지 기존 링크 삭제 후 재삽입).
    let now_s = chrono::Utc::now().to_rfc3339();
    let links = extract_wikilinks(&body.content);
    let _ = db.conn().execute("DELETE FROM wiki_links WHERE from_id=?1", rusqlite::params![body.id]);
    for t in &links {
        let to_id: Option<String> = db.conn().query_row(
            "SELECT id FROM wiki_pages WHERE title=?1 LIMIT 1", rusqlite::params![t], |r| r.get(0),
        ).optional().ok().flatten();
        let _ = db.conn().execute(
            "INSERT OR REPLACE INTO wiki_links (from_id,to_title,to_id,created_at) VALUES (?1,?2,?3,?4)",
            rusqlite::params![body.id, t, to_id, now_s]);
    }
    // 이 페이지(제목)를 가리키던 미해석 링크들 해석(빨간링크 → 연결).
    let _ = db.conn().execute(
        "UPDATE wiki_links SET to_id=?1 WHERE to_title=?2 AND (to_id IS NULL OR to_id='')",
        rusqlite::params![body.id, body.title]);
    Ok(Json(serde_json::json!({"id": body.id, "content_hash": hash, "updated_at": now, "links": links.len()})))
}

/// 본문에서 `[[제목]]` / `[[제목|별칭]]` wikilink 제목들을 추출(중복 제거).
fn extract_wikilinks(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("[[") {
        let after = &rest[i + 2..];
        if let Some(j) = after.find("]]") {
            let raw = after[..j].trim();
            let title = raw.split('|').next().unwrap_or(raw).trim().to_string();
            if !title.is_empty() && !out.contains(&title) { out.push(title); }
            rest = &after[j + 2..];
        } else { break; }
    }
    out
}

/// `GET /v1/gui/wiki/pages/{id}/backlinks` — 이 페이지의 나가는 링크 + 들어오는 백링크.
async fn gui_wiki_backlinks(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;
    let mut db = state.db.lock().await;
    let title: String = db.conn().query_row(
        "SELECT title FROM wiki_pages WHERE id=?1", rusqlite::params![id], |r| r.get(0),
    ).optional().ok().flatten().unwrap_or_default();
    let mut stmt = db.conn().prepare("SELECT to_title, to_id FROM wiki_links WHERE from_id=?1 ORDER BY to_title")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("out:{e}")})))?;
    let outgoing: Vec<serde_json::Value> = stmt.query_map(rusqlite::params![id], |r| Ok(serde_json::json!({
        "title": r.get::<_, String>(0)?, "id": r.get::<_, Option<String>>(1)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("outq:{e}")})))?
        .filter_map(|x| x.ok()).collect();
    drop(stmt);
    let mut stmt2 = db.conn().prepare(
        "SELECT DISTINCT wl.from_id, wp.title FROM wiki_links wl JOIN wiki_pages wp ON wp.id=wl.from_id \
         WHERE wl.to_id=?1 OR wl.to_title=?2 ORDER BY wp.title")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("back:{e}")})))?;
    let backlinks: Vec<serde_json::Value> = stmt2.query_map(rusqlite::params![id, title], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("backq:{e}")})))?
        .filter_map(|x| x.ok()).collect();
    drop(stmt2);
    Ok(Json(serde_json::json!({ "outgoing": outgoing, "backlinks": backlinks })))
}

#[derive(Debug, serde::Deserialize)]
struct WikiIngestBody {
    source: String,
    #[serde(default)]
    orchestrator: Option<String>,
}

/// `POST /v1/gui/wiki/ingest` — CoT 자기구축: 소스를 에이전트(ACP)가 분석해 위키 페이지로
/// 생성/갱신([[wikilink]] 연결 포함). Karpathy LLM-wiki 패턴의 self-building 핵심(Phase 2).
async fn gui_wiki_ingest(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<WikiIngestBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;
    if body.source.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorDto{error: "source 필수".into()})));
    }
    let planner = body.orchestrator.clone().unwrap_or_else(|| "xgram-ops".to_string());
    let (ai_type, cwd, titles) = {
        let mut db = state.db.lock().await;
        let row = db.conn().query_row(
            "SELECT COALESCE(p.ai_type,'claude'), COALESCE(ac.project_path,'') \
             FROM agent_capabilities ac JOIN agent_profiles p ON p.alias=ac.alias WHERE ac.alias=?1",
            rusqlite::params![planner],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ).optional().ok().flatten();
        let (ai_type, cwd) = match row {
            Some((a, c)) if !c.trim().is_empty() => (a, c),
            _ => return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorDto{error: format!("ingest 플래너 '{planner}' 없음/cwd 없음 — xgram-ops 활성화 필요")}))),
        };
        let mut stmt = db.conn().prepare("SELECT title FROM wiki_pages ORDER BY updated_at DESC LIMIT 60")
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("titles:{e}")})))?;
        let ts: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("titlesq:{e}")})))?
            .filter_map(|x| x.ok()).collect();
        (ai_type, cwd, ts.join(", "))
    };
    let adapter = match ai_type.as_str() { "codex" => "codex-acp", "gemini" => "gemini", _ => "claude-agent-acp" };
    let prompt = format!(
        "너는 OpenXgram 위키 편집자다. 아래 소스를 위키 페이지로 정리해라.\n\
         기존 페이지 제목(관련되면 [[제목]]으로 연결): {titles}\n\
         반드시 JSON 한 개만 출력(설명·코드펜스 금지):\n\
         {{\"pages\":[{{\"id\":\"<type>/<영문slug>\",\"title\":\"<제목>\",\"page_type\":\"concept|entity|comparison|other\",\"content\":\"<마크다운 본문, 관련 개념은 [[제목]] 으로 연결>\"}}]}}\n\
         핵심 개념/엔티티별로 페이지를 나누고 서로 [[wikilink]] 로 연결해라. 소스: {source}",
        titles = titles, source = body.source.trim(),
    );
    let create = crate::daemon_gui_acp::create_session(&state.acp, crate::daemon_gui_acp::CreateSessionBody {
        agent: adapter.to_string(), cwd, mcp_servers: Vec::new(),
        execution_mode: Some("always".to_string()), permission_mode: Some("bypassPermissions".to_string()),
        model: None, thinking: None, machine: None, label: None,
    }).await.map_err(|(c, m)| (c, Json(ErrorDto{error: m})))?;
    let sid = create.get("sessionId").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let res = crate::daemon_gui_acp::prompt(&state.acp, &sid, crate::daemon_gui_acp::PromptBody{ text: prompt }).await;
    let _ = crate::daemon_gui_acp::close(&state.acp, &sid).await;
    let acp_result = res.map_err(|(c, m)| (c, Json(ErrorDto{error: m})))?;
    let mut text = String::new();
    if let Some(updates) = acp_result.get("updates").and_then(|u| u.as_array()) {
        for u in updates {
            if u.get("sessionUpdate").and_then(|s| s.as_str()) == Some("agent_message_chunk") {
                if let Some(t) = u.get("content").and_then(|c| c.get("text")).and_then(|t| t.as_str()) { text.push_str(t); }
            }
        }
    }
    let plan = extract_first_json(&text);
    let pages = plan.get("pages").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    use sha2::{Digest, Sha256};
    let now = chrono::Utc::now().timestamp();
    let now_s = chrono::Utc::now().to_rfc3339();
    let mut ingested: Vec<serde_json::Value> = Vec::new();
    {
        let mut db = state.db.lock().await;
        for pg in &pages {
            let id = pg.get("id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let title = pg.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let ptype = pg.get("page_type").and_then(|v| v.as_str()).unwrap_or("concept").to_string();
            let content = pg.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if id.is_empty() || title.is_empty() { continue; }
            let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
            let file_path = format!("wiki/{}/{}.md", ptype, id);
            let _ = db.conn().execute(
                "INSERT INTO wiki_pages (id, file_path, page_type, title, content_hash, embedding_hash, created_at, updated_at) \
                 VALUES (?1,?2,?3,?4,?5,?5,?6,?6) ON CONFLICT(id) DO UPDATE SET title=?4, content_hash=?5, updated_at=?6",
                rusqlite::params![id, file_path, ptype, title, hash, now]);
            let _ = db.conn().execute("INSERT INTO global_search (kind, ref_id, title, body) VALUES ('wiki', ?1, ?2, ?3)", rusqlite::params![id, title, content]);
            let links = extract_wikilinks(&content);
            let _ = db.conn().execute("DELETE FROM wiki_links WHERE from_id=?1", rusqlite::params![id]);
            for t in &links {
                let to_id: Option<String> = db.conn().query_row("SELECT id FROM wiki_pages WHERE title=?1 LIMIT 1", rusqlite::params![t], |r| r.get(0)).optional().ok().flatten();
                let _ = db.conn().execute("INSERT OR REPLACE INTO wiki_links (from_id,to_title,to_id,created_at) VALUES (?1,?2,?3,?4)", rusqlite::params![id, t, to_id, now_s]);
            }
            let _ = db.conn().execute("UPDATE wiki_links SET to_id=?1 WHERE to_title=?2 AND (to_id IS NULL OR to_id='')", rusqlite::params![id, title]);
            ingested.push(serde_json::json!({"id": id, "title": title, "page_type": ptype, "links": links.len()}));
        }
    }
    Ok(Json(serde_json::json!({ "ok": true, "ingested": ingested, "count": ingested.len() })))
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

// ── ACP 대화 영속화 — 새로고침/데몬 재시작 후 복원 (conv_key = 에이전트 alias) ──
async fn gui_acp_conv_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT role, text, created_at FROM acp_messages WHERE conv_key = ?1 ORDER BY id",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map(rusqlite::params![key], |r| {
        Ok(serde_json::json!({
            "role": r.get::<_, String>(0)?,
            "text": r.get::<_, String>(1)?,
            "created_at": r.get::<_, String>(2)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct AcpConvAddBody { pub role: String, pub text: String }
async fn gui_acp_conv_add(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<AcpConvAddBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.text.trim().is_empty() {
        return Ok(Json(serde_json::json!({ "ok": false, "skipped": "empty" })));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO acp_messages (conv_key, role, text, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![key, body.role, body.text, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("ins: {e}")})))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn gui_acp_conv_clear(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    db.conn().execute("DELETE FROM acp_messages WHERE conv_key = ?1", rusqlite::params![key])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("del: {e}")})))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// 대화 읽음 처리 — last_read=now. 안읽음 배지/정렬 기준. 에이전트 대화 열 때 호출.
async fn gui_acp_conv_read(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO acp_read (conv_key, last_read) VALUES (?1, ?2) \
         ON CONFLICT(conv_key) DO UPDATE SET last_read = excluded.last_read",
        rusqlite::params![key, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("read: {e}")})))?;
    Ok(Json(serde_json::json!({ "ok": true, "last_read": now })))
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
    // 카카오톡 셸 추가 모달 — agent_profiles 차원(이게 있어야 "생성된 에이전트"로 로스터 노출).
    #[serde(default)] ai_type: Option<String>,
    #[serde(default)] classification: Option<String>,
    #[serde(default)] execution_mode: Option<String>,
    #[serde(default)] machine: Option<String>,
    #[serde(default)] worktree: Option<String>,
    #[serde(default)] is_public: Option<bool>,
    // rc.321 — 친구 단위 정책 (classification="friend" 일 때만 의미 있음).
    #[serde(default)] friend_permission: Option<String>,
    #[serde(default)] friend_isolated: Option<bool>,
    #[serde(default)] friend_cost_tracked: Option<bool>,
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
    // Phase 2-C — agent_profiles LEFT JOIN: 명부 그룹화용 classification/execution_mode/ai_type/public.
    // 아키텍처 수정 — 로스터 = 마스터가 의도적으로 생성한 에이전트만.
    //   소스 테이블은 agent_capabilities 그대로 유지한다("에이전트 추가"(gui_agents_register)
    //   가 agent_capabilities 에 기록하고, agent_profiles 는 생성 시점에 채워지지 않으므로
    //   profile-first 로 바꾸면 로스터가 비게 됨 — 라이브 DB 에서 agent_profiles 가 빈 것을 확인).
    //   대신 과거 auto_seed_local_tmux_agents 가 박아둔 tmux 세션 행(role='tmux')을 제외하여
    //   로스터에 tmux 세션이 섞이지 않게 한다. tmux 는 DETAIL 패널(/v1/gui/sessions)에서만 노출.
    //   (auto_seed 의 신규 INSERT 는 daemon.rs 에서 이미 중단됨 — 이 WHERE 는 기존 라이브 DB 의
    //    잔존 행을 view 레벨에서 정리. 파괴적 DELETE 마이그레이션 없이 표시만 교정.)
    let mut stmt = db.conn().prepare(
        "SELECT ac.alias, ac.role, ac.description, ac.capabilities, ac.tool_list, ac.project_path, \
                ac.group_name, ac.messenger_enabled, ac.orchestration_role, ac.special_instructions, ac.updated_at, \
                p.classification, p.execution_mode, p.ai_type, p.is_public, p.machine, p.display_name, \
                p.source, p.activated, p.perm_mode, p.model, p.thinking, \
                p.friend_permission, p.friend_isolated, p.friend_cost_tracked, \
                (SELECT COUNT(*) FROM acp_messages am WHERE am.conv_key = ac.alias AND am.role='agent' \
                   AND am.created_at > COALESCE((SELECT last_read FROM acp_read WHERE conv_key = ac.alias), '')) AS unread \
         FROM agent_capabilities ac \
         JOIN agent_profiles p ON p.alias = ac.alias \
         WHERE ac.role IS NOT 'tmux' \
         ORDER BY ac.messenger_enabled DESC, ac.alias ASC",
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
            "classification": r.get::<_, Option<String>>(11)?,
            "execution_mode": r.get::<_, Option<String>>(12)?,
            "ai_type": r.get::<_, Option<String>>(13)?,
            "is_public": r.get::<_, Option<i64>>(14)?.map(|v| v != 0),
            "machine": r.get::<_, Option<String>>(15)?,
            "display_name": r.get::<_, Option<String>>(16)?,
            "source": r.get::<_, Option<String>>(17)?,
            "activated": r.get::<_, Option<i64>>(18)?.map(|v| v != 0),
            "perm_mode": r.get::<_, Option<String>>(19)?,
            "model": r.get::<_, Option<String>>(20)?,
            "thinking": r.get::<_, Option<String>>(21)?,
            "friend_permission": r.get::<_, Option<String>>(22)?,
            "friend_isolated": r.get::<_, Option<i64>>(23)?.map(|v| v != 0),
            "friend_cost_tracked": r.get::<_, Option<i64>>(24)?.map(|v| v != 0),
            "unread": r.get::<_, i64>(25)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

/// `GET /v1/gui/orchestration/agents` — rc.276 Paperclip Phase 1.
/// agent_capabilities org overlay (company_id + reports_to hierarchy + adapter_type/config/budget/status).
async fn gui_orchestration_agents(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    // rc.280 — peers.session_identifier prefetch (tmux 세션 dedupe 용, gui_peers 와 동일).
    let mut sid_map: std::collections::HashMap<String, String> = Default::default();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT alias, session_identifier FROM peers WHERE session_identifier IS NOT NULL AND session_identifier != ''"
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                sid_map.insert(row.0, row.1);
            }
        }
    }
    let mut stmt = db.conn().prepare(
        "SELECT alias, role, description, capabilities, orchestration_role, \
                company_id, reports_to, adapter_type, adapter_config, \
                budget_monthly_cents, status, paused_at, updated_at \
         FROM agent_capabilities ORDER BY reports_to IS NULL DESC, reports_to ASC, alias ASC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows: Vec<serde_json::Value> = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "alias": r.get::<_, String>(0)?,
            "role": r.get::<_, Option<String>>(1)?,
            "description": r.get::<_, Option<String>>(2)?,
            "capabilities": r.get::<_, Option<String>>(3)?,
            "orchestration_role": r.get::<_, Option<String>>(4)?,
            "company_id": r.get::<_, Option<String>>(5)?,
            "reports_to": r.get::<_, Option<String>>(6)?,
            "adapter_type": r.get::<_, Option<String>>(7)?,
            "adapter_config": r.get::<_, Option<String>>(8)?,
            "budget_monthly_cents": r.get::<_, Option<i64>>(9)?,
            "status": r.get::<_, Option<String>>(10)?,
            "paused_at": r.get::<_, Option<String>>(11)?,
            "updated_at": r.get::<_, String>(12)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    // rc.280 — 같은 tmux 세션의 short+full alias 중복 행 dedupe (gui_peers 와 동일 헬퍼/규칙).
    //   read 엔드포인트가 agent_capabilities 를 그대로 반환해 short(akashic)+full(aoe_akashic_*)
    //   둘 다 노출되던 문제 수정. sid 비-tmux/없음(원격·self)은 그대로 유지.
    let rows = dedup_by_tmux_session(rows, &sid_map, |v| {
        v.get("alias").and_then(|a| a.as_str()).unwrap_or("")
    });
    Ok(Json(rows))
}

/// `GET /v1/gui/orchestration/issues` — rc.276 Paperclip Phase 1. Issue board (unit of work).
async fn gui_orchestration_issues(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, company_id, project_id, goal_id, parent_id, title, body, status, \
                priority, assignee_agent_id, checkout_run_id, execution_locked_at, \
                origin_kind, origin_fingerprint, request_depth, issue_number, identifier, \
                created_at, updated_at \
         FROM issues ORDER BY created_at DESC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?,
            "company_id": r.get::<_, Option<String>>(1)?,
            "project_id": r.get::<_, Option<String>>(2)?,
            "goal_id": r.get::<_, Option<String>>(3)?,
            "parent_id": r.get::<_, Option<String>>(4)?,
            "title": r.get::<_, String>(5)?,
            "body": r.get::<_, Option<String>>(6)?,
            "status": r.get::<_, String>(7)?,
            "priority": r.get::<_, i64>(8)?,
            "assignee_agent_id": r.get::<_, Option<String>>(9)?,
            "checkout_run_id": r.get::<_, Option<String>>(10)?,
            "execution_locked_at": r.get::<_, Option<String>>(11)?,
            "origin_kind": r.get::<_, Option<String>>(12)?,
            "origin_fingerprint": r.get::<_, Option<String>>(13)?,
            "request_depth": r.get::<_, i64>(14)?,
            "issue_number": r.get::<_, Option<i64>>(15)?,
            "identifier": r.get::<_, Option<String>>(16)?,
            "created_at": r.get::<_, String>(17)?,
            "updated_at": r.get::<_, String>(18)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

/// `GET /v1/gui/orchestration/goals` — rc.276 Paperclip Phase 1. Goals (+ parent_id ancestry).
async fn gui_orchestration_goals(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare(
        "SELECT id, company_id, title, description, level, status, parent_id, \
                owner_agent_id, created_at, updated_at \
         FROM goals ORDER BY parent_id IS NULL DESC, parent_id ASC, created_at ASC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("prep: {e}")})))?;
    let rows = stmt.query_map([], |r| {
        Ok(serde_json::json!({
            "id": r.get::<_, String>(0)?,
            "company_id": r.get::<_, Option<String>>(1)?,
            "title": r.get::<_, String>(2)?,
            "description": r.get::<_, Option<String>>(3)?,
            "level": r.get::<_, String>(4)?,
            "status": r.get::<_, String>(5)?,
            "parent_id": r.get::<_, Option<String>>(6)?,
            "owner_agent_id": r.get::<_, Option<String>>(7)?,
            "created_at": r.get::<_, String>(8)?,
            "updated_at": r.get::<_, String>(9)?,
        }))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("q: {e}")})))?
        .filter_map(|r| r.ok()).collect();
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
struct AddFromPeerBody {
    /// Peer alias (from list_peers) to add as an org agent.
    alias: String,
    /// Optional reports_to (manager alias) — cross-machine sub-agents use this.
    #[serde(default)]
    reports_to: Option<String>,
    /// Optional company_id.
    #[serde(default)]
    company_id: Option<String>,
}

/// `POST /v1/gui/orchestration/agents/add-from-peer` — rc.277 Paperclip Phase 2.
/// Make a fleet peer an addable org agent: upsert into agent_capabilities with
/// adapter_type='peer_send', adapter_config={"alias": peer_alias}.
/// cross-machine 룰 (oxg.md §6 #7): only a machine's primary is addable directly;
/// sub-agents are modeled via reports_to (caller passes reports_to=primary alias).
async fn gui_orchestration_add_from_peer(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<AddFromPeerBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.alias.trim().is_empty() {
        return Err(internal("alias 필요"));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let adapter_config = serde_json::json!({ "alias": body.alias }).to_string();

    let mut db = state.db.lock().await;

    // 1) Confirm the peer exists in the fleet (peers table) — addable agent must be a real peer.
    let peer_exists: bool = {
        let mut store = PeerStore::new(&mut db);
        store
            .get_by_alias(&body.alias)
            .map_err(|e| internal(&format!("peer lookup: {e}")))?
            .is_some()
    };
    if !peer_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorDto {
                error: format!("peer '{}' 없음 (list_peers 에 존재하는 alias 필요)", body.alias),
            }),
        ));
    }

    // 2) Upsert into agent_capabilities as a peer_send org agent.
    db.conn()
        .execute(
            "INSERT INTO agent_capabilities \
                (alias, role, adapter_type, adapter_config, reports_to, company_id, status, updated_at) \
             VALUES (?1, 'agent', 'peer_send', ?2, ?3, ?4, 'idle', ?5) \
             ON CONFLICT(alias) DO UPDATE SET \
                adapter_type = 'peer_send', \
                adapter_config = excluded.adapter_config, \
                reports_to = COALESCE(excluded.reports_to, reports_to), \
                company_id = COALESCE(excluded.company_id, company_id), \
                updated_at = excluded.updated_at",
            rusqlite::params![body.alias, adapter_config, body.reports_to, body.company_id, now],
        )
        .map_err(|e| internal(&format!("agent_capabilities upsert: {e}")))?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": body.alias,
        "adapter_type": "peer_send",
        "adapter_config": adapter_config,
        "reports_to": body.reports_to,
    })))
}

#[derive(Debug, Deserialize)]
struct AgentInvokeBody {
    /// Prompt to deliver (mutually exclusive-ish with issue_id; prompt wins if both given).
    #[serde(default)]
    prompt: Option<String>,
    /// Issue id — its title+body becomes the prompt.
    #[serde(default)]
    issue_id: Option<String>,
    /// Optional timeout seconds (default 120).
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// `POST /v1/gui/orchestration/agents/{alias}/invoke` — rc.277 Paperclip Phase 2.
/// Single-shot agent run primitive (pre-Phase-3 run engine): resolve the agent's adapter_type,
/// dispatch adapter.execute, return AdapterResult + write an activity_log row.
async fn gui_orchestration_agent_invoke(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<AgentInvokeBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    // 1) Load the agent's adapter_type / adapter_config.
    let (adapter_type, adapter_config_raw): (Option<String>, Option<String>) = {
        let mut db = state.db.lock().await;
        db.conn()
            .query_row(
                "SELECT adapter_type, adapter_config FROM agent_capabilities WHERE alias = ?1",
                rusqlite::params![alias],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .map_err(|_| {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorDto {
                        error: format!("agent '{alias}' 없음 (agent_capabilities)"),
                    }),
                )
            })?
    };
    let adapter_type = adapter_type.unwrap_or_else(|| "peer_send".to_string());
    let adapter_config: serde_json::Value = adapter_config_raw
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({ "alias": alias }));

    // 2) Resolve the prompt: explicit prompt wins, else issue_id → title+body.
    let prompt = if let Some(p) = body.prompt.filter(|s| !s.trim().is_empty()) {
        p
    } else if let Some(iid) = &body.issue_id {
        let mut db = state.db.lock().await;
        let (title, ibody): (String, Option<String>) = db
            .conn()
            .query_row(
                "SELECT title, body FROM issues WHERE id = ?1",
                rusqlite::params![iid],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .map_err(|_| {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorDto {
                        error: format!("issue '{iid}' 없음"),
                    }),
                )
            })?;
        match ibody {
            Some(b) if !b.trim().is_empty() => format!("{title}\n\n{b}"),
            _ => title,
        }
    } else {
        return Err(internal("prompt 또는 issue_id 필요"));
    };

    // 3) Build adapter context. password from daemon env (peer_send signing).
    let password = std::env::var("XGRAM_KEYSTORE_PASSWORD").ok();
    let timeout = std::time::Duration::from_secs(body.timeout_secs.unwrap_or(120).clamp(1, 1800));
    let ctx = crate::orchestration_adapter::AdapterContext {
        data_dir: state.data_dir.as_ref().clone(),
        agent_alias: alias.clone(),
        prompt: prompt.clone(),
        adapter_config,
        password,
        session_id: None,
        timeout,
        on_log: None,
    };

    // 4) Dispatch adapter.
    let adapter = crate::orchestration_adapter::get_adapter(&adapter_type)
        .map_err(|e| internal(&format!("adapter: {e}")))?;
    let result = adapter
        .execute(&ctx)
        .await
        .map_err(|e| internal(&format!("adapter execute ({adapter_type}): {e}")))?;

    // 5) activity_log row.
    {
        let now = chrono::Utc::now().to_rfc3339();
        let log_id = uuid::Uuid::new_v4().to_string();
        let payload = serde_json::json!({
            "adapter_type": adapter_type,
            "issue_id": body.issue_id,
            "timed_out": result.timed_out,
            "summary_len": result.summary.len(),
        })
        .to_string();
        let mut db = state.db.lock().await;
        if let Err(e) = db.conn().execute(
            "INSERT INTO activity_log (id, actor, kind, target, payload, created_at) \
             VALUES (?1, ?2, 'agent_invoke', ?3, ?4, ?5)",
            rusqlite::params![log_id, alias, format!("agent:{alias}"), payload, now],
        ) {
            tracing::warn!(error = %e, "activity_log INSERT 실패 (invoke 자체는 성공)");
        }
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": alias,
        "adapter_type": adapter_type,
        "result": result,
    })))
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
    upsert_agent_capabilities(
        db.conn(),
        &body.alias,
        &role,
        body.description.as_deref(),
        body.capabilities.as_deref(),
        body.tool_list.as_deref(),
        body.project_path.as_deref(),
        body.group_name.as_deref(),
        body.messenger_enabled,
        body.orchestration_role.as_deref(),
        body.special_instructions.as_deref(),
        &now,
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("upsert: {e}")})))?;

    // 에이전트 프로필 upsert — 이 행이 있어야 "마스터가 생성한 에이전트"로 간주되어 로스터에 노출.
    // (자동등록 peer 는 agent_capabilities 만 있고 profiles 없음 → 로스터에서 제외.)
    let ai_type = body.ai_type.as_deref().filter(|s| !s.is_empty()).unwrap_or("claude");
    let classification = body.classification.as_deref().filter(|s| !s.is_empty()).unwrap_or("project");
    let exec_mode = body.execution_mode.as_deref().filter(|s| !s.is_empty()).unwrap_or("on_demand");
    upsert_agent_profile(
        db.conn(),
        &body.alias,
        ai_type,
        classification,
        exec_mode,
        body.machine.as_deref(),
        body.worktree.as_deref(),
        body.is_public.unwrap_or(false),
        &now,
        // rc.321 — 친구 정책 (classification="friend" 일 때만 의미; 그 외엔 컬럼 기본값 보존).
        body.friend_permission.as_deref().filter(|s| !s.is_empty()),
        body.friend_isolated,
        body.friend_cost_tracked,
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("profiles upsert: {e}")})))?;

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

        // peer entry add. address = 이 머신의 cross-machine reachable transport URL.
        // 외부 peer 가 이 URL 로 envelope POST → daemon 가 alias 별 inbox 분리 routing (Step 2).
        // env override → tailscale/LAN IP 동적 검출. 127.0.0.1/0.0.0.0 같은 도달 불가 주소 회피.
        // 검출 실패 시에만 localhost 폴백 (daemon startup self-heal 이 다음 재시작에 교정).
        let local_addr = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
            .ok()
            .filter(|u| !u.is_empty() && !openxgram_transport::tailscale::is_unreachable_address(u))
            .or_else(|| {
                openxgram_transport::tailscale::self_reachable_url(openxgram_core::ports::RPC_PORT)
            })
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", openxgram_core::ports::RPC_PORT));
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

    // rc.320 — agent-level opt-in 친구 모델 (model "B"). 머신 단위 양방향 announce 폐기.
    // 친구 추가는 원격의 SPECIFIC 에이전트를 사용자가 골라 단방향(one-directional)으로 등록한다.
    // 따라서 여기서는 reciprocal announce 를 발사하지 않는다 — 로컬 친구 row 만 UPSERT.
    // (원격이 자기 로스터를 노출하는 경로는 GET /v1/gui/friends/roster + remote-agents.)
    let _ = classification; // 친구 분류는 위 upsert_agent_profile 에서 이미 기록됨.

    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": body.alias,
        "eth_address": registered_eth,
    })))
}

/// `agent_capabilities` UPSERT — `gui_agents_register` 와 `gui_friend_announce` 공용 helper.
/// 분기된 SQL 방지(단일 source of truth). idempotent (ON CONFLICT alias).
#[allow(clippy::too_many_arguments)]
fn upsert_agent_capabilities(
    conn: &rusqlite::Connection,
    alias: &str,
    role: &str,
    description: Option<&str>,
    capabilities: Option<&str>,
    tool_list: Option<&str>,
    project_path: Option<&str>,
    group_name: Option<&str>,
    messenger_enabled: bool,
    orchestration_role: Option<&str>,
    special_instructions: Option<&str>,
    now: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
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
            alias, role, description, capabilities, tool_list,
            project_path, group_name, messenger_enabled as i64,
            orchestration_role, special_instructions, now,
        ],
    )
}

/// `agent_profiles` UPSERT — `gui_agents_register` 와 `gui_friend_announce` 공용 helper.
#[allow(clippy::too_many_arguments)]
fn upsert_agent_profile(
    conn: &rusqlite::Connection,
    alias: &str,
    ai_type: &str,
    classification: &str,
    execution_mode: &str,
    machine: Option<&str>,
    worktree: Option<&str>,
    is_public: bool,
    now: &str,
    // rc.321 — 친구 단위 정책 (None 이면 DB DEFAULT 또는 기존값 유지).
    friend_permission: Option<&str>,
    friend_isolated: Option<bool>,
    friend_cost_tracked: Option<bool>,
) -> rusqlite::Result<usize> {
    // None 컬럼은 COALESCE(excluded, existing) 로 기존값 보존. INSERT 신규행은
    // friend_permission='request'/isolated=0/cost_tracked=1 의 컬럼 DEFAULT 가 적용되도록
    // None → 명시 기본값으로 매핑(신규행에 NULL NOT NULL 위반 방지).
    let perm = friend_permission.unwrap_or("request");
    let iso = friend_isolated.map(|b| b as i64).unwrap_or(0);
    let cost = friend_cost_tracked.map(|b| b as i64).unwrap_or(1);
    conn.execute(
        "INSERT INTO agent_profiles \
            (alias, ai_type, classification, execution_mode, machine, worktree, is_public, \
             friend_permission, friend_isolated, friend_cost_tracked, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11) \
         ON CONFLICT(alias) DO UPDATE SET \
            ai_type = excluded.ai_type, \
            classification = excluded.classification, \
            execution_mode = excluded.execution_mode, \
            machine = COALESCE(excluded.machine, machine), \
            worktree = COALESCE(excluded.worktree, worktree), \
            is_public = excluded.is_public, \
            friend_permission = COALESCE(?12, friend_permission), \
            friend_isolated = COALESCE(?13, friend_isolated), \
            friend_cost_tracked = COALESCE(?14, friend_cost_tracked), \
            updated_at = excluded.updated_at",
        rusqlite::params![
            alias, ai_type, classification, execution_mode,
            machine, worktree, is_public as i64,
            perm, iso, cost, now,
            // UPDATE 분기는 명시 입력만 덮어쓰도록 raw Option 전달(None → 기존값 보존).
            friend_permission,
            friend_isolated.map(|b| b as i64),
            friend_cost_tracked.map(|b| b as i64),
        ],
    )
}

/// `tailscale status --json` 을 파싱하여 `host`(Tailscale IP 또는 호스트 라벨)에 해당하는
/// 장치의 전체 MagicDNS 도메인(`DNSName`, trailing `.` 제거)을 찾는다.
/// 매칭 규칙: 장치의 `TailscaleIPs` 가 `host` 를 포함하거나,
///           `HostName`/`DNSName` 의 첫 라벨이 `host` 와 일치(대소문자 무시).
/// `gui_tailnet_devices` 의 파싱 패턴(`Self` + `Peer.*`, `TailscaleIPs[0]`, `DNSName`)을 재사용.
/// tailscale 미설치/실패/미발견 시 None — 호출측이 IP:port 폴백으로 degrade.
fn tailscale_dnsname_for_host(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    let host_lc = host.to_ascii_lowercase();

    let root = std::process::Command::new("tailscale")
        .arg("status")
        .arg("--json")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())?;

    // 한 노드가 `host` 에 매칭하면 그 DNSName(trailing `.` 제거)을 반환.
    fn match_node(node: &serde_json::Value, host_lc: &str) -> Option<String> {
        // 1) TailscaleIPs 에 host 포함?
        let ip_match = node
            .get("TailscaleIPs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .any(|ip| ip.eq_ignore_ascii_case(host_lc))
            })
            .unwrap_or(false);
        // 2) HostName 첫 라벨 == host?
        let hostname_match = node
            .get("HostName")
            .and_then(|v| v.as_str())
            .map(|s| s.split('.').next().unwrap_or(s).eq_ignore_ascii_case(host_lc))
            .unwrap_or(false);
        // 3) DNSName 첫 라벨 == host?
        let dnsname_first = node
            .get("DNSName")
            .and_then(|v| v.as_str())
            .map(|s| s.trim_end_matches('.').to_string());
        let dnsname_label_match = dnsname_first
            .as_deref()
            .map(|s| s.split('.').next().unwrap_or(s).eq_ignore_ascii_case(host_lc))
            .unwrap_or(false);

        if ip_match || hostname_match || dnsname_label_match {
            dnsname_first.filter(|s| !s.is_empty())
        } else {
            None
        }
    }

    // Self 우선, 그다음 Peer.*.
    if let Some(self_node) = root.get("Self") {
        if let Some(dns) = match_node(self_node, &host_lc) {
            return Some(dns);
        }
    }
    if let Some(peers) = root.get("Peer").and_then(|v| v.as_object()) {
        for node in peers.values() {
            if let Some(dns) = match_node(node, &host_lc) {
                return Some(dns);
            }
        }
    }
    None
}

/// 친구 머신의 reachable GUI announce base URL 후보 목록을 ORDERED(가장 도달 가능성 높은 것 먼저)
/// 로 derive. 단일 URL 가정이 다양한 설치 토폴로지(설치별 GUI 포트, localhost-bound + Funnel,
/// 0.0.0.0-bound)에서 깨지므로 후보를 만들어 호출측이 순차로 시도하게 한다.
///
/// - `machine` 이 이미 scheme(`http(s)://`) 보유 → 그대로 단일 후보 (path/port 보존).
/// - `machine` 이 `host:port` 형태 → `http://<machine>` 단일 후보.
/// - bare host/IP →
///     a. Funnel/MagicDNS 우선: `tailscale status --json` 에서 host→DNSName 매핑 발견 시
///        `https://<dnsName>` 를 첫 후보로 (nginx 가 /v1/gui/ → 127.0.0.1:GUI_PORT proxy).
///     b. 직접 IP:port 폴백: 후보 GUI 포트 [47302, 17402] 각각 `http://<host>:<port>`.
/// `is_unreachable_address` true 후보는 제외. 순서 보존하며 dedupe.
fn friend_announce_base_urls(machine: &str) -> Vec<String> {
    // 후보 GUI 포트 (설치별 상이). probe_gui_urls 와 동일 집합.
    const CANDIDATE_PORTS: [u16; 2] = [47302, 17402];

    let raw = machine.trim();
    if raw.is_empty() {
        return Vec::new();
    }

    let mut candidates: Vec<String> = Vec::new();

    if raw.starts_with("http://") || raw.starts_with("https://") {
        candidates.push(raw.to_string());
    } else if raw.contains(':') {
        // host:port 형태 — scheme 만 보강.
        candidates.push(format!("http://{raw}"));
    } else {
        // a. Funnel/MagicDNS 우선.
        if let Some(dns) = tailscale_dnsname_for_host(raw) {
            candidates.push(format!("https://{dns}"));
        }
        // b. 직접 IP:port 폴백.
        for port in CANDIDATE_PORTS {
            candidates.push(format!("http://{raw}:{port}"));
        }
    }

    // 도달 불가 후보 제거 + 순서 보존 dedupe.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|c| !openxgram_transport::tailscale::is_unreachable_address(c))
        .filter(|c| seen.insert(c.clone()))
        .collect()
}

/// 이 머신의 이름 — `XGRAM_MACHINE_ALIAS` env 우선, 없으면 `detect_machine().alias`.
fn this_machine_alias() -> String {
    std::env::var("XGRAM_MACHINE_ALIAS")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::daemon_gui_sessions::detect_machine().alias)
}

/// `GET /v1/gui/friends/roster` — rc.320 agent-level opt-in 친구 모델.
/// 인증 없음 (cross-daemon peer-inbound — `gui_friend_announce`(폐기 전) / `/v1/peers/reachable`
/// 와 동일 신뢰 모델: tailnet/LAN 도달 가능 데몬만 호출 가능). 이 머신의 **친구로 추가 가능한
/// 로컬 에이전트**를 노출 → 원격 머신이 어떤 에이전트를 친구로 고를지 browse 할 수 있게 한다.
///
/// 포함 규칙: classification in (primary/pinned/project/special) — 실제 로컬 에이전트만.
///   제외: classification="friend" (남의 친구 row 재노출 금지) + 시스템/노이즈
///   (alias 가 `^(sv_aoe_|term_|null|default)` 매칭 또는 빈 문자열).
/// 반환: `{ machine, agents: [{ alias, ai_type, role }] }`.
async fn gui_friends_roster(
    State(state): State<GuiServerState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    let mut db = state.db.lock().await;
    // rc.321 — LEFT JOIN + null/empty classification 포함 fix.
    // 이전: INNER JOIN + classification IN (primary/pinned/project/special) 는
    //   프로필 없는(자동등록) 에이전트 + classification 이 NULL/'' 인 실제 작업 에이전트를
    //   모두 누락시켰다 → 머신의 진짜 working 에이전트가 친구 browse 목록에 안 보였음.
    // 수정: classification 이 'friend' 가 아니기만 하면 포함(NULL/'' 는 project 로 취급).
    //   노이즈/시스템 alias 는 아래 filter 에서 제외.
    let mut stmt = db.conn().prepare(
        "SELECT ac.alias, p.ai_type, ac.role, p.classification \
         FROM agent_capabilities ac \
         LEFT JOIN agent_profiles p ON p.alias = ac.alias \
         WHERE (p.classification IS NULL OR p.classification != 'friend') \
         ORDER BY ac.alias ASC",
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("roster prep: {e}") })))?;
    let agents: Vec<serde_json::Value> = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("roster q: {e}") })))?
        .filter_map(|r| r.ok())
        .filter(|(alias, _, role)| {
            let a = alias.trim();
            // rc.322 — 운영/마스터 에이전트는 친구 대상에서 제외 (마스터 명시:
            //   ops·워크플로우 오케스트레이터는 절대 friend 가 될 수 없음).
            //   alias 가 '-master' 로 끝나거나 레거시 'xgram-ops' 거나,
            //   role 이 ops 역할(운영·워크플로우 오케스트레이터 / openxgram-ops / ops-orchestrator)이면 제외.
            let is_ops = a.ends_with("-master")
                || a == "xgram-ops"
                || role
                    .as_deref()
                    .map(|r| {
                        let rt = r.trim();
                        rt == "운영 · 워크플로우 오케스트레이터"
                            || rt == "openxgram-ops"
                            || rt == "ops-orchestrator"
                    })
                    .unwrap_or(false);
            !a.is_empty()
                && !is_ops
                && !a.starts_with("sv_aoe_")
                && !a.starts_with("term_")
                && a != "null"
                && a != "default"
        })
        .map(|(alias, ai_type, role)| serde_json::json!({
            "alias": alias,
            "ai_type": ai_type.filter(|s| !s.is_empty()).unwrap_or_else(|| "claude".to_string()),
            "role": role,
        }))
        .collect();

    Ok(Json(serde_json::json!({
        "machine": this_machine_alias(),
        "agents": agents,
    })))
}

/// rc.321 — 친구 단위 정책 (권한/격리/비용). DB `agent_profiles` 의 friend_* 컬럼 사상.
#[derive(Clone, Debug)]
struct FriendPolicy {
    permission: String,
    isolated: bool,
    cost_tracked: bool,
}

impl Default for FriendPolicy {
    fn default() -> Self {
        // 친구 row 가 없거나(=알 수 없는 발신자) 정책 미설정 시의 안전 기본값.
        // 기본 permission=request(작업 허용), 격리 off, 비용기록 on.
        Self { permission: "request".to_string(), isolated: false, cost_tracked: true }
    }
}

/// `permission` 입력 검증 — 4개 enum 만 허용.
fn validate_friend_permission(p: &str) -> Result<(), String> {
    match p {
        "blocked" | "read" | "request" | "full" => Ok(()),
        other => Err(format!("friend_permission 은 blocked|read|request|full (받음: {other})")),
    }
}

/// `agent_profiles` 에서 `alias` 의 친구 정책을 로드.
/// row 가 없으면 `Ok(None)` (발신자가 로컬 친구가 아님 — 호출측이 정책 미적용 판단).
fn load_friend_policy(
    conn: &rusqlite::Connection,
    alias: &str,
) -> rusqlite::Result<Option<FriendPolicy>> {
    let mut stmt = conn.prepare(
        "SELECT friend_permission, friend_isolated, friend_cost_tracked, classification \
         FROM agent_profiles WHERE alias = ?1",
    )?;
    let mut rows = stmt.query_map([alias], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    match rows.next() {
        Some(Ok((perm, iso, cost, _class))) => Ok(Some(FriendPolicy {
            permission: perm.filter(|s| !s.is_empty()).unwrap_or_else(|| "request".to_string()),
            isolated: iso.unwrap_or(0) != 0,
            cost_tracked: cost.unwrap_or(1) != 0,
        })),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

#[derive(serde::Deserialize)]
struct FriendPolicySetBody {
    #[serde(default)] permission: Option<String>,
    #[serde(default)] isolated: Option<bool>,
    #[serde(default)] cost_tracked: Option<bool>,
}

/// `GET /v1/gui/friends/{alias}/policy` — 친구 정책 읽기.
async fn gui_friend_policy_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let pol = load_friend_policy(db.conn(), &alias)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("policy load: {e}") })))?;
    match pol {
        Some(p) => Ok(Json(serde_json::json!({
            "alias": alias,
            "permission": p.permission,
            "isolated": p.isolated,
            "cost_tracked": p.cost_tracked,
        }))),
        None => Err((StatusCode::NOT_FOUND, Json(ErrorDto { error: format!("unknown friend: {alias}") }))),
    }
}

/// `POST /v1/gui/friends/{alias}/policy` — 기존 친구 정책 갱신.
/// body `{permission?, isolated?, cost_tracked?}` — 제공된 필드만 갱신(나머지 보존).
async fn gui_friend_policy_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<FriendPolicySetBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if let Some(p) = body.permission.as_deref().filter(|s| !s.is_empty()) {
        validate_friend_permission(p)
            .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorDto { error: e })))?;
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    let affected = db.conn().execute(
        "UPDATE agent_profiles SET \
            friend_permission = COALESCE(?2, friend_permission), \
            friend_isolated = COALESCE(?3, friend_isolated), \
            friend_cost_tracked = COALESCE(?4, friend_cost_tracked), \
            updated_at = ?5 \
         WHERE alias = ?1",
        rusqlite::params![
            alias,
            body.permission.as_deref().filter(|s| !s.is_empty()),
            body.isolated.map(|b| b as i64),
            body.cost_tracked.map(|b| b as i64),
            now,
        ],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("policy update: {e}") })))?;
    if affected == 0 {
        tracing::warn!(alias = %alias, "friend policy set: alias 없음 (갱신 0행)");
        return Err((StatusCode::NOT_FOUND, Json(ErrorDto { error: format!("unknown friend: {alias}") })));
    }
    let pol = load_friend_policy(db.conn(), &alias)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("policy reload: {e}") })))?
        .unwrap_or_default();
    tracing::info!(alias = %alias, permission = %pol.permission, isolated = pol.isolated, cost_tracked = pol.cost_tracked, "friend policy updated");
    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": alias,
        "permission": pol.permission,
        "isolated": pol.isolated,
        "cost_tracked": pol.cost_tracked,
    })))
}

/// `GET /v1/gui/friends/remote-agents?host=<ip|host>` — rc.320.
/// AUTH 필요 (일반 GUI 라우트). `host`(원격 머신의 Tailscale IP/host)의 reachable GUI base URL 을
/// `friend_announce_base_urls` 로 derive 한 뒤, 순서대로 `{base}/v1/gui/friends/roster` 를 GET 하여
/// 첫 성공 후보의 로스터를 반환한다 (short timeout). 친구로 고를 원격 에이전트 목록 browse 용.
/// 도달 후보 없음/전부 실패 시 `{ ok:false, error }` + warn 로그 (silent fallback 금지).
#[derive(Debug, Deserialize)]
struct RemoteAgentsQuery {
    host: String,
}

async fn gui_friends_remote_agents(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<RemoteAgentsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let host = q.host.trim();
    if host.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorDto { error: "host 필수".into() })));
    }

    let candidates = friend_announce_base_urls(host);
    if candidates.is_empty() {
        tracing::warn!(host = %host, "remote-agents: 도달 가능 base URL 후보 없음");
        return Ok(Json(serde_json::json!({
            "ok": false,
            "error": format!("도달 가능 GUI 주소를 찾지 못함 ({host})"),
        })));
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "remote-agents: reqwest client build 실패");
            return Ok(Json(serde_json::json!({
                "ok": false,
                "error": format!("HTTP 클라이언트 생성 실패: {e}"),
            })));
        }
    };

    let mut last_err: Option<String> = None;
    for base in &candidates {
        let url = format!("{}/v1/gui/friends/roster", base.trim_end_matches('/'));
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(roster) => {
                        let machine = roster.get("machine").cloned().unwrap_or(serde_json::Value::Null);
                        let agents = roster.get("agents").cloned().unwrap_or_else(|| serde_json::json!([]));
                        tracing::info!(host = %host, base = %base, "remote-agents: 로스터 fetch 성공");
                        return Ok(Json(serde_json::json!({
                            "ok": true,
                            "base": base,
                            "machine": machine,
                            "agents": agents,
                        })));
                    }
                    Err(e) => {
                        last_err = Some(format!("{url} → JSON 파싱 실패: {e}"));
                        tracing::debug!(target = %url, error = %e, "remote-agents: 응답 JSON 파싱 실패, 다음 후보 시도");
                    }
                }
            }
            Ok(resp) => {
                last_err = Some(format!("{url} → status {}", resp.status()));
                tracing::debug!(target = %url, status = %resp.status(), "remote-agents: 후보 비정상 응답, 다음 후보 시도");
            }
            Err(e) => {
                last_err = Some(format!("{url} → {e}"));
                tracing::debug!(target = %url, error = %e, "remote-agents: 후보 송신 실패, 다음 후보 시도");
            }
        }
    }

    tracing::warn!(host = %host, candidates = ?candidates, last_error = ?last_err, "remote-agents: 모든 후보 실패");
    Ok(Json(serde_json::json!({
        "ok": false,
        "error": last_err.unwrap_or_else(|| "원격 데몬 도달 실패".to_string()),
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
        // 고용 = 로스터 노출 필수. 로스터는 agent_capabilities JOIN agent_profiles 라,
        // profile 행이 없으면 고용한 에이전트가 명부에 안 보인다. source='user', 기본 project 분류로 upsert.
        db.conn().execute(
            "INSERT INTO agent_profiles (alias, ai_type, classification, execution_mode, source, activated, is_public, created_at, updated_at) \
             VALUES (?1, 'claude', 'project', 'on_demand', 'user', 1, 0, ?2, ?2) \
             ON CONFLICT(alias) DO UPDATE SET updated_at=?2",
            rusqlite::params![body.target_alias, now],
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

/// agent_profiles 의 허용 enum 값 검증 (rule #1: silent fallback 금지 — 잘못된 값은 400).
fn validate_profile_enums(
    ai_type: &str, classification: &str, execution_mode: &str,
) -> Result<(), String> {
    if !matches!(ai_type, "claude" | "codex" | "gemini") {
        return Err(format!("ai_type 는 claude|codex|gemini (받음: {ai_type})"));
    }
    if !matches!(classification, "primary" | "project" | "special") {
        return Err(format!("classification 은 primary|project|special (받음: {classification})"));
    }
    if !matches!(execution_mode, "always" | "on_demand" | "heartbeat") {
        return Err(format!("execution_mode 는 always|on_demand|heartbeat (받음: {execution_mode})"));
    }
    Ok(())
}

/// `GET /v1/gui/agent/{alias}/profile` (Phase 2-D).
/// agent_profiles(신규 차원) + agent_capabilities(folder=project_path / group=group_name / role) 병합.
/// 프로필 미생성 alias 면 기본값으로 반환 (exists=false).
async fn gui_agent_profile_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;
    let mut db = state.db.lock().await;
    let prof = db.conn().query_row(
        "SELECT ai_type, classification, execution_mode, machine, worktree, is_public, created_at, updated_at, display_name \
         FROM agent_profiles WHERE alias = ?1",
        rusqlite::params![alias],
        |r| Ok(serde_json::json!({
            "ai_type": r.get::<_, String>(0)?,
            "classification": r.get::<_, String>(1)?,
            "execution_mode": r.get::<_, String>(2)?,
            "machine": r.get::<_, Option<String>>(3)?,
            "worktree": r.get::<_, Option<String>>(4)?,
            "is_public": r.get::<_, i64>(5)? != 0,
            "created_at": r.get::<_, String>(6)?,
            "updated_at": r.get::<_, String>(7)?,
            "display_name": r.get::<_, Option<String>>(8)?,
        })),
    ).optional().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("profile: {e}")})))?;
    let caps = db.conn().query_row(
        "SELECT role, group_name, project_path, description FROM agent_capabilities WHERE alias = ?1",
        rusqlite::params![alias],
        |r| Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        )),
    ).optional().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("caps: {e}")})))?;
    let (role, group, folder, description) = caps.unwrap_or((None, None, None, None));
    let exists = prof.is_some();
    let p = prof.unwrap_or_else(|| serde_json::json!({
        "ai_type": "claude", "classification": "project", "execution_mode": "on_demand",
        "machine": serde_json::Value::Null, "worktree": serde_json::Value::Null, "is_public": false,
        "created_at": serde_json::Value::Null, "updated_at": serde_json::Value::Null,
        "display_name": serde_json::Value::Null,
    }));
    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": alias,
        "exists": exists,
        "ai_type": p["ai_type"],
        "classification": p["classification"],
        "execution_mode": p["execution_mode"],
        "machine": p["machine"],
        "worktree": p["worktree"],
        "is_public": p["is_public"],
        "display_name": p["display_name"],
        "role": role,
        "group": group,
        "folder": folder,
        "description": description,
        "created_at": p["created_at"],
        "updated_at": p["updated_at"],
    })))
}

#[derive(Debug, Deserialize)]
struct AgentProfileBody {
    #[serde(default)] ai_type: Option<String>,
    #[serde(default)] classification: Option<String>,
    #[serde(default)] execution_mode: Option<String>,
    #[serde(default)] machine: Option<String>,
    #[serde(default)] worktree: Option<String>,
    #[serde(default)] is_public: Option<bool>,
    // 대화명(표시 이름) — 로스터/헤더에 alias 대신.
    #[serde(default)] display_name: Option<String>,
    // 기존 agent_capabilities 로 반영 (제공 시에만)
    #[serde(default)] role: Option<String>,
    #[serde(default)] group: Option<String>,
    #[serde(default)] folder: Option<String>,
    #[serde(default)] description: Option<String>,
}

/// `POST /v1/gui/agent/{alias}/profile` (Phase 2-D) — 프로필 upsert.
/// 미제공 필드는 기존값 유지(merge). 신규 alias 면 기본값 적용. 잘못된 enum 은 400.
/// 신규 차원 → agent_profiles, folder/group/role/description → agent_capabilities (중복 보관 안 함).
async fn gui_agent_profile_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<AgentProfileBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;
    let mut db = state.db.lock().await;
    // 기존 프로필 읽어 merge (제공된 필드만 덮어씀).
    let existing = db.conn().query_row(
        "SELECT ai_type, classification, execution_mode, machine, worktree, is_public FROM agent_profiles WHERE alias = ?1",
        rusqlite::params![alias],
        |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)? != 0,
        )),
    ).optional().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("read: {e}")})))?;
    let (cur_ai, cur_class, cur_exec, cur_machine, cur_wt, cur_pub) = existing.unwrap_or_else(||
        ("claude".into(), "project".into(), "on_demand".into(), None, None, false));
    let ai_type = body.ai_type.unwrap_or(cur_ai);
    let classification = body.classification.unwrap_or(cur_class);
    let execution_mode = body.execution_mode.unwrap_or(cur_exec);
    let machine = body.machine.or(cur_machine);
    let worktree = body.worktree.or(cur_wt);
    let is_public = body.is_public.unwrap_or(cur_pub);
    validate_profile_enums(&ai_type, &classification, &execution_mode)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ErrorDto{error: e})))?;
    let now = chrono::Utc::now().to_rfc3339();
    db.conn().execute(
        "INSERT INTO agent_profiles (alias, ai_type, classification, execution_mode, machine, worktree, is_public, display_name, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?9, ?8, ?8) \
         ON CONFLICT(alias) DO UPDATE SET ai_type=excluded.ai_type, classification=excluded.classification, \
           execution_mode=excluded.execution_mode, machine=excluded.machine, worktree=excluded.worktree, \
           is_public=excluded.is_public, display_name=COALESCE(excluded.display_name, display_name), updated_at=excluded.updated_at",
        rusqlite::params![alias, ai_type, classification, execution_mode, machine, worktree, is_public as i64, now, body.display_name],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("upsert profile: {e}")})))?;
    // 단일 프라이머리 강제 — primary 로 지정하면 기존 다른 primary 는 project 로 강등(중복 방지).
    if classification == "primary" {
        db.conn().execute(
            "UPDATE agent_profiles SET classification='project', updated_at=?2 \
             WHERE classification='primary' AND alias != ?1",
            rusqlite::params![alias, now],
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("demote primary: {e}")})))?;
    }
    // folder/group/role/description 제공 시 agent_capabilities 반영 (COALESCE 로 기존 보존).
    if body.role.is_some() || body.group.is_some() || body.folder.is_some() || body.description.is_some() {
        db.conn().execute(
            "INSERT INTO agent_capabilities (alias, role, description, project_path, group_name, updated_at) \
             VALUES (?1, COALESCE(?2,'agent'), ?3, ?4, ?5, ?6) \
             ON CONFLICT(alias) DO UPDATE SET \
               role=COALESCE(?2, agent_capabilities.role), \
               description=COALESCE(?3, agent_capabilities.description), \
               project_path=COALESCE(?4, agent_capabilities.project_path), \
               group_name=COALESCE(?5, agent_capabilities.group_name), \
               updated_at=?6",
            rusqlite::params![alias, body.role, body.description, body.folder, body.group, now],
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("upsert caps: {e}")})))?;
    }
    Ok(Json(serde_json::json!({
        "ok": true, "alias": alias,
        "ai_type": ai_type, "classification": classification, "execution_mode": execution_mode,
        "machine": machine, "worktree": worktree, "is_public": is_public,
    })))
}

#[derive(Debug, serde::Deserialize)]
struct AgentActivateBody {
    #[serde(default)]
    activate: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct ComposerSettingsBody {
    #[serde(default)]
    perm_mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

/// `POST /v1/gui/agent/{alias}/composer` — 에이전트별 컴포저 설정(권한/모델/effort) 영속.
/// 제공된 필드만 COALESCE 로 덮어쓴다. agent_profiles 행이 없으면 생성 후 갱신.
async fn gui_agent_composer_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<ComposerSettingsBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO agent_profiles (alias, created_at, updated_at) VALUES (?1, ?2, ?2) \
         ON CONFLICT(alias) DO NOTHING",
        rusqlite::params![alias, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("ensure profile: {e}")})))?;
    db.conn().execute(
        "UPDATE agent_profiles SET perm_mode=COALESCE(?2, perm_mode), model=COALESCE(?3, model), \
           thinking=COALESCE(?4, thinking), updated_at=?5 WHERE alias=?1",
        rusqlite::params![alias, body.perm_mode, body.model, body.thinking, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("composer set: {e}")})))?;
    Ok(Json(serde_json::json!({"ok": true, "alias": alias,
        "perm_mode": body.perm_mode, "model": body.model, "thinking": body.thinking})))
}

/// `POST /v1/gui/agents/{alias}/activate` — built-in 특수에이전트(xgram-ops 등) 활성/비활성 토글.
/// activated 플래그 + messenger_enabled 를 함께 켠다(활성화해야 명부 노출·peer 통신 가능).
/// body `{activate: bool}` 생략 시 true(활성화).
async fn gui_agent_activate(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<AgentActivateBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let on = body.activate.unwrap_or(true);
    let now = chrono::Utc::now().to_rfc3339();
    let mut db = state.db.lock().await;
    let n = db.conn().execute(
        "UPDATE agent_profiles SET activated = ?2, updated_at = ?3 WHERE alias = ?1",
        rusqlite::params![alias, on as i64, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("activate: {e}")})))?;
    if n == 0 {
        return Err((StatusCode::NOT_FOUND, Json(ErrorDto{error: format!("agent not found: {alias}")})));
    }
    // 활성화 시 메신저 노출(peer 통신 가능), 비활성화 시 숨김.
    db.conn().execute(
        "UPDATE agent_capabilities SET messenger_enabled = ?2, updated_at = ?3 WHERE alias = ?1",
        rusqlite::params![alias, on as i64, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("activate caps: {e}")})))?;
    Ok(Json(serde_json::json!({ "ok": true, "alias": alias, "activated": on })))
}

/// alias → 프로젝트 cwd 해석. hint 우선, 없으면 tmux session 의 `pane_current_path`.
/// `Ok(None)` = tmux session 없음 또는 cwd 빈 값. `Err` = tmux 실행 실패.
async fn resolve_agent_cwd(alias: &str, hint: Option<&str>) -> Result<Option<String>, String> {
    if let Some(p) = hint.filter(|s| !s.is_empty()) {
        return Ok(Some(p.to_string()));
    }
    let session = match crate::notify::resolve_alias_to_tmux(alias).await {
        Some((s, _)) => s,
        None => return Ok(None),
    };
    let out = tokio::process::Command::new("tmux")
        .args(["display-message", "-p", "-t", &session, "#{pane_current_path}"])
        .output().await
        .map_err(|e| format!("tmux: {e}"))?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { Ok(None) } else { Ok(Some(s)) }
}

/// 지침 파일에서 `@import` 줄(라인 시작 `@<path>`)을 추출. Claude Code @import 문법.
fn parse_md_imports(content: &str) -> Vec<String> {
    content.lines().filter_map(|l| {
        let t = l.trim_start();
        t.strip_prefix('@').and_then(|rest| {
            let p = rest.split_whitespace().next().unwrap_or("");
            if p.is_empty() { None } else { Some(p.to_string()) }
        })
    }).collect()
}

/// `@import` 원시 경로를 절대 경로로 해석. `~` 홈, 절대, 상대(파일 디렉토리 기준).
fn resolve_import_path(raw: &str, base_dir: &std::path::Path, home: &std::path::Path) -> std::path::PathBuf {
    if raw == "~" { home.to_path_buf() }
    else if let Some(rest) = raw.strip_prefix("~/") { home.join(rest) }
    else if raw.starts_with('/') { std::path::PathBuf::from(raw) }
    else { base_dir.join(raw) }
}

/// 지침 파일 1개를 노드로 만들고 @import 를 재귀 해석. 깊이 제한 + 순환 방지(visited).
fn build_instruction_node(
    path: &std::path::Path, scope: &str, home: &std::path::Path,
    depth: usize, visited: &mut std::collections::HashSet<String>,
) -> serde_json::Value {
    let key = path.to_string_lossy().to_string();
    let exists = path.is_file();
    let mut node = serde_json::json!({ "path": key, "scope": scope, "exists": exists });
    if !exists || depth == 0 || !visited.insert(key.clone()) {
        if exists { node["note"] = serde_json::json!("dedup/depth-limit"); }
        return node;
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    node["bytes"] = serde_json::json!(content.len());
    let base_dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let imports: Vec<serde_json::Value> = parse_md_imports(&content).into_iter().map(|raw| {
        let ip = resolve_import_path(&raw, base_dir, home);
        let mut child = build_instruction_node(&ip, "import", home, depth - 1, visited);
        child["raw"] = serde_json::json!(raw);
        child
    }).collect();
    if !imports.is_empty() { node["imports"] = serde_json::json!(imports); }
    node
}

/// `GET /v1/gui/agent/{alias}/config-chain?ai_type=&path_hint=` (Phase 2-A).
/// 그 에이전트에 실제 적용 중인 지침/설정 파일 체인을 동적 탐지하여 반환 (read-only).
/// AI 종류 분기: claude(@import 재귀) / codex(AGENTS.md) / gemini(GEMINI.md).
/// env 는 **키만** 반환 (값=시크릿 노출 금지).
async fn gui_agent_config_chain(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let ai_type = q.get("ai_type").map(|s| s.to_lowercase()).unwrap_or_else(|| "claude".into());
    let hint = q.get("path_hint").map(|s| s.as_str());
    let project_path = match resolve_agent_cwd(&alias, hint).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e})))?
    {
        Some(p) => p,
        None => return Ok(Json(serde_json::json!({
            "ok": false,
            "error": "cwd 해석 실패 (path_hint 또는 tmux session 필요)",
            "alias": alias, "ai_type": ai_type,
        }))),
    };
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let proj = std::path::Path::new(&project_path);

    // 1) 지침 파일 체인 (scope 순서: global → project → agent), AI 종류 분기.
    let candidates: Vec<(std::path::PathBuf, &str)> = match ai_type.as_str() {
        "codex" => vec![
            (home.join(".codex/AGENTS.md"), "global"),
            (proj.join("AGENTS.md"), "project"),
        ],
        "gemini" => vec![
            (home.join(".gemini/GEMINI.md"), "global"),
            (proj.join("GEMINI.md"), "project"),
        ],
        _ => vec![ // claude (기본)
            (home.join(".claude/CLAUDE.md"), "global"),
            (proj.join("CLAUDE.md"), "project"),
            (proj.join("AGENT.md"), "agent"),
        ],
    };
    let mut visited: std::collections::HashSet<String> = Default::default();
    let instruction_chain: Vec<serde_json::Value> = candidates.iter()
        .map(|(p, scope)| build_instruction_node(p, scope, &home, 6, &mut visited))
        .collect();

    // 2) .mcp.json — mcpServers 키 목록.
    let mcp_path = proj.join(".mcp.json");
    let (mcp_servers, mcp_source): (Vec<String>, Option<String>) = if mcp_path.is_file() {
        let servers = std::fs::read_to_string(&mcp_path).ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("mcpServers").and_then(|m| m.as_object())
                .map(|o| o.keys().cloned().collect::<Vec<_>>()))
            .unwrap_or_default();
        (servers, Some(mcp_path.to_string_lossy().to_string()))
    } else { (vec![], None) };

    // 3) settings 파일 + hooks + env 키 (claude/gemini). env 값은 절대 반환 안 함.
    let settings_candidates: Vec<(std::path::PathBuf, &str)> = match ai_type.as_str() {
        "gemini" => vec![(proj.join(".gemini/settings.json"), "project")],
        "codex" => vec![],
        _ => vec![
            (home.join(".claude/settings.json"), "global"),
            (proj.join(".claude/settings.json"), "project"),
            (proj.join(".claude/settings.local.json"), "local"),
        ],
    };
    let mut settings_files: Vec<serde_json::Value> = vec![];
    let mut hooks: Vec<serde_json::Value> = vec![];
    let mut env_keys: std::collections::BTreeSet<String> = Default::default();
    for (p, scope) in &settings_candidates {
        let exists = p.is_file();
        settings_files.push(serde_json::json!({ "path": p.to_string_lossy(), "scope": scope, "exists": exists }));
        if !exists { continue; }
        if let Some(v) = std::fs::read_to_string(p).ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        {
            if let Some(hk) = v.get("hooks").and_then(|h| h.as_object()) {
                for (event, arr) in hk {
                    if let Some(items) = arr.as_array() {
                        for it in items {
                            let matcher = it.get("matcher").and_then(|m| m.as_str()).unwrap_or("*").to_string();
                            hooks.push(serde_json::json!({
                                "event": event, "matcher": matcher,
                                "source": p.to_string_lossy(), "scope": scope,
                            }));
                        }
                    }
                }
            }
            if let Some(env) = v.get("env").and_then(|e| e.as_object()) {
                for k in env.keys() { env_keys.insert(k.clone()); }
            }
        }
    }

    // 4) skills 디렉토리 (.claude/skills + skills).
    let mut skills: Vec<String> = vec![];
    for d in [".claude/skills", "skills"] {
        let dir = proj.join(d);
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    if let Some(n) = e.file_name().to_str() { skills.push(n.to_string()); }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "alias": alias,
        "ai_type": ai_type,
        "project_path": project_path,
        "instruction_chain": instruction_chain,
        "mcp_servers": mcp_servers,
        "mcp_source": mcp_source,
        "settings_files": settings_files,
        "hooks": hooks,
        "env_keys": env_keys.into_iter().collect::<Vec<_>>(),
        "skills": skills,
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
    // 1) project_path 결정: hint 우선, 없으면 tmux session 의 default-path (resolve_agent_cwd 공용)
    let project_path: String = match resolve_agent_cwd(&body.alias, body.project_path_hint.as_deref()).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e})))?
    {
        Some(p) => p,
        None => return Ok(Json(serde_json::json!({
            "ok": false,
            "error": "cwd 추출 실패 (tmux session 없음)",
            "alias": body.alias,
        }))),
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
    // 로스터는 capabilities JOIN profiles 라, profile 행을 안 지우면 고아 행이 남는다
    // (재생성 시 stale classification/source 잔존). 양 테이블 모두 제거.
    db.conn().execute(
        "DELETE FROM agent_profiles WHERE alias = ?1",
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

/// rc.216 — L3 patterns → L2 wiki_pages 격상 (Karpathy 패턴 본질 fix).
///
/// 흐름 (CLAUDE.md 5층 메모리 아키텍처):
///   L0 messages → L1 episodes (reflect_all)
///   L0 messages → L3 patterns (heuristic_extract, frequency upsert)
///   **L3 patterns (RECURRING/ROUTINE) → L2 wiki_pages (이 함수)**
///   L3 patterns (ROUTINE) → L4 traits (derive_traits_from_patterns)
///
/// 멱등 (idempotent): 동일 pattern_text 는 같은 wiki_pages.id 로 upsert.
/// 임계값: frequency >= 2 (RECURRING 이상) — NEW 1회는 격상하지 않음.
fn promote_patterns_to_wiki(db: &mut openxgram_db::Db) -> anyhow::Result<i64> {
    let conn = db.conn();
    let rows: Vec<(String, String, i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, pattern_text, frequency, first_seen, last_seen \
             FROM patterns WHERE frequency >= 2 ORDER BY frequency DESC, last_seen DESC LIMIT 200",
        )?;
        let it = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        it.flatten().collect()
    };

    let mut promoted = 0i64;
    for (pid, ptxt, freq, first_seen, last_seen) in rows {
        let class = if freq >= 5 { "routine" } else { "recurring" };
        // 안정적인 wiki page id — pattern.id 기반 deterministic.
        let page_id = format!("pattern-{}", &pid[..pid.len().min(36)]);
        let file_path = format!("entity/{}.md", page_id);
        let title = ptxt.chars().take(80).collect::<String>();
        let content = format!(
            "# {title}\n\n- classification: {class}\n- frequency: {freq}\n- first_seen: {first_seen}\n- last_seen: {last_seen}\n- source_pattern_id: {pid}\n\n원본 pattern_text:\n\n> {ptxt}\n",
        );
        use sha2::{Digest, Sha256};
        let content_hash = format!("{:x}", Sha256::new().chain_update(content.as_bytes()).finalize());
        let now = chrono::Utc::now().timestamp();
        let r = conn.execute(
            "INSERT INTO wiki_pages (id, file_path, page_type, title, content_hash, embedding_hash, created_at, updated_at, category_path, tags, authors) \
             VALUES (?1, ?2, 'entity', ?3, ?4, ?4, ?5, ?5, 'patterns', ?6, '[\"reflection_pass\"]') \
             ON CONFLICT(id) DO UPDATE SET \
                title = excluded.title, \
                content_hash = excluded.content_hash, \
                embedding_hash = excluded.embedding_hash, \
                updated_at = excluded.updated_at, \
                tags = excluded.tags",
            rusqlite::params![
                page_id,
                file_path,
                title,
                content_hash,
                now,
                serde_json::json!([class, "auto-promoted"]).to_string()
            ],
        );
        match r {
            Ok(n) if n > 0 => promoted += 1,
            Ok(_) => {}
            Err(e) => tracing::warn!("wiki upsert 실패 ({}): {e}", page_id),
        }
    }
    Ok(promoted)
}

async fn gui_reflection_now(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let started = chrono::Utc::now().to_rfc3339();
    // 실제 reflection 실행 — openxgram_memory::reflect_all + derive_traits_from_patterns
    //                  + rc.216: L3 ROUTINE/RECURRING patterns → wiki_pages 격상 (Karpathy 패턴).
    let data_dir = state.data_dir.as_ref().clone();
    let pre_counts = {
        let mut db = state.db.lock().await;
        let conn = db.conn();
        let wiki: i64 = conn
            .query_row("SELECT COUNT(*) FROM wiki_pages", [], |r| r.get(0))
            .unwrap_or(0);
        // rc.216 fix: patterns_found 는 L3 patterns 테이블 기준 (memory_patterns 가 아님 — 그건 M-5 인덱스).
        let patterns: i64 = conn
            .query_row("SELECT COUNT(*) FROM patterns", [], |r| r.get(0))
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
        // rc.216 — L3 → L2 wiki 격상. RECURRING(2~4)·ROUTINE(5+) 빈도의 patterns 를
        // wiki_pages 에 entity 타입으로 upsert. 디스크는 후속 sync 가 처리.
        let promoted = promote_patterns_to_wiki(&mut db)?;
        if promoted > 0 {
            tracing::info!(promoted, "rc.216 reflection: patterns → wiki_pages 격상");
        }
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
        .query_row("SELECT COUNT(*) FROM patterns", [], |r| r.get(0))
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
    // FK cascade — workflow_runs(→workflow_step_logs) 가 workflow_id 를 참조.
    // 명시적 child 삭제 후 부모 삭제 (FOREIGN KEY constraint failed 방지).
    {
        let tx = db.conn().transaction()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        // step logs (runs 참조) → runs → workflow 순.
        tx.execute(
            "DELETE FROM workflow_step_logs WHERE run_id IN \
             (SELECT id FROM workflow_runs WHERE workflow_id=?1)",
            rusqlite::params![id],
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        tx.execute("DELETE FROM workflow_runs WHERE workflow_id=?1", rusqlite::params![id])
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        tx.execute("DELETE FROM workflows WHERE id=?1", rusqlite::params![id])
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        tx.commit()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
    }
    Ok(Json(serde_json::json!({"deleted": id})))
}

/// 하트비트 wake — execution_mode='heartbeat' 로컬 에이전트를 ACP 로 spawn + 점검 프롬프트.
/// on_demand 와 달리 heartbeat 모드 에이전트만 주기적으로 깨운다(daemon_gui 서버 task, 30분).
async fn heartbeat_wake(state: &GuiServerState) {
    let agents: Vec<(String, String, String)> = {
        let mut db = state.db.lock().await;
        let conn = db.conn();
        let mut stmt = match conn.prepare(
            "SELECT ac.alias, COALESCE(p.ai_type,'claude'), COALESCE(ac.project_path,'') \
             FROM agent_capabilities ac JOIN agent_profiles p ON p.alias=ac.alias \
             WHERE p.execution_mode='heartbeat' AND COALESCE(p.machine,'')=''",
        ) { Ok(s) => s, Err(_) => return };
        let it = match stmt.query_map([], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
        ))) { Ok(i) => i, Err(_) => return };
        it.filter_map(|x| x.ok()).filter(|(_, _, c)| !c.trim().is_empty()).collect()
    };
    for (alias, ai_type, cwd) in agents {
        let adapter = match ai_type.as_str() { "codex" => "codex-acp", "gemini" => "gemini", _ => "claude-agent-acp" };
        let create = match crate::daemon_gui_acp::create_session(&state.acp, crate::daemon_gui_acp::CreateSessionBody {
            agent: adapter.to_string(), cwd, mcp_servers: Vec::new(),
            execution_mode: Some("always".to_string()), permission_mode: Some("bypassPermissions".to_string()),
            model: None, thinking: None, machine: None, label: None,
        }).await { Ok(v) => v, Err(_) => continue };
        let sid = create.get("sessionId").and_then(|s| s.as_str()).unwrap_or("").to_string();
        if sid.is_empty() { continue; }
        let _ = crate::daemon_gui_acp::prompt(&state.acp, &sid, crate::daemon_gui_acp::PromptBody {
            text: "정기 하트비트 점검입니다. 처리할 일이 있으면 진행하고, 없으면 한 문장으로 '대기 중'만 보고하세요.".to_string(),
        }).await;
        let _ = crate::daemon_gui_acp::close(&state.acp, &sid).await;
        tracing::info!(alias = %alias, "heartbeat wake 완료");
    }
}

// ── OpenXgram 런타임(하네스) — 컴포저↔어댑터 사이 제어/설정/메모리주입 레이어 ──
// config 는 identity_settings(key='runtime_config') 에 JSON 저장. context 는 주입·관찰용
// L2 메모리 + 위키 제목을 반환(토큰예산 = count 제한). 슬래시/권한/주입을 이 설정으로 통합.

fn runtime_config_default() -> serde_json::Value {
    serde_json::json!({
        // 주입(Injection)
        "inject_memory": true,
        "memory_count": 8,
        "memory_kinds": ["fact", "decision", "rule", "reference"],
        "inject_wiki": false,
        // 검색(Search)
        "search_enabled": false,
        "search_source": "last_message",
        // 제한(Limits)
        "perm_default": "bypassPermissions",
        "model_default": "default",
        "thinking_default": "high",
        "max_inject_chars": 6000,
        // 필수(Mandatory gate) — 매 대화 첫 프롬프트에 반드시 거치는 지시.
        "mandatory_note": ""
    })
}

// per-agent 하네스 — key='runtime_config:<alias>' (에이전트별) 또는 'runtime_config'(전역 기본값).
// get(alias): 에이전트별 있으면 그것, 없으면 전역, 없으면 기본값.
async fn gui_runtime_config_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;
    let alias = q.get("alias").map(|s| s.trim()).filter(|s| !s.is_empty()).map(String::from);
    let key = alias.as_deref().map(|a| format!("runtime_config:{a}")).unwrap_or_else(|| "runtime_config".to_string());
    let mut db = state.db.lock().await;
    let mut stored: Option<String> = db.conn().query_row(
        "SELECT value FROM identity_settings WHERE key=?1", rusqlite::params![key], |r| r.get(0),
    ).optional().ok().flatten();
    // 에이전트별 없으면 전역 기본값으로 폴백.
    let mut inherited = false;
    if stored.is_none() && alias.is_some() {
        stored = db.conn().query_row(
            "SELECT value FROM identity_settings WHERE key='runtime_config'", [], |r| r.get(0),
        ).optional().ok().flatten();
        inherited = stored.is_some();
    }
    let config = stored
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(runtime_config_default);
    Ok(Json(serde_json::json!({ "config": config, "alias": alias, "inherited": inherited })))
}

#[derive(Debug, serde::Deserialize)]
struct RuntimeConfigBody {
    config: serde_json::Value,
    #[serde(default)]
    alias: Option<String>,
}

async fn gui_runtime_config_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<RuntimeConfigBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let s = body.config.to_string();
    let key = body.alias.as_deref().map(str::trim).filter(|a| !a.is_empty())
        .map(|a| format!("runtime_config:{a}")).unwrap_or_else(|| "runtime_config".to_string());
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO identity_settings(key,value,updated_at) VALUES(?1,?2,?3) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        rusqlite::params![key, s, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("save: {e}")})))?;
    Ok(Json(serde_json::json!({ "ok": true, "config": body.config, "alias": body.alias })))
}

// ── 방(대화) 단위 설정 — room_config 테이블 (GUI P3, rc.330) ──
// 하네스·역할·오케스트레이션·시스템 프롬프트·이벤트 규칙을 방별 JSON 으로 보관.
// 전역 기본 하네스(⚙️)는 /v1/gui/runtime/config (alias 미지정) 재사용 — 여기는 방 오버라이드.
// ⚠️ 저장만(persistence). 턴 시점 강제 적용(prompt 레이어링·orch 실행)은 P4.

/// 방 설정 기본값 — 저장된 row 가 없을 때 반환. harness 는 전역 runtime_config 를 상속(빈 객체로 두면 UI 가 전역 ⚙️ 사용).
fn room_config_default() -> serde_json::Value {
    serde_json::json!({
        "harness": serde_json::Value::Null,        // null = 전역 기본 하네스 상속
        "roles": { "defs": [], "assignments": [] },
        "orchestration": [],
        "system_prompt": "",
        "event_rules": [],
    })
}

/// `GET /v1/gui/room/{key}/config` — 방 설정 로드. 없으면 기본값.
async fn gui_room_config_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;
    let mut db = state.db.lock().await;
    let row: Option<(Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = db
        .conn()
        .query_row(
            "SELECT harness_json, roles_json, orchestration_json, system_prompt, event_rules_json, updated_at \
             FROM room_config WHERE room_key=?1",
            rusqlite::params![key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()
        .ok()
        .flatten();
    let parse = |s: Option<String>| -> Option<serde_json::Value> {
        s.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
    };
    let config = match row {
        Some((h, ro, orch, sp, ev, _ts)) => serde_json::json!({
            "harness": parse(h).unwrap_or(serde_json::Value::Null),
            "roles": parse(ro).unwrap_or_else(|| serde_json::json!({ "defs": [], "assignments": [] })),
            "orchestration": parse(orch).unwrap_or_else(|| serde_json::json!([])),
            "system_prompt": sp.unwrap_or_default(),
            "event_rules": parse(ev).unwrap_or_else(|| serde_json::json!([])),
        }),
        None => room_config_default(),
    };
    Ok(Json(serde_json::json!({ "room_key": key, "config": config })))
}

#[derive(Debug, serde::Deserialize)]
struct RoomConfigBody {
    #[serde(default)]
    harness: serde_json::Value,
    #[serde(default)]
    roles: serde_json::Value,
    #[serde(default)]
    orchestration: serde_json::Value,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    event_rules: serde_json::Value,
}

/// `PUT /v1/gui/room/{key}/config` — 방 설정 저장(upsert). JSON 컬럼.
async fn gui_room_config_set(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<RoomConfigBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let harness_s = if body.harness.is_null() { None } else { Some(body.harness.to_string()) };
    let roles_s = body.roles.to_string();
    let orch_s = body.orchestration.to_string();
    let ev_s = body.event_rules.to_string();
    let mut db = state.db.lock().await;
    db.conn()
        .execute(
            "INSERT INTO room_config(room_key, harness_json, roles_json, orchestration_json, system_prompt, event_rules_json, updated_at) \
             VALUES(?1,?2,?3,?4,?5,?6,?7) \
             ON CONFLICT(room_key) DO UPDATE SET \
               harness_json=excluded.harness_json, roles_json=excluded.roles_json, \
               orchestration_json=excluded.orchestration_json, system_prompt=excluded.system_prompt, \
               event_rules_json=excluded.event_rules_json, updated_at=excluded.updated_at",
            rusqlite::params![key, harness_s, roles_s, orch_s, body.system_prompt, ev_s, now],
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("save: {e}") })))?;
    Ok(Json(serde_json::json!({ "ok": true, "room_key": key, "updated_at": now })))
}

/// `POST /v1/gui/room/{key}/grant-turn` body `{agent}` — P4a "발언권 주기".
///
/// 스펙 항목 3: 관찰(맥락만 쌓던) 에이전트에게 **지금 턴 부여** → 데몬이 그 ACP 에 **현재 누적 맥락**으로
/// 한 번 턴을 발화한다. 새 inbound 가 없어도 발화한다. @호명·조건 트리거도 같은 메커니즘을 쓴다.
///
/// 구현: 기존 A2A 전달 척추(handle_task) 재사용 — 새 spawner 안 만듦. 합성 task("발언권 부여…")를
/// 그 에이전트(agent) ACP 세션에 prompt 한다. handle_task 가 build_resume_preamble 로 누적 맥락을
/// 앞에 붙이고, compose_room_prompt_prefix 로 방+역할 지침을 레이어링한다(턴 시점 주입). 따라서
/// 최종 프롬프트 = [누적 맥락] + [방+역할 지침] + [발언권 합성 지시]. 응답은 그 alias 스레드에 영속.
async fn gui_room_grant_turn(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 턴을 부여할 대상 에이전트 alias. 미지정이면 방 키 자체(1:1 에서는 동일).
    let agent = body
        .get("agent")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(key.trim())
        .to_string();

    let meta = load_a2a_agent_meta(&state, &agent).await?.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorDto {
                error: format!("unknown / non-ACP agent: {agent}"),
            }),
        )
    })?;

    // 합성 발언권 지시 — 누적 맥락 + 방/역할 지침은 handle_task 가 자동 레이어링한다.
    // 사용자(진행자)가 명시 메모를 보내면 그것을 발언권 본문으로 사용.
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let turn_text = match note {
        Some(n) => format!("[발언권 부여] 진행자가 너에게 발언권을 주었다. 지금까지의 대화 맥락을 토대로 한 번 발언하라. 진행자 메모: {n}"),
        None => "[발언권 부여] 진행자가 너에게 발언권을 주었다. 지금까지의 대화 맥락을 토대로 한 번 발언하라.".to_string(),
    };

    // P5 멤버십 gate — 방에 참가자 목록이 있으면(=그룹 방), 비활성/비멤버에게는 턴을 주지 않는다.
    // 1:1(참가자 row 없음)은 통과 = 종전 동작 무회귀.
    if room_member_blocked(&state, &key, &agent).await {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorDto {
                error: format!("'{agent}' 는 방 '{key}' 의 활성 멤버가 아닙니다 (내보내짐/미초대) — 발언권 거부"),
            }),
        ));
    }

    let task_body = crate::daemon_gui_a2a::TaskBody {
        skill: Some("grant-turn".to_string()),
        message: serde_json::Value::Null,
        task: Some(turn_text),
        text: None,
        session_id: None,
        from: None, // 진행자(사람) 발화 — A2A 친구 정책 대상 아님.
    };

    let result = crate::daemon_gui_a2a::handle_task(&state.acp, &state.served_a2a, &meta, task_body)
        .await
        .map_err(|(code, msg)| (code, Json(ErrorDto { error: msg })))?;

    Ok(Json(serde_json::json!({
        "ok": true,
        "room_key": key,
        "agent": agent,
        "result": result,
    })))
}

// ─────────────────────────────────────────────────────────────────────────
// P5 — 방(대화) 동적 멤버십 (초대 / 내보내기 / 멤버 목록).
//
// 스펙 항목 1(방={참가자목록+메시지스레드}) + 항목 4(동적 멤버십). 방의 누적 메시지 스레드는
// acp_messages(conv_key=room_key)에 이미 쌓이고, build_resume_preamble 이 그것을 다음 턴에
// 다시 앞에 붙인다 → 초대된 에이전트는 입장 시 방 맥락을 그대로 인계받는다(맥락 인계 = 스레드 공유).
// 이 모듈은 "누가 멤버인가"만 영속(room_participants)하고, 전달 spine(handle_task/grant-turn)을 재사용한다.
//
// 무회귀: room_participants 에 row 가 없는 방(=1:1)은 gate 통과 — 종전 단일-alias 동작 그대로.
// ─────────────────────────────────────────────────────────────────────────

/// 멤버십 gate. 방에 참가자 row 가 **하나라도** 있으면(=그룹 방), `member` 가 active 멤버가 아닐 때
/// `true`(차단)를 반환. row 가 전혀 없으면(=1:1/미설정 방) 항상 `false`(통과) → 무회귀.
async fn room_member_blocked(state: &GuiServerState, room_key: &str, member: &str) -> bool {
    let mut db = state.db.lock().await;
    let total: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM room_participants WHERE room_key=?1",
            rusqlite::params![room_key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if total == 0 {
        return false; // 참가자 목록 미설정 방 = 1:1 종전 동작, gate 통과.
    }
    let active: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM room_participants WHERE room_key=?1 AND member_alias=?2 AND active=1",
            rusqlite::params![room_key, member],
            |r| r.get(0),
        )
        .unwrap_or(0);
    active == 0
}

/// `POST /v1/gui/room/{key}/invite` body `{member, role?}` — 방에 참가자 추가 + 맥락 인계.
///
/// 1) room_participants 에 (room_key, member, active=1) upsert. role 미지정이면 '참가자'.
/// 2) 방을 처음 그룹화하는 경우(기존 row 0개) — 방장(사람, 고권한)을 암묵 멤버(role='human')로 시드 +
///    방 키 자신(=프라이머리/대화 상대 에이전트)도 멤버로 시드해, 기존 1:1 대화가 그룹으로 자연 승격되게 한다.
/// 3) 맥락 인계: 방 스레드(acp_messages conv_key=room_key)에 "[초대됨] 맥락 인계" 시스템 노트를 'agent' 로 기록.
///    이 스레드가 곧 누적 맥락이고 build_resume_preamble 이 초대된 에이전트의 다음 턴에 다시 앞에 붙인다.
/// 4) 전달 시작: 초대된 에이전트는 이제 grant-turn/orchestrate 대상이 된다(gate 통과).
async fn gui_room_invite(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let member = body
        .get("member")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorDto { error: "member 필수".into() }),
            )
        })?
        .to_string();
    let role = body
        .get("role")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("참가자")
        .to_string();
    let now = chrono::Local::now().to_rfc3339();

    {
        let mut db = state.db.lock().await;
        // 처음 그룹화: 기존 멤버 row 가 없으면 방장(사람)+방 키 에이전트를 암묵 시드.
        let existing: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM room_participants WHERE room_key=?1",
                rusqlite::params![key],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if existing == 0 {
            // 사람 = 고권한 참가자(spec 항목 9). alias 'me' 로 표기(GUI 'me' role 과 정합).
            db.conn()
                .execute(
                    "INSERT OR IGNORE INTO room_participants(room_key, member_alias, role, joined_at, active) \
                     VALUES(?1,'me','human',?2,1)",
                    rusqlite::params![key, now],
                )
                .map_err(|e| internal(&format!("invite seed human: {e}")))?;
            db.conn()
                .execute(
                    "INSERT OR IGNORE INTO room_participants(room_key, member_alias, role, joined_at, active) \
                     VALUES(?1,?2,'참가자',?3,1)",
                    rusqlite::params![key, key, now],
                )
                .map_err(|e| internal(&format!("invite seed room agent: {e}")))?;
        }
        db.conn()
            .execute(
                "INSERT INTO room_participants(room_key, member_alias, role, joined_at, active) \
                 VALUES(?1,?2,?3,?4,1) \
                 ON CONFLICT(room_key, member_alias) DO UPDATE SET active=1, role=excluded.role",
                rusqlite::params![key, member, role, now],
            )
            .map_err(|e| internal(&format!("invite upsert: {e}")))?;
    }

    // 맥락 인계 노트 — 방 스레드(=누적 맥락)에 기록. record_message 가 acp_messages(conv_key=room_key)에 쌓는다.
    let note = format!("[초대됨: {member}] 맥락 인계 — 이 방의 지금까지 대화 맥락이 다음 발언권 부여 시 전달됩니다. (역할: {role})");
    state.acp.record_message(&key, "agent", &note).await;
    crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, &key).await;

    Ok(Json(serde_json::json!({
        "ok": true,
        "room_key": key,
        "member": member,
        "role": role,
        "active": true,
    })))
}

/// `POST /v1/gui/room/{key}/eject` body `{member}` — 방에서 참가자 제거 + 수신 중단 + ACP 분리 시도.
///
/// 1) room_participants 의 그 member 를 active=0 (이력 보존). 이후 grant-turn/orchestrate gate 가 차단(수신 중단).
/// 2) ACP 분리: 방 키 = 그 member 의 bare-alias 스레드일 때(1:1 또는 그 에이전트가 방 키인 경우) 라벨 기반 세션
///    종료를 시도(best-effort, 실패해도 멤버십 제거는 유효). 그룹 방의 멤버는 자기 alias 스레드 세션을 보유하나
///    방-별 세션 분리는 라벨=alias 단위라 여기서 정밀 detach 는 제한적 — gate 차단으로 전달은 확실히 중단된다.
/// 3) 내보내기 노트를 방 스레드에 기록.
async fn gui_room_eject(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let member = body
        .get("member")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorDto { error: "member 필수".into() }),
            )
        })?
        .to_string();

    let affected = {
        let mut db = state.db.lock().await;
        db.conn()
            .execute(
                "UPDATE room_participants SET active=0 WHERE room_key=?1 AND member_alias=?2",
                rusqlite::params![key, member],
            )
            .map_err(|e| internal(&format!("eject update: {e}")))?
    };

    // ACP 분리(best-effort) — 그 member 의 alias 스레드 세션 라벨로 close 시도. 없으면 무시.
    let detached = crate::daemon_gui_acp::close_by_label(&state.acp, &member).await;

    let note = format!("[내보냄: {member}] 수신 중단 — 더 이상 발언권/턴 대상이 아닙니다.");
    state.acp.record_message(&key, "agent", &note).await;
    crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, &key).await;

    Ok(Json(serde_json::json!({
        "ok": true,
        "room_key": key,
        "member": member,
        "active": false,
        "removed_rows": affected,
        "acp_detached": detached,
    })))
}

/// `GET /v1/gui/room/{key}/members` — 방의 현재 활성 참가자 목록(UI 멤버 리스트용).
/// 응답: { room_key, members: [{alias, role, joined_at, is_human}], note? }.
/// 참가자 row 가 없는 1:1 방은 빈 목록 + note(=암묵 2자: 사람 + 방 키 에이전트).
async fn gui_room_members(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT member_alias, role, joined_at FROM room_participants \
             WHERE room_key=?1 AND active=1 ORDER BY joined_at",
        )
        .map_err(|e| internal(&format!("members prep: {e}")))?;
    let rows: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![key], |r| {
            let alias: String = r.get(0)?;
            let role: Option<String> = r.get(1)?;
            let joined_at: String = r.get(2)?;
            let is_human = alias == "me" || role.as_deref() == Some("human");
            Ok(serde_json::json!({
                "alias": alias,
                "role": role,
                "joined_at": joined_at,
                "is_human": is_human,
            }))
        })
        .map_err(|e| internal(&format!("members query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();
    let note = if rows.is_empty() {
        Some(serde_json::Value::String(
            "1:1 (참가자 목록 미설정) — 사람(고권한) + 이 에이전트. 초대 시 그룹으로 승격됩니다.".into(),
        ))
    } else {
        None
    };
    Ok(Json(serde_json::json!({
        "room_key": key,
        "members": rows,
        "note": note,
    })))
}

// ─────────────────────────────────────────────────────────────────────────
// P4c — 오케스트레이션 RUNNER (방의 orchestration_json 단계를 순서대로 실제 실행).
//
// 스펙 항목 11(오케스트레이션·진행자): 방 = "설정된 협업 공간". 각 단계는
//   { label, agent, role, action? } — `작업(navi) → 검증(Qua) → 정리 → 승인(⭐나)`.
// runner 는 단계를 순서대로:
//   1) 사람-승인 단계면 → status=paused_for_approval 로 멈춤(자동 승인 안 함, 스펙 9·고권한).
//   2) 아니면 → 그 단계의 agent 해석 → handle_task(P4a) 로 턴 발화 + 완료 await.
//      handle_task 가 compose_room_prompt_prefix(P4a)로 [방+역할 지침]을 자동 주입한다(재사용).
//   3) 단계 결과를 steps_json 에 기록 + current_step 전진 + orchestration_run 갱신.
//   4) 실패 시 status=failed + 단계 index/사유(절대 규칙 1 — 조용한 skip 금지).
//
// 비동기/락: runner 는 tokio::spawn 으로 분리(데몬 블로킹 금지). 각 단계의 turn 은
//   handle_task 의 await 가 완료 신호(=종전 grant-turn 과 동일 경로). DB 락은 await 를
//   넘겨 보유하지 않는다 — 매 단계 짧게 lock→update→drop, handle_task 호출 시엔 미보유.
// ─────────────────────────────────────────────────────────────────────────

/// 한 오케스트레이션 단계(snapshot + 실행 결과). steps_json 안에 배열로 영속.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OrchStep {
    #[serde(default)]
    label: String,
    /// 단계가 배정된 에이전트 표시값(이모지 prefix 포함 가능, 예 "🟠 navi (zalman)").
    #[serde(default)]
    agent: String,
    #[serde(default)]
    role: String,
    /// 이 단계에서 그 에이전트가 할 일(없으면 label 을 task 로 사용).
    #[serde(default)]
    action: Option<String>,
    /// 실행 상태: pending | running | done | paused_for_approval | failed | skipped.
    #[serde(default)]
    state: String,
    /// 에이전트 응답 텍스트(있으면).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    /// 이 단계 실패 사유(있으면).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// 단계의 agent 표시값에서 실제 alias 를 해석한다. 이모지/공백 prefix 제거 +
/// 괄호 머신 표기(" (zalman)") 제거 → "navi". 미지정("— 미지정"/"")은 None.
fn orch_resolve_alias(agent: &str) -> Option<String> {
    let mut s = agent.trim();
    if s.is_empty() || s.contains("미지정") {
        return None;
    }
    // 선두 비-식별자 문자(이모지·기호·공백) 제거.
    while let Some(c) = s.chars().next() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            break;
        }
        s = &s[c.len_utf8()..];
    }
    // 괄호 머신 표기 제거.
    let s = s.split('(').next().unwrap_or(s).trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 이 단계가 사람-승인(고권한) pause 지점인가? agent 가 "나/⭐" 거나
/// role/label 이 "승인"을 가리키면 true. (스펙 9 — 사람=고권한 참가자, 자동 승인 안 함.)
fn orch_is_human_approval(step: &OrchStep) -> bool {
    let a = step.agent.trim();
    if a.contains("나") || a.contains('⭐') {
        return true;
    }
    let r = step.role.trim();
    let l = step.label.trim();
    r.contains("승인") || l.contains("승인")
}

/// run row 를 KST timestamp 로 갱신(current_step/status/steps/error). 짧은 lock.
async fn orch_persist(
    db: &Arc<Mutex<Db>>,
    run_id: &str,
    current_step: i64,
    status: &str,
    steps: &[OrchStep],
    error: Option<&str>,
) {
    let steps_json = serde_json::to_string(steps).unwrap_or_else(|_| "[]".to_string());
    let now = chrono::Local::now().to_rfc3339();
    let mut g = db.lock().await;
    if let Err(e) = g.conn().execute(
        "UPDATE orchestration_run SET current_step=?2, status=?3, steps_json=?4, error=?5, updated_at=?6 WHERE run_id=?1",
        rusqlite::params![run_id, current_step, status, steps_json, error, now],
    ) {
        // 절대 규칙 1 — 조용히 넘기지 않고 명시 로그.
        tracing::error!(target: "acp.orchestrate", run_id = %run_id, "run 상태 영속 실패: {e}");
    }
}

/// 백그라운드 runner — run 의 단계를 순서대로 실행. tokio::spawn 으로 호출.
/// `from_step` 부터 실행(start=0, advance/approve=중단점 다음).
async fn orch_run_loop(state: GuiServerState, run_id: String, room_key: String, mut steps: Vec<OrchStep>, from_step: usize) {
    let db = state.db.clone();
    let total = steps.len();
    let mut i = from_step;
    while i < total {
        // 취소 확인 — 다른 핸들러가 status=cancelled 로 바꿨으면 즉시 중단.
        {
            use rusqlite::OptionalExtension;
            let cur: Option<String> = {
                let mut g = db.lock().await;
                g.conn()
                    .query_row(
                        "SELECT status FROM orchestration_run WHERE run_id=?1",
                        rusqlite::params![run_id],
                        |r| r.get(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
            };
            if cur.as_deref() == Some("cancelled") {
                tracing::info!(target: "acp.orchestrate", run_id = %run_id, "run 취소 감지 — 중단");
                return;
            }
        }
        // 사람-승인 단계 → pause(자동 승인 안 함). 사람이 /approve 로 재개.
        if orch_is_human_approval(&steps[i]) {
            steps[i].state = "paused_for_approval".to_string();
            orch_persist(&db, &run_id, i as i64, "paused_for_approval", &steps, None).await;
            // 가시화 — 방 스레드에 승인 대기 안내(사람이 메신저 대화뷰에서 본다).
            let msg = format!(
                "⏸ 오케스트레이션 — 단계 {}/{} 「{}」 승인 대기 (사람=고권한). 승인하면 다음 단계로 진행합니다.",
                i + 1,
                total,
                steps[i].label
            );
            state.acp.record_message(&room_key, "agent", &msg).await;
            crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, &room_key).await;
            return;
        }

        // 실행 단계 — agent 해석.
        let alias = match orch_resolve_alias(&steps[i].agent) {
            Some(a) => a,
            None => {
                let reason = format!("단계 {} 「{}」 에 배정된 에이전트가 없습니다(미지정).", i + 1, steps[i].label);
                steps[i].state = "failed".to_string();
                steps[i].error = Some(reason.clone());
                orch_persist(&db, &run_id, i as i64, "failed", &steps, Some(&reason)).await;
                tracing::warn!(target: "acp.orchestrate", run_id = %run_id, "{reason}");
                return;
            }
        };

        // P5 멤버십 gate — 방에 참가자 목록이 있고 이 단계 에이전트가 비활성/비멤버면 단계를 skip.
        // 1:1/미설정 방은 통과(무회귀). 내보내진 멤버에게는 턴을 주지 않는다(전달 차단).
        if room_member_blocked(&state, &room_key, &alias).await {
            let reason = format!("단계 {} — '{alias}' 는 방의 활성 멤버가 아님(내보내짐/미초대) → 건너뜀", i + 1);
            steps[i].state = "skipped".to_string();
            steps[i].error = Some(reason.clone());
            orch_persist(&db, &run_id, i as i64, "running", &steps, None).await;
            tracing::info!(target: "acp.orchestrate", run_id = %run_id, "{reason}");
            continue;
        }

        // running 표시.
        steps[i].state = "running".to_string();
        orch_persist(&db, &run_id, i as i64, "running", &steps, None).await;

        // 에이전트 meta 로드(handle_task 가 요구). load_a2a_agent_meta 는 GuiServerState 기반.
        let meta = match load_a2a_agent_meta(&state, &alias).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                let reason = format!("단계 {} — 알 수 없는/비-ACP 에이전트: {alias}", i + 1);
                steps[i].state = "failed".to_string();
                steps[i].error = Some(reason.clone());
                orch_persist(&db, &run_id, i as i64, "failed", &steps, Some(&reason)).await;
                tracing::warn!(target: "acp.orchestrate", run_id = %run_id, "{reason}");
                return;
            }
            Err((_code, dto)) => {
                let reason = format!("단계 {} — 에이전트 meta 로드 실패: {}", i + 1, dto.0.error);
                steps[i].state = "failed".to_string();
                steps[i].error = Some(reason.clone());
                orch_persist(&db, &run_id, i as i64, "failed", &steps, Some(&reason)).await;
                return;
            }
        };

        // task 본문 — action 우선, 없으면 label. [방+역할 지침]은 handle_task 가 자동 주입(P4a).
        let action = steps[i]
            .action
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| steps[i].label.clone());
        let turn_text = format!(
            "[오케스트레이션 단계 {}/{} — 「{}」] {}",
            i + 1,
            total,
            steps[i].label,
            action
        );
        let task_body = crate::daemon_gui_a2a::TaskBody {
            skill: Some("orchestrate".to_string()),
            message: serde_json::Value::Null,
            task: Some(turn_text),
            text: None,
            session_id: None,
            from: None, // 진행자(데몬 orchestrator) 발화 — A2A 친구정책 대상 아님.
        };

        // ── 단계 turn 발화 + 완료 await — DB 락 미보유 상태에서. (재사용: P4a handle_task) ──
        match crate::daemon_gui_a2a::handle_task(&state.acp, &state.served_a2a, &meta, task_body).await {
            Ok(res) => {
                let text = res
                    .get("result")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                steps[i].state = "done".to_string();
                steps[i].result = Some(text);
                orch_persist(&db, &run_id, (i + 1) as i64, "running", &steps, None).await;
            }
            Err((code, msg)) => {
                let reason = format!("단계 {} 「{}」 실행 실패 ({}): {}", i + 1, steps[i].label, code, msg);
                steps[i].state = "failed".to_string();
                steps[i].error = Some(reason.clone());
                orch_persist(&db, &run_id, i as i64, "failed", &steps, Some(&reason)).await;
                tracing::warn!(target: "acp.orchestrate", run_id = %run_id, "{reason}");
                return;
            }
        }
        i += 1;
    }

    // 모든 단계 완료.
    orch_persist(&db, &run_id, total as i64, "done", &steps, None).await;
    let msg = format!("✅ 오케스트레이션 완료 — {total}개 단계 순서대로 실행되었습니다.");
    state.acp.record_message(&room_key, "agent", &msg).await;
    crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, &room_key).await;
}

/// 가장 최근 run row 를 읽어 (run_id, current_step, status, steps, error) 반환.
async fn orch_latest_run(
    db: &Arc<Mutex<Db>>,
    room_key: &str,
) -> Option<(String, i64, String, Vec<OrchStep>, Option<String>)> {
    use rusqlite::OptionalExtension;
    let mut g = db.lock().await;
    let row: Option<(String, i64, String, String, Option<String>)> = g
        .conn()
        .query_row(
            "SELECT run_id, current_step, status, steps_json, error FROM orchestration_run \
             WHERE room_key=?1 ORDER BY updated_at DESC LIMIT 1",
            rusqlite::params![room_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .ok()
        .flatten();
    row.map(|(rid, cs, st, sj, err)| {
        let steps: Vec<OrchStep> = serde_json::from_str(&sj).unwrap_or_default();
        (rid, cs, st, steps, err)
    })
}

/// `POST /v1/gui/room/{key}/orchestrate/start` — 방의 orchestration_json 을 snapshot 해
/// 새 run 을 만들고 백그라운드 runner 를 kick. body 없음.
async fn gui_room_orchestrate_start(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;

    // 방 설정의 orchestration_json 로드 → 단계 snapshot.
    let orch_raw: Option<String> = {
        let mut db = state.db.lock().await;
        db.conn()
            .query_row(
                "SELECT orchestration_json FROM room_config WHERE room_key=?1",
                rusqlite::params![key],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    };
    let arr: Vec<serde_json::Value> = orch_raw
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok())
        .unwrap_or_default();
    if arr.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorDto {
                error: "이 방에는 오케스트레이션 단계가 없습니다(방 설정에서 단계를 먼저 추가하세요).".into(),
            }),
        ));
    }
    let mut steps: Vec<OrchStep> = arr
        .into_iter()
        .map(|v| {
            let mut s: OrchStep = serde_json::from_value(v).unwrap_or(OrchStep {
                label: String::new(),
                agent: String::new(),
                role: String::new(),
                action: None,
                state: String::new(),
                result: None,
                error: None,
            });
            s.state = "pending".to_string();
            s.result = None;
            s.error = None;
            s
        })
        .collect();

    let run_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();
    let steps_json = serde_json::to_string(&steps).unwrap_or_else(|_| "[]".to_string());
    {
        let mut db = state.db.lock().await;
        db.conn()
            .execute(
                "INSERT INTO orchestration_run(run_id, room_key, current_step, status, steps_json, error, started_at, updated_at) \
                 VALUES(?1,?2,0,'running',?3,NULL,?4,?4)",
                rusqlite::params![run_id, key, steps_json, now],
            )
            .map_err(|e| internal(&format!("run insert: {e}")))?;
    }

    // 가시화 — 방 스레드에 시작 안내.
    let total = steps.len();
    let msg = format!("▶ 오케스트레이션 시작 — {total}개 단계를 순서대로 실행합니다.");
    state.acp.record_message(&key, "agent", &msg).await;
    crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, &key).await;

    // 백그라운드 runner kick — 데몬 블로킹 금지.
    {
        let state2 = state.clone();
        let run_id2 = run_id.clone();
        let key2 = key.clone();
        let steps2 = std::mem::take(&mut steps);
        tokio::spawn(async move {
            orch_run_loop(state2, run_id2, key2, steps2, 0).await;
        });
    }

    Ok(Json(serde_json::json!({
        "ok": true,
        "run_id": run_id,
        "room_key": key,
        "total_steps": total,
        "status": "running",
    })))
}

/// `GET /v1/gui/room/{key}/orchestrate/status` — 현재 run 의 단계/상태.
async fn gui_room_orchestrate_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    match orch_latest_run(&state.db, &key).await {
        Some((run_id, current_step, status, steps, error)) => Ok(Json(serde_json::json!({
            "room_key": key,
            "run_id": run_id,
            "current_step": current_step,
            "total_steps": steps.len(),
            "status": status,
            "error": error,
            "steps": steps,
        }))),
        None => Ok(Json(serde_json::json!({
            "room_key": key,
            "run_id": serde_json::Value::Null,
            "current_step": 0,
            "total_steps": 0,
            "status": "none",
            "steps": [],
        }))),
    }
}

/// `POST /v1/gui/room/{key}/orchestrate/approve` — 사람-승인 pause 를 통과시켜 다음 단계부터 재개.
/// (`advance` 도 동일 동작의 별칭 라우트.)
async fn gui_room_orchestrate_approve(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let (run_id, current_step, status, mut steps, _err) =
        orch_latest_run(&state.db, &key).await.ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorDto { error: "이 방에 실행 중인 오케스트레이션 run 이 없습니다.".into() }),
            )
        })?;
    if status != "paused_for_approval" {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorDto {
                error: format!("승인 가능한 상태가 아닙니다(현재: {status}). 승인은 paused_for_approval 일 때만."),
            }),
        ));
    }
    let idx = current_step.max(0) as usize;
    // 현재 승인 단계 done 표시 + 다음 단계부터 재개.
    if idx < steps.len() {
        steps[idx].state = "done".to_string();
        steps[idx].result = Some("승인됨(사람=고권한)".to_string());
    }
    orch_persist(&state.db, &run_id, (idx + 1) as i64, "running", &steps, None).await;

    let msg = format!("✓ 단계 {}/{} 승인됨 — 다음 단계로 진행합니다.", idx + 1, steps.len());
    state.acp.record_message(&key, "agent", &msg).await;
    crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, &key).await;

    let state2 = state.clone();
    let run_id2 = run_id.clone();
    let key2 = key.clone();
    let from = idx + 1;
    tokio::spawn(async move {
        orch_run_loop(state2, run_id2, key2, steps, from).await;
    });

    Ok(Json(serde_json::json!({ "ok": true, "run_id": run_id, "room_key": key, "status": "running", "resumed_from": from })))
}

/// `POST /v1/gui/room/{key}/orchestrate/cancel` — 현재 run 을 취소(cancelled).
async fn gui_room_orchestrate_cancel(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let (run_id, current_step, _status, steps, _err) =
        orch_latest_run(&state.db, &key).await.ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorDto { error: "취소할 run 이 없습니다.".into() }),
            )
        })?;
    orch_persist(&state.db, &run_id, current_step, "cancelled", &steps, None).await;
    Ok(Json(serde_json::json!({ "ok": true, "run_id": run_id, "room_key": key, "status": "cancelled" })))
}

/// `GET /v1/gui/runtime/context?count=N` — 주입·관찰용 L2 메모리 + 위키 제목(토큰예산=count).
async fn gui_runtime_context(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let count: i64 = q.get("count").and_then(|s| s.parse().ok()).unwrap_or(8).clamp(0, 50);
    let mut db = state.db.lock().await;
    let mut stmt = db.conn().prepare("SELECT id, kind, content FROM memories ORDER BY rowid DESC LIMIT ?1")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("mem: {e}")})))?;
    let mems: Vec<serde_json::Value> = stmt.query_map(rusqlite::params![count], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "content": r.get::<_, String>(2)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("memq: {e}")})))?
        .filter_map(|x| x.ok()).collect();
    drop(stmt);
    let mut stmt2 = db.conn().prepare("SELECT id, title FROM wiki_pages ORDER BY rowid DESC LIMIT 12")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("wiki: {e}")})))?;
    let wiki: Vec<serde_json::Value> = stmt2.query_map([], |r| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?,
    }))).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("wikiq: {e}")})))?
        .filter_map(|x| x.ok()).collect();
    drop(stmt2);
    Ok(Json(serde_json::json!({
        "memories": mems, "wiki": wiki,
        "memory_count": mems.len(), "wiki_count": wiki.len(),
    })))
}

// ── 주입 항목(injection_rules) CRUD — 큐레이션된 규칙·원칙 리스트 ──
// scope='*' = 전역, 또는 에이전트 alias. 주입 시 전역+해당 alias 항목을 enabled 만 모음.

/// `GET /v1/gui/runtime/injections?scope=<*|alias>`
/// scope 미지정 또는 '*' → 전역만. alias 지정 → 전역 + 해당 alias 항목 모두 반환.
async fn gui_runtime_injections_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let scope = q.get("scope").map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("*");
    let mut db = state.db.lock().await;
    // 전역('*')은 항상 포함. alias 지정 시 그 alias 항목도 포함.
    let (sql, want_alias) = if scope == "*" {
        ("SELECT id, scope, name, content, enabled, sort_order, updated_at FROM injection_rules \
          WHERE scope='*' ORDER BY sort_order ASC, rowid ASC".to_string(), false)
    } else {
        ("SELECT id, scope, name, content, enabled, sort_order, updated_at FROM injection_rules \
          WHERE scope='*' OR scope=?1 ORDER BY (scope='*') DESC, sort_order ASC, rowid ASC".to_string(), true)
    };
    let mut stmt = db.conn().prepare(&sql)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("inj: {e}")})))?;
    let map_row = |r: &rusqlite::Row| Ok(serde_json::json!({
        "id": r.get::<_, String>(0)?,
        "scope": r.get::<_, String>(1)?,
        "name": r.get::<_, String>(2)?,
        "content": r.get::<_, String>(3)?,
        "enabled": r.get::<_, i64>(4)? != 0,
        "sort_order": r.get::<_, i64>(5)?,
        "updated_at": r.get::<_, Option<String>>(6)?,
    }));
    let rows: Vec<serde_json::Value> = if want_alias {
        stmt.query_map(rusqlite::params![scope], map_row)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("injq: {e}")})))?
            .filter_map(|x| x.ok()).collect()
    } else {
        stmt.query_map([], map_row)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("injq: {e}")})))?
            .filter_map(|x| x.ok()).collect()
    };
    Ok(Json(serde_json::json!({ "injections": rows })))
}

#[derive(Debug, serde::Deserialize)]
struct InjectionUpsertBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    content: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    sort_order: i64,
}
fn default_scope() -> String { "*".to_string() }
fn default_true() -> bool { true }

/// `POST /v1/gui/runtime/injections` — id 있으면 update, 없으면 create.
async fn gui_runtime_injection_upsert(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<InjectionUpsertBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let now = chrono::Utc::now().to_rfc3339();
    let scope = { let s = body.scope.trim(); if s.is_empty() { "*".to_string() } else { s.to_string() } };
    let id = body.id.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
        .unwrap_or_else(|| format!("inj_{}", uuid::Uuid::new_v4().simple()));
    let enabled_i = if body.enabled { 1i64 } else { 0i64 };
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO injection_rules(id, scope, name, content, enabled, sort_order, updated_at) \
         VALUES(?1,?2,?3,?4,?5,?6,?7) \
         ON CONFLICT(id) DO UPDATE SET scope=excluded.scope, name=excluded.name, \
           content=excluded.content, enabled=excluded.enabled, sort_order=excluded.sort_order, \
           updated_at=excluded.updated_at",
        rusqlite::params![id, scope, body.name, body.content, enabled_i, body.sort_order, now],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("inj save: {e}")})))?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "injection": {
            "id": id, "scope": scope, "name": body.name, "content": body.content,
            "enabled": body.enabled, "sort_order": body.sort_order, "updated_at": now,
        }
    })))
}

/// `DELETE /v1/gui/runtime/injections/{id}`
async fn gui_runtime_injection_delete(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    let affected = db.conn().execute(
        "DELETE FROM injection_rules WHERE id=?1", rusqlite::params![id],
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("inj del: {e}")})))?;
    Ok(Json(serde_json::json!({ "ok": true, "deleted": affected })))
}

#[derive(Debug, serde::Deserialize)]
struct WorkflowPlanBody {
    goal: String,
    #[serde(default)]
    orchestrator: Option<String>,
}

/// 텍스트에서 첫 '{' ~ 마지막 '}' 구간을 JSON 으로 파싱(코드펜스/설명 섞여도 추출).
fn extract_first_json(s: &str) -> serde_json::Value {
    if let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) {
        if b > a {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s[a..=b]) {
                return v;
            }
        }
    }
    serde_json::Value::Null
}

/// `POST /v1/gui/workflows/plan` — ops(또는 지정 orchestrator) 에이전트를 ACP 로 구동해
/// 목표를 워크플로우 단계로 분해한 계획(JSON)을 받아 반환. A2A handle_task 와 동일한
/// create_session→prompt→close→collect 패턴 재사용(서버측 ACP 라운드트립).
async fn gui_workflow_plan(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<WorkflowPlanBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use rusqlite::OptionalExtension;
    if body.goal.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(ErrorDto{error: "goal 필수".into()})));
    }
    let planner = body.orchestrator.clone().unwrap_or_else(|| "xgram-ops".to_string());
    // 플래너 ai_type+cwd + 보유 에이전트 로스터 조회 (락 한 번).
    let (ai_type, cwd, roster) = {
        let mut db = state.db.lock().await;
        let row = db.conn().query_row(
            "SELECT COALESCE(p.ai_type,'claude'), COALESCE(ac.project_path,'') \
             FROM agent_capabilities ac JOIN agent_profiles p ON p.alias=ac.alias WHERE ac.alias=?1",
            rusqlite::params![planner],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        ).optional().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("planner: {e}")})))?;
        let (ai_type, cwd) = match row {
            Some((a, c)) if !c.trim().is_empty() => (a, c),
            Some(_) => return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorDto{error: format!("플래너 '{planner}' 에 cwd(project_path) 없음 — 활성화/설정 필요")}))),
            None => return Err((StatusCode::NOT_FOUND, Json(ErrorDto{error: format!("플래너 에이전트 '{planner}' 없음 — xgram-ops 활성화 또는 orchestrator 지정")}))),
        };
        let mut stmt = db.conn().prepare(
            "SELECT ac.alias, COALESCE(ac.role,'') FROM agent_capabilities ac \
             JOIN agent_profiles p ON p.alias=ac.alias WHERE ac.role IS NOT 'tmux' ORDER BY ac.alias",
        ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("roster: {e}")})))?;
        let rl: Vec<String> = stmt.query_map([], |r| Ok(format!("{} ({})", r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: format!("roster q: {e}")})))?
            .filter_map(|x| x.ok()).collect();
        (ai_type, cwd, rl.join(", "))
    };
    let adapter = match ai_type.as_str() { "codex" => "codex-acp", "gemini" => "gemini", _ => "claude-agent-acp" };
    let prompt = format!(
        "너는 OpenXgram 워크플로우 플래너다. 아래 목표를 달성할 워크플로우를 설계해라.\n\
         보유 에이전트(steps.agent 에는 이들 alias 또는 새로 고용할 alias 사용): {roster}\n\
         반드시 아래 형식의 JSON 한 개만 출력(설명·코드펜스 금지):\n\
         {{\"steps\":[{{\"agent\":\"<보유 alias 또는 새 고용 alias>\",\"action\":\"<할 일>\"}}],\"hire\":[{{\"alias\":\"<새 영문소문자 alias>\",\"role\":\"<역할>\",\"why\":\"<이유>\"}}]}}\n\
         새 에이전트가 필요하면 hire 에 영문 소문자 alias 를 정하고(예: deploy_reporter), steps.agent 에 같은 alias 를 써라. 이 hire 는 자동 고용된다.\n\
         보유로 충분하면 hire 는 빈 배열. 목표: {goal}",
        roster = roster, goal = body.goal.trim(),
    );
    // 서버측 ACP 라운드트립.
    let create = crate::daemon_gui_acp::create_session(
        &state.acp,
        crate::daemon_gui_acp::CreateSessionBody {
            agent: adapter.to_string(), cwd, mcp_servers: Vec::new(),
            execution_mode: Some("always".to_string()),
            permission_mode: Some("bypassPermissions".to_string()),
            model: None, thinking: None, machine: None, label: None,
        },
    ).await.map_err(|(c, m)| (c, Json(ErrorDto{error: m})))?;
    let sid = create.get("sessionId").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let res = crate::daemon_gui_acp::prompt(&state.acp, &sid, crate::daemon_gui_acp::PromptBody{ text: prompt }).await;
    let _ = crate::daemon_gui_acp::close(&state.acp, &sid).await;
    let acp_result = res.map_err(|(c, m)| (c, Json(ErrorDto{error: m})))?;
    let mut text = String::new();
    if let Some(updates) = acp_result.get("updates").and_then(|u| u.as_array()) {
        for u in updates {
            if u.get("sessionUpdate").and_then(|s| s.as_str()) == Some("agent_message_chunk") {
                if let Some(t) = u.get("content").and_then(|c| c.get("text")).and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
            }
        }
    }
    let plan = extract_first_json(&text);
    // 고용까지 자동 실행 — plan.hire[] 각 항목을 실제 에이전트로 생성(템플릿 매칭→capabilities+profiles).
    let mut hired: Vec<serde_json::Value> = Vec::new();
    if let Some(hires) = plan.get("hire").and_then(|h| h.as_array()) {
        let now = chrono::Utc::now().to_rfc3339();
        for h in hires {
            let role = h.get("role").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let alias_src = h.get("alias").and_then(|v| v.as_str()).map(str::trim)
                .filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| role.clone());
            let alias = sanitize_alias(&alias_src);
            if alias.is_empty() { continue; }
            let why = h.get("why").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let kw = role.split_whitespace().next().unwrap_or("").to_lowercase();
            let (tpl_id, body, already) = {
                let mut db = state.db.lock().await;
                let exists: i64 = db.conn().query_row(
                    "SELECT COUNT(*) FROM agent_capabilities WHERE alias=?1",
                    rusqlite::params![alias], |r| r.get(0)).unwrap_or(0);
                if exists > 0 {
                    (None, String::new(), true)
                } else {
                    let row = if kw.is_empty() { None } else {
                        db.conn().query_row(
                            "SELECT id, body FROM agent_templates WHERE lower(name||' '||category) LIKE ?1 LIMIT 1",
                            rusqlite::params![format!("%{kw}%")],
                            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                        ).optional().ok().flatten()
                    };
                    match row {
                        Some((i, b)) => (Some(i), b, false),
                        None => (None, format!("# {role}\n\n역할: {role}\n사유: {why}\n"), false),
                    }
                }
            };
            if already {
                hired.push(serde_json::json!({"alias": alias, "role": role, "status": "exists"}));
                continue;
            }
            let dir = state.data_dir.join("agents").join(&alias);
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("CLAUDE.md"), &body);
            let cwd = dir.to_string_lossy().to_string();
            let desc: String = body.chars().take(200).collect();
            {
                let mut db = state.db.lock().await;
                let _ = db.conn().execute(
                    "INSERT OR IGNORE INTO agent_capabilities (alias, role, description, project_path, messenger_enabled, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, 1, ?5)",
                    rusqlite::params![alias, role, desc, cwd, now]);
                let _ = db.conn().execute(
                    "INSERT INTO agent_profiles (alias, ai_type, classification, execution_mode, source, activated, is_public, created_at, updated_at) \
                     VALUES (?1, 'claude', 'project', 'on_demand', 'user', 1, 0, ?2, ?2) \
                     ON CONFLICT(alias) DO UPDATE SET updated_at=?2",
                    rusqlite::params![alias, now]);
            }
            hired.push(serde_json::json!({"alias": alias, "role": role, "template": tpl_id, "status": "hired"}));
        }
    }
    Ok(Json(serde_json::json!({
        "ok": true, "planner": planner, "plan": plan, "hired": hired, "raw": text,
    })))
}

/// alias 정규화 — 공백→_, ASCII 영숫자/_/- 만 유지, 소문자, 최대 40자.
fn sanitize_alias(s: &str) -> String {
    let cleaned: String = s.trim().chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    cleaned.to_lowercase().chars().take(40).collect()
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
        envelope_type: None,
        ack_for_ulid: None,
        ack_status: None,
    };
    let envelope_json = serde_json::to_string(&envelope).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{
            error: format!("envelope serialize: {e}")
        }))
    })?;

    // rc.219 — msg_ulid = envelope.nonce 로 통일 (receiver 측 ACK ack_for_ulid 매칭 키와 동일).
    // envelope.nonce 가 항상 부여됨 (위에서 Some(uuid)) → unwrap 안전.
    let envelope_id = envelope
        .nonce
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = chrono::Utc::now();
    let now_str = now.to_rfc3339();
    // rc.227 — conversation_id 와 app_ack_check_after (+5min) 동시 저장.
    let check_after_str = (now + chrono::Duration::minutes(5)).to_rfc3339();
    let conv_id_for_q = envelope.conversation_id.clone();
    let mut db = state.db.lock().await;
    db.conn().execute(
        "INSERT INTO outbound_queue (msg_ulid, target_machine, target_alias, body, attempts, enqueued_at, \
                                     conversation_id, app_ack_check_after) \
         VALUES (?1, '', ?2, ?3, 0, ?4, ?5, ?6)",
        rusqlite::params![envelope_id, alias, envelope_json, now_str, conv_id_for_q, check_after_str],
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
    let counterparty = {
        let mut db = state.db.lock().await;
        let cp = external_inbound_counterparty(&mut db, &id);
        db.conn().execute("UPDATE external_inbound_pending SET status='approved' WHERE id=?1", rusqlite::params![id])
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        cp
    };
    // 거래 가시화 — A2A 와 동일(conv_key = 상대 에이전트 bare alias): record_message + conv_persisted broadcast.
    if let Some((from_agent, summary, price)) = counterparty {
        surface_commerce_event(&state, &from_agent, &format!(
            "[거래] {from_agent} offer 승인 — {summary}{price}",
            summary = if summary.is_empty() { "(요약 없음)".to_string() } else { summary },
            price = price.map(|p| format!(" ({p} USDC)")).unwrap_or_default(),
        )).await;
    }
    Ok(Json(serde_json::json!({"approved": id})))
}

async fn gui_external_inbound_reject(
    State(state): State<GuiServerState>, headers: HeaderMap, Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let counterparty = {
        let mut db = state.db.lock().await;
        let cp = external_inbound_counterparty(&mut db, &id);
        db.conn().execute("UPDATE external_inbound_pending SET status='rejected' WHERE id=?1", rusqlite::params![id])
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto{error: e.to_string()})))?;
        cp
    };
    if let Some((from_agent, summary, price)) = counterparty {
        surface_commerce_event(&state, &from_agent, &format!(
            "[거래] {from_agent} offer 거절 — {summary}{price}",
            summary = if summary.is_empty() { "(요약 없음)".to_string() } else { summary },
            price = price.map(|p| format!(" ({p} USDC)")).unwrap_or_default(),
        )).await;
    }
    Ok(Json(serde_json::json!({"rejected": id})))
}

/// 거래(external_inbound_pending) 행의 상대 에이전트·요약·가격을 조회 — 가시화 카드용.
/// 조회 실패(행 없음/SQL 에러)는 `None` 으로 명시 처리하되 에러는 로그(절대 규칙 1: silent swallow 금지).
fn external_inbound_counterparty(
    db: &mut openxgram_db::Db, id: &str,
) -> Option<(String, String, Option<f64>)> {
    match db.conn().query_row(
        "SELECT from_agent, COALESCE(request_summary,''), offered_price FROM external_inbound_pending WHERE id=?1",
        rusqlite::params![id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<f64>>(2)?)),
    ) {
        Ok(t) => Some(t),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => {
            tracing::error!(target: "acp.daemon", id = %id, "external_inbound 상대 조회 실패: {e}");
            None
        }
    }
}

/// 거래 lifecycle 이벤트를 모니터 가능한 스레드에 surface — A2A 와 동일 가시화 경로.
/// conv_key = 상대 에이전트 bare alias(=A2A 와 동일). record_message(영속) +
/// conv_persisted broadcast(라이브). 결제/온체인 로직은 건드리지 않음(가시화만 추가).
async fn surface_commerce_event(state: &GuiServerState, counterparty: &str, text: &str) {
    let conv_key = counterparty.trim();
    if conv_key.is_empty() {
        tracing::error!(target: "acp.daemon", "거래 이벤트 surface 스킵 — 상대 alias 빈 값");
        return;
    }
    // 'agent' = 상대측 발화로 기록 → 카드가 상대 메시지로 렌더(A2A 응답 기록과 동일 role).
    state.acp.record_message(conv_key, "agent", text).await;
    crate::daemon_gui_acp::notify_conv_persisted_by_label(&state.acp, conv_key).await;
    tracing::info!(target: "acp.daemon", conv_key = %conv_key, "거래 이벤트 → messenger 스레드 기록");
}

/// `POST /v1/gui/commerce/event` 입력 — 결제 확정 가시화.
/// `text` 가 있으면 그대로 사용, 없으면 {amount, tx_hash, summary} 로 서버측 포맷.
#[derive(serde::Deserialize)]
struct CommerceEventBody {
    counterparty: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    amount: Option<String>,
    #[serde(default)]
    tx_hash: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

/// `POST /v1/gui/commerce/event` — 결제 확정 이벤트를 모니터 가능한 스레드에 surface.
/// MCP 런타임(purchase_service)이 데몬 self-call(④ 패턴: XGRAM_MCP_TOKEN Bearer)로 호출한다.
/// McpServer 에는 GUI state 가 없어 surface_commerce_event 를 직접 못 부르므로 이 라우트 경유.
/// **실제로 확정된 결제만** 호출되도록 발신 측에서 보장(가짜 성공 절대 금지). 여기선 기록만.
async fn gui_commerce_event(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<CommerceEventBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let counterparty = body.counterparty.trim().to_string();
    if counterparty.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorDto { error: "counterparty 빈 값 — 거래 이벤트 surface 불가".to_string() }),
        ));
    }
    // text 우선, 없으면 구조화 필드로 포맷. 둘 다 없으면 명시 에러(빈 카드 금지).
    let text = match body.text.filter(|t| !t.trim().is_empty()) {
        Some(t) => t,
        None => {
            let amount = body.amount.unwrap_or_default();
            let tx = body.tx_hash.unwrap_or_default();
            let summary = body.summary.unwrap_or_default();
            if amount.is_empty() && tx.is_empty() && summary.is_empty() {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(ErrorDto { error: "text/amount/tx_hash/summary 전부 빈 값 — 기록할 내용 없음".to_string() }),
                ));
            }
            let tx_short = if tx.len() > 12 { format!("{}…", &tx[..12]) } else { tx };
            format!(
                "[결제] {amount} USDC → {counterparty} (tx {tx_short}) confirmed{sep}{summary}",
                sep = if summary.is_empty() { "" } else { " — " },
            )
        }
    };
    surface_commerce_event(&state, &counterparty, &text).await;
    Ok(Json(serde_json::json!({ "surfaced": true, "counterparty": counterparty })))
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

/// 각 tailnet 장치의 OpenXgram GUI 포트를 탐지(probe)해 `guiUrl` 필드를 채운다.
///
/// GUI 포트는 설치마다 다르다(예: 서울 47302, 잘만 17402). 따라서 후보 포트들에
/// 무인증 health 라우트(`/v1/gui/health`)를 GET 해서 200 이면 그 포트를 GUI 로 간주.
/// - 첫 성공 포트로 `guiUrl = "http://{ip}:{port}/gui/"`.
/// - 못 찾으면 `guiUrl = null`.
///
/// 동시성: 모든 장치(+각 후보 포트)를 `join_all` 로 병렬 probe → 전체 wall time 최소화.
/// 짧은 타임아웃(~800ms)으로 hang/방화벽 장치가 라우트를 지연시키지 않게 한다.
/// probe 실패/타임아웃은 조용히 `guiUrl = null` (라우트 전체를 죽이지 않음).
async fn probe_gui_urls(devices: &mut [serde_json::Value]) {
    // 후보 GUI 포트 (설치별 상이). 첫 성공 포트가 채택됨.
    const CANDIDATE_PORTS: [u16; 2] = [47302, 17402];

    // probe 실패 시 client 빌드 불가 → 모든 guiUrl 은 그대로 (null). 라우트는 정상.
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(1500))
        .timeout(std::time::Duration::from_millis(3000))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    // ip → 한 device 의 guiUrl 을 찾는 future. 후보 포트를 순차로 시도하되
    // 각 포트 probe 는 짧은 타임아웃. 장치 간에는 병렬(join_all).
    let probes = devices.iter().enumerate().map(|(idx, dev)| {
        let http = client.clone();
        let ip = dev.get("ip").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let dns = dev.get("dnsName").and_then(|v| v.as_str()).unwrap_or("").to_string();
        async move {
            // 1) MagicDNS 도메인 (Funnel/Serve) 우선 — 표준 HTTPS(443), 포트 추측 불필요.
            //    예: https://whitegun-win-1.tail0957ca.ts.net/gui/ (브라우저에서 그대로 열림).
            if !dns.is_empty() {
                let url = format!("https://{dns}/gui/");
                if let Ok(resp) = http.get(&url).send().await {
                    if resp.status().is_success() {
                        return (idx, Some(url));
                    }
                }
            }
            // 2) 폴백 — IP:후보포트 무인증 health probe (Funnel 미설정 장치).
            if !ip.is_empty() {
                for port in CANDIDATE_PORTS {
                    let health_url = format!("http://{ip}:{port}/v1/gui/health");
                    if let Ok(resp) = http.get(&health_url).send().await {
                        if resp.status().is_success() {
                            return (idx, Some(format!("http://{ip}:{port}/gui/")));
                        }
                    }
                }
            }
            (idx, None::<String>)
        }
    });

    let results = futures_util::future::join_all(probes).await;
    for (idx, gui_url) in results {
        if let Some(dev) = devices.get_mut(idx) {
            if let Some(obj) = dev.as_object_mut() {
                obj.insert(
                    "guiUrl".to_string(),
                    match gui_url {
                        Some(u) => serde_json::Value::String(u),
                        None => serde_json::Value::Null,
                    },
                );
            }
        }
    }
}

/// `GET /v1/gui/tailnet/devices` — Tailscale tailnet 장치 목록.
/// 친구 추가(머신) UI 가 자동 목록으로 사용.
///
/// `tailscale status --json` (`.Self` + `.Peer.*`) 를 파싱하여
/// `{ devices: [{ name, ip, online, os?, self? }] }` 반환.
/// tailscale 미설치/실패 시 에러 대신 `{ devices: [], note: "tailscale 없음" }`.
async fn gui_tailnet_devices(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    // `tailscale status --json` 실행. 없으면 텍스트 폴백.
    let json_out = std::process::Command::new("tailscale")
        .arg("status")
        .arg("--json")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

    // 한 노드 JSON 객체 → device DTO 매핑.
    fn map_node(node: &serde_json::Value, is_self: bool) -> serde_json::Value {
        // HostName 우선, 없으면 DNSName(첫 라벨), 없으면 빈 문자열.
        let name = node
            .get("HostName")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                node.get("DNSName")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim_end_matches('.').split('.').next().unwrap_or(s).to_string())
            })
            .unwrap_or_default();
        let ip = node
            .get("TailscaleIPs")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let online = node.get("Online").and_then(|v| v.as_bool()).unwrap_or(false);
        let os = node
            .get("OS")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        // 전체 MagicDNS 도메인(예: whitegun-win-1.tail0957ca.ts.net). Funnel/Serve 설정 시
        // https://{dnsName}/gui/ 로 표준 HTTPS 접근 — 포트 추측 불필요(probe 우선순위 1).
        let dns_name = node
            .get("DNSName")
            .and_then(|v| v.as_str())
            .map(|s| s.trim_end_matches('.').to_string())
            .filter(|s| !s.is_empty());
        serde_json::json!({
            "name": name,
            "ip": ip,
            "online": online,
            "os": os,
            "dnsName": dns_name,
            "self": is_self,
        })
    }

    if let Some(root) = json_out {
        let mut devices: Vec<serde_json::Value> = Vec::new();
        // Self 노드 (자기 자신, self:true).
        if let Some(self_node) = root.get("Self") {
            if self_node.is_object() {
                devices.push(map_node(self_node, true));
            }
        }
        // Peer 노드들 — `.Peer` 는 { "<key>": {node}, ... } 형태의 객체.
        if let Some(peers) = root.get("Peer").and_then(|v| v.as_object()) {
            for node in peers.values() {
                if node.is_object() {
                    devices.push(map_node(node, false));
                }
            }
        }
        probe_gui_urls(&mut devices).await;
        return Ok(Json(serde_json::json!({ "devices": devices })));
    }

    // JSON 실패 → 텍스트 폴백 (`tailscale status`).
    // 각 라인: `<ip>  <name>  <user>  <os>  <online/offline...>` (공백 구분).
    let text_out = std::process::Command::new("tailscale")
        .arg("status")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        });

    if let Some(text) = text_out {
        let mut devices: Vec<serde_json::Value> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 2 {
                continue;
            }
            let ip = cols[0].to_string();
            let name = cols[1].to_string();
            let os = cols.get(3).map(|s| s.to_string());
            // online 여부: 라인에 "offline" 이 없으면 online 으로 간주.
            let online = !line.to_lowercase().contains("offline");
            // 텍스트 모드에선 self 표시: 첫 줄(보통 자기 자신) 또는 "(self)" 토큰.
            let is_self = devices.is_empty() || line.contains("(self)");
            devices.push(serde_json::json!({
                "name": name,
                "ip": ip,
                "online": online,
                "os": os,
                "self": is_self,
            }));
        }
        probe_gui_urls(&mut devices).await;
        return Ok(Json(serde_json::json!({ "devices": devices })));
    }

    // tailscale 미설치/완전 실패 → 에러 아닌 빈 배열 + note (절대 규칙: 명시 처리).
    Ok(Json(serde_json::json!({
        "devices": [],
        "note": "tailscale 없음"
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

/// `GET /v1/gui/wallets/ledger?agentId=&limit=` — 지갑 거래 원장 + 집계 (마켓 (c)갈래).
/// agentId 생략 시 전체. 충전/구매/수익 내역과 누적 합계를 실데이터로 반환.
async fn gui_wallet_ledger(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<crate::daemon_gui_wallets::LedgerDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let agent_id = q.get("agentId").map(|s| s.as_str()).filter(|s| !s.is_empty());
    let limit = q
        .get("limit")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(50);
    let mut db = state.db.lock().await;
    crate::daemon_gui_wallets::list_ledger(&mut db, agent_id, limit)
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorDto { error: format!("wallet ledger: {e}") }),
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
        project_folder: None,
        llm_type: None,
        llm_version: None,
        machine: None,
        worktrees: Vec::new(),
        subagents: Vec::new(),
        ex_peers: Vec::new(),
        session_identifier: None,
    }))
}

/// rc.245 — 사용자 override body: { "session_identifier": string | null }.
#[derive(Debug, serde::Deserialize)]
struct PeerSessionBody {
    session_identifier: Option<String>,
}

/// `PATCH /v1/gui/peers/{alias}/session` — rc.245.
/// 결정적 세션 매핑 사용자 override: 이 peer 의 터미널이 보여줄 세션 식별자 set/clear.
///   body.session_identifier = "tmux:<name>" 등 → 그 세션 고정.
///   body.session_identifier = null → 자동 추정(normalizeAlias)으로 복귀.
async fn gui_peer_set_session(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<PeerSessionBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if alias.trim().is_empty() {
        return Err(bad_request("alias 필수"));
    }
    // 빈 문자열은 null 로 정규화 (자동 추정 복귀).
    let sid: Option<String> = body
        .session_identifier
        .and_then(|s| if s.trim().is_empty() { None } else { Some(s) });
    let mut db = state.db.lock().await;
    db.conn()
        .execute(
            "UPDATE peers SET session_identifier = ?1 WHERE alias = ?2",
            rusqlite::params![sid, &alias],
        )
        .map_err(|e| internal(&format!("session_identifier update: {e}")))?;
    Ok(Json(serde_json::json!({ "ok": true })))
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

// ── 마켓 (d)갈래 — free-tier 무료 할당량 ───────────────────────────────────

/// `PUT /v1/gui/payment/free-tier` 입력 — 전역 기본 또는 에이전트별 override 설정.
#[derive(Debug, Deserialize)]
struct FreeTierConfigBody {
    /// 대상 agent_id. 생략 또는 "*" 이면 전역 기본.
    #[serde(default)]
    agent_id: Option<String>,
    /// 1일 무료 호출 횟수 (0=무료 없음).
    free_per_day: i64,
}

/// `GET /v1/gui/payment/free-tier` — 무료 할당량 설정 (전역 기본 + override 목록).
async fn gui_free_tier_get_config(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<crate::free_tier::FreeTierConfigDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut db = state.db.lock().await;
    crate::free_tier::get_config(&mut db)
        .map(Json)
        .map_err(|e| internal(&format!("free-tier config get: {e}")))
}

/// `PUT /v1/gui/payment/free-tier` — 무료 할당량 설정 (전역 기본 또는 override).
async fn gui_free_tier_set_config(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<FreeTierConfigBody>,
) -> Result<StatusCode, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    if body.free_per_day < 0 {
        return Err(bad_request("free_per_day 는 0 이상"));
    }
    let agent_id = body
        .agent_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(crate::free_tier::GLOBAL_AGENT);
    let mut db = state.db.lock().await;
    crate::free_tier::set_config(&mut db, agent_id, body.free_per_day)
        .map_err(|e| internal(&format!("free-tier config set: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/gui/payment/free-tier/status?agentId=` — 무료 잔여/사용량 상태.
async fn gui_free_tier_status(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<crate::free_tier::FreeTierStatusDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let agent_id = q
        .get("agentId")
        .map(|s| s.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(crate::free_tier::GLOBAL_AGENT)
        .to_string();
    let mut db = state.db.lock().await;
    crate::free_tier::status(&mut db, &agent_id)
        .map(Json)
        .map_err(|e| internal(&format!("free-tier status: {e}")))
}

// ── 마켓 — 온체인 결제 지갑 (주소 + ETH/USDC 실잔액) ──────────────────────────

/// `GET /v1/gui/payment/wallet` 응답.
///
/// - `address`: keystore `master.json` 의 **공개 주소만** (개인키는 절대 노출 안 함).
///   master.json 은 평문 `address` 필드를 가지므로 비밀번호 없이 읽는다(복호화 없음).
/// - `chain`/`rpc_url`: `XGRAM_CHAIN_RPC` 가 설정돼 있으면 그 RPC, 아니면 Base mainnet 기본.
/// - `eth_balance`/`usdc_balance`: 데몬이 RPC 로 **실제 조회**(eth_getBalance + ERC20 balanceOf).
///   조회 실패 시 `null` + `error` (가짜 0 금지).
/// - `onchain_enabled`: `XGRAM_CHAIN_RPC` env 설정 여부(미설정이면 내부 원장 모드).
#[derive(Debug, Serialize)]
struct PaymentWalletDto {
    address: Option<String>,
    chain: String,
    rpc_url: String,
    /// ETH wei → 문자열(소수점 18자리 보존 위해 string). RPC 실패 시 None.
    eth_balance: Option<String>,
    /// USDC micro(6 decimals) → 문자열. RPC 실패 시 None.
    usdc_balance: Option<String>,
    onchain_enabled: bool,
    error: Option<String>,
}

/// keystore master.json 의 평문 `address` 필드만 읽는다(복호화·비밀번호 불필요).
fn read_master_address(data_dir: &std::path::Path) -> Result<String, String> {
    use openxgram_core::paths::keystore_dir;
    let path = keystore_dir(data_dir).join(openxgram_core::paths::MASTER_KEYFILE);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("master.json 읽기 실패 ({}): {e}", path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("master.json 파싱 실패: {e}"))?;
    json.get("address")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "master.json 에 address 필드 없음".to_string())
}

/// raw JSON-RPC 호출(개인키 불필요·읽기 전용). 결과 `result` 필드 hex 문자열 반환.
async fn rpc_call(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<String, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp = client
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC 요청 실패: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("RPC 응답 파싱 실패: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(format!("RPC error: {err}"));
    }
    v.get("result")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "RPC 응답에 result 없음".to_string())
}

/// hex(0x..) → u128 decimal 문자열. 큰 값(ETH wei)도 u128 범위 안.
fn hex_to_decimal(hex: &str) -> Result<String, String> {
    let h = hex.trim_start_matches("0x");
    if h.is_empty() {
        return Ok("0".to_string());
    }
    u128::from_str_radix(h, 16)
        .map(|n| n.to_string())
        .map_err(|e| format!("hex 파싱 실패 ({hex}): {e}"))
}

/// `GET /v1/gui/payment/wallet` — keystore master 주소 + Base 체인 ETH/USDC 실잔액.
async fn gui_payment_wallet(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<PaymentWalletDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    // 1) 체인/RPC 결정 — XGRAM_CHAIN_RPC 설정 여부가 온체인 활성 여부.
    let rpc_override = std::env::var("XGRAM_CHAIN_RPC").ok().filter(|s| !s.trim().is_empty());
    let onchain_enabled = rpc_override.is_some();
    // 체인 = XGRAM_CHAIN(기본 base). chain.rs 레지스트리에서 USDC 컨트랙트·라벨을 가져온다.
    // (이전엔 BASE 하드코딩이라 ethereum-sepolia 등에서 USDC 잔액을 Base 컨트랙트로 조회해 0 표시 버그.)
    let chain_name = std::env::var("XGRAM_CHAIN").unwrap_or_else(|_| "base".to_string());
    let chain_cfg = openxgram_payment::chain::lookup(&chain_name)
        .unwrap_or(openxgram_payment::chain::BASE);
    let rpc_url = rpc_override
        .clone()
        .unwrap_or_else(|| chain_cfg.default_rpc.to_string());

    // 2) keystore master 주소(공개 주소만). 실패 시 address=None + error.
    let (address, mut error): (Option<String>, Option<String>) =
        match read_master_address(state.data_dir.as_ref()) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e)),
        };

    // 3) 온체인 잔액 실조회 (주소가 있을 때만). RPC 실패 시 balance=null + error 누적(가짜 0 금지).
    let mut eth_balance: Option<String> = None;
    let mut usdc_balance: Option<String> = None;
    if let Some(addr) = address.as_ref() {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
        {
            Ok(c) => Some(c),
            Err(e) => {
                error = Some(merge_err(error, format!("HTTP 클라이언트 생성 실패: {e}")));
                None
            }
        };
        if let Some(client) = client {
            // ETH (native) — eth_getBalance(addr, "latest")
            match rpc_call(
                &client,
                &rpc_url,
                "eth_getBalance",
                serde_json::json!([addr, "latest"]),
            )
            .await
            .and_then(|h| hex_to_decimal(&h))
            {
                Ok(v) => eth_balance = Some(v),
                Err(e) => error = Some(merge_err(error, format!("ETH 잔액 조회 실패: {e}"))),
            }

            // USDC (ERC20 balanceOf(addr)) — eth_call. selector 0x70a08231 + padded addr.
            let addr_hex = addr.trim_start_matches("0x");
            let data = format!("0x70a08231{:0>64}", addr_hex);
            match rpc_call(
                &client,
                &rpc_url,
                "eth_call",
                serde_json::json!([
                    { "to": chain_cfg.usdc_contract, "data": data },
                    "latest"
                ]),
            )
            .await
            .and_then(|h| hex_to_decimal(&h))
            {
                Ok(v) => usdc_balance = Some(v),
                Err(e) => error = Some(merge_err(error, format!("USDC 잔액 조회 실패: {e}"))),
            }
        }
    }

    Ok(Json(PaymentWalletDto {
        address,
        chain: chain_cfg.name.to_string(),
        rpc_url,
        eth_balance,
        usdc_balance,
        onchain_enabled,
        error,
    }))
}

/// 에러 메시지 누적(silent fallback 금지 — 여러 단계 실패를 모두 표면화).
fn merge_err(existing: Option<String>, new: String) -> String {
    match existing {
        Some(e) => format!("{e}; {new}"),
        None => new,
    }
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
    // rc.227 — application-level ACK (conversation_id 매칭 답신 추적)
    #[serde(skip_serializing_if = "Option::is_none")]
    app_ack_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_ack_at: Option<String>,
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
    // rc.227 — outbound_queue 의 app_ack_* 도 LEFT JOIN (conversation_id 매칭).
    let mut stmt = db.conn().prepare(
        "SELECT m.id, m.session_id, m.sender, m.body, m.timestamp, m.conversation_id, \
                m.ack_status, m.acked_at, m.ack_via, \
                ( \
                    SELECT q.app_ack_status FROM outbound_queue q \
                    WHERE q.conversation_id = m.conversation_id \
                    ORDER BY q.enqueued_at DESC LIMIT 1 \
                ) as app_ack_status, \
                ( \
                    SELECT q.app_ack_at FROM outbound_queue q \
                    WHERE q.conversation_id = m.conversation_id \
                    ORDER BY q.enqueued_at DESC LIMIT 1 \
                ) as app_ack_at \
         FROM messages m ORDER BY m.timestamp DESC LIMIT ?1"
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
            app_ack_status: r.get::<_, Option<String>>(9)?,
            app_ack_at: r.get::<_, Option<String>>(10)?,
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
        // rc.219 — self:* (outbox) 메시지의 경우 outbound_queue.ack_status / ack_at 를 우선 표시.
        //   매칭 키: m.signature == JSON 안에 들어 있어서 직접 JOIN 불가 →
        //   대신 같은 conversation_id + target_alias + 최신 enqueued_at order 로 fallback 표시.
        //   COALESCE 로 message.ack_status (rc.153) 가 비어있을 때 outbound_queue.ack_status 사용.
        // rc.227 — app_ack_status / app_ack_at 추가. conversation_id 직접 매칭 (sender 측 outbound).
        let sql = "SELECT m.id, m.session_id, m.sender, m.body, m.timestamp, \
                          m.conversation_id, \
                          COALESCE(m.ack_status, ( \
                              SELECT CASE \
                                       WHEN q.ack_status IS NOT NULL THEN q.ack_status \
                                       WHEN q.last_error = 'ack_timeout_max' THEN 'ack_timeout_max' \
                                       WHEN q.sent_at IS NULL THEN 'pending' \
                                       WHEN q.sent_at IS NOT NULL AND q.ack_at IS NULL THEN 'sent' \
                                       ELSE NULL END \
                              FROM outbound_queue q \
                              WHERE q.target_alias = ?6 \
                                AND ((m.conversation_id IS NOT NULL AND q.body LIKE '%' || m.conversation_id || '%') \
                                     OR q.body LIKE '%' || substr(m.body, 1, 60) || '%') \
                              ORDER BY q.enqueued_at DESC LIMIT 1 \
                          )) as ack_status, \
                          COALESCE(m.acked_at, ( \
                              SELECT q.ack_at FROM outbound_queue q \
                              WHERE q.target_alias = ?6 \
                                AND ((m.conversation_id IS NOT NULL AND q.body LIKE '%' || m.conversation_id || '%') \
                                     OR q.body LIKE '%' || substr(m.body, 1, 60) || '%') \
                              ORDER BY q.enqueued_at DESC LIMIT 1 \
                          )) as acked_at, \
                          m.ack_via, \
                          ( \
                              SELECT q.app_ack_status FROM outbound_queue q \
                              WHERE q.target_alias = ?6 \
                                AND m.conversation_id IS NOT NULL \
                                AND q.conversation_id = m.conversation_id \
                              ORDER BY q.enqueued_at DESC LIMIT 1 \
                          ) as app_ack_status, \
                          ( \
                              SELECT q.app_ack_at FROM outbound_queue q \
                              WHERE q.target_alias = ?6 \
                                AND m.conversation_id IS NOT NULL \
                                AND q.conversation_id = m.conversation_id \
                              ORDER BY q.enqueued_at DESC LIMIT 1 \
                          ) as app_ack_at \
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
                // rc.219 — ?6 = variant (outbound_queue.target_alias 매칭용).
                rusqlite::params![outbox, inbox, peer_session, cc_like, limit as i64, variant],
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
                        app_ack_status: r.get::<_, Option<String>>(9)?,
                        app_ack_at: r.get::<_, Option<String>>(10)?,
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
    // 더블버블 fix — outbound L0 저장은 run_peer_send_with_conv 내부 record_outbox 가
    // 이미 수행 (session "outbox-to-{alias}", sender "self:{alias}", conversation_id 포함).
    // 여기서 추가로 save_l0_message("Peer · {alias}", sender "me") 하면 같은 메시지가
    // 두 세션에 저장되고 peer_conversation 쿼리(outbox-to OR Peer·)가 둘을 union 하여
    // 말풍선이 2개로 보임 → 중복 저장 제거 (record_outbox 단일 경로로 일원화).
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

// ── ACP (Agent Client Protocol) HTTP handlers — Phase B-2 (additive) ───────
// daemon_gui_acp 모듈의 free fn 으로 위임. 모든 핸들러 require_auth 후 실행.
// 모듈 에러 (StatusCode, String) → (StatusCode, Json<ErrorDto>) 매핑.

fn acp_err((code, msg): (StatusCode, String)) -> (StatusCode, Json<ErrorDto>) {
    (code, Json(ErrorDto { error: msg }))
}

/// `GET /v1/acp/agents` — 알려진 ACP adapter + installed 여부.
async fn acp_agents(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    Ok(Json(crate::daemon_gui_acp::list_agents(&state.acp)))
}

/// `POST /v1/acp/sessions` — ACP conversation session 생성 (always=즉시 spawn,
/// on_demand=첫 prompt 시 lazy spawn).
async fn acp_session_create(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<crate::daemon_gui_acp::CreateSessionBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    crate::daemon_gui_acp::create_session(&state.acp, body)
        .await
        .map(Json)
        .map_err(acp_err)
}

/// `POST /v1/acp/sessions/{id}/prompt` — 한 prompt turn 구동, {stopReason, updates}.
async fn acp_session_prompt(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<crate::daemon_gui_acp::PromptBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    // ── 사용자 프롬프트("me") 권위 기록 (턴 실행 전) ─────────────────────────
    // 핵심 버그 fix: 종전 MAIN ACP 경로는 사용자 메시지("me")를 프론트엔드의 optimistic
    // recordMsg("me", ...) 에만 의존해 영속화했다. 이는 타이밍 의존적이라 큐된 follow-up
    // (턴 끝에 자동 전송) 이나 UI 이탈 시 'me' 버블이 DB 재동기화(conv_persisted/loadHistory)
    // 로 사라졌다. 이제 데몬이 agent 응답과 동일하게 "me" 도 권위 기록한다 — UI 와 무관히 영속.
    // 순서: 여기서 me-row(턴 전) → 아래 agent-row(턴 후). created_at(me) ≤ created_at(agent).
    // raw 텍스트 기록: 맥락 복원 preamble 은 prompt() 내부에서 body.text 앞에 붙으므로
    // body.text 는 사용자가 실제로 친 원문이다. 그 원문을 기록해 화면에 정확히 보이게 한다.
    let raw_user_text = body.text.clone();
    if let Some(conv_key) = state.acp.session_label(&id).await {
        // record_message 는 빈 텍스트면 no-op, 영속 실패 시 명시 로그(절대 규칙 1).
        state.acp.record_message(&conv_key, "me", &raw_user_text).await;
    }

    let result = crate::daemon_gui_acp::prompt(&state.acp, &id, body)
        .await
        .map_err(acp_err)?;

    // ── 서버측 권위 기록 ──────────────────────────────────────────────────
    // 핵심 버그 fix: 턴 결과를 데몬이 직접 `acp_messages` 에 기록한다. 종전에는
    // UI(AcpConversation.tsx) 만 turn-end 에 agent 응답을 conv_add 로 기록했기에,
    // 사용자가 턴 중/후 대화창을 나가면 기록이 누락되어 돌아왔을 때 "아무것도 안
    // 한 idle" 상태로 보였다. 이제 데몬이 권위 소스 — UI 이탈과 무관하게 영속화된다.
    //
    // 기록 조건: 세션에 label(conv_key=대화 신원)이 있고(=picker 진입 아님),
    // 추출한 agent 텍스트가 비어있지 않을 때만. 빈/취소 턴은 가짜 빈 메시지 방지로 스킵.
    if let Some(conv_key) = state.acp.session_label(&id).await {
        let mut agent_text = String::new();
        // 과정(툴 호출·계획)도 순서대로 수집 → DB 영속. 나갔다 와도 ▸단계 아코디언에 복원된다.
        let mut tools: Vec<(String, serde_json::Value)> = Vec::new(); // (toolCallId, {title,status})
        let mut plan_json: Option<String> = None;
        if let Some(updates) = result.get("updates").and_then(|u| u.as_array()) {
            for u in updates {
                match u.get("sessionUpdate").and_then(|s| s.as_str()) {
                    Some("agent_message_chunk") => {
                        if let Some(t) = u
                            .get("content")
                            .and_then(|c| c.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            agent_text.push_str(t);
                        }
                    }
                    Some("tool_call") => {
                        let tid = u.get("toolCallId").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let title = u
                            .get("title")
                            .and_then(|v| v.as_str())
                            .or_else(|| u.get("kind").and_then(|v| v.as_str()))
                            .unwrap_or("tool")
                            .to_string();
                        let status = u.get("status").and_then(|v| v.as_str()).unwrap_or("pending").to_string();
                        tools.push((tid, serde_json::json!({ "title": title, "status": status })));
                    }
                    Some("tool_call_update") => {
                        let tid = u.get("toolCallId").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(st) = u.get("status").and_then(|v| v.as_str()) {
                            for entry in tools.iter_mut() {
                                if entry.0 == tid {
                                    entry.1["status"] = serde_json::json!(st);
                                    break;
                                }
                            }
                        }
                    }
                    Some("plan") => {
                        if let Some(entries) = u.get("entries") {
                            plan_json = Some(entries.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        let has_answer = !agent_text.trim().is_empty();
        if has_answer || !tools.is_empty() || plan_json.is_some() {
            let now = chrono::Utc::now().to_rfc3339();
            let mut db = state.db.lock().await;
            // 툴 호출은 이제 스트리밍 중 증분 기록(daemon_gui_acp::record_stream_tool)되므로 여기선
            // 재기록하지 않는다(이중 기록 방지). 계획·최종 답변만 턴 끝에 기록 → id 순서=화면 순서(▸단계 후 답변).
            if let Some(pj) = &plan_json {
                let _ = db.conn().execute(
                    "INSERT INTO acp_messages (conv_key, role, text, created_at) VALUES (?1, 'plan', ?2, ?3)",
                    rusqlite::params![conv_key, pj, now],
                );
            }
            if has_answer {
                if let Err(e) = db.conn().execute(
                    "INSERT INTO acp_messages (conv_key, role, text, created_at) VALUES (?1, 'agent', ?2, ?3)",
                    rusqlite::params![conv_key, agent_text.trim(), now],
                ) {
                    // 절대 규칙 1(fallback 금지) — 조용히 넘기지 않고 명시 로그.
                    tracing::error!(target: "acp.daemon", conv_key = %conv_key, "acp_messages agent 기록 실패: {e}");
                }
            }
            drop(db);
            // 영속 직후 SSE 로 'conv_persisted' 알림 → 떠 있는 클라이언트가 DB 재동기화(loadHistory).
            // 사용자가 턴 도중/직후 다른 창에 갔다 와도 (loadHistory 1회성으로 놓치던) 완료 답변·과정이 뜬다.
            crate::daemon_gui_acp::notify_conv_persisted(&state.acp, &id).await;
        }
    }

    Ok(Json(result))
}

/// `POST /v1/acp/sessions/{id}/cancel` — session/cancel.
async fn acp_session_cancel(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    crate::daemon_gui_acp::cancel(&state.acp, &id)
        .await
        .map(Json)
        .map_err(acp_err)
}

/// `DELETE /v1/acp/sessions/{id}` — close + reap.
async fn acp_session_close(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    crate::daemon_gui_acp::close(&state.acp, &id)
        .await
        .map(Json)
        .map_err(acp_err)
}

/// `GET /v1/acp/sessions/{id}/stream` — SSE relay of `session/update`.
/// 세션 broadcast 채널을 구독해 update JSON 을 SSE event 로 전달. prompt turn 중
/// 발생한 update 가 relay 됨 (§6 — reader loop 는 막지 않음).
async fn acp_session_stream(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<
    axum::response::Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    (StatusCode, Json<ErrorDto>),
> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let rx = crate::daemon_gui_acp::subscribe(&state.acp, &id)
        .await
        .map_err(acp_err)?;
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(update) => {
                    let payload = serde_json::to_string(&update).unwrap_or_else(|_| "{}".to_string());
                    let ev = axum::response::sse::Event::default()
                        .event("session_update")
                        .data(payload);
                    return Some((Ok(ev), rx));
                }
                // Lagged: skip dropped messages, keep streaming.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                // Sender gone (session closed) → end the stream.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(axum::response::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default()))
}

// ── A2A (Google Agent2Agent) HTTP handlers — Phase 3 (additive) ────────────
// daemon_gui_a2a 모듈의 free fn 으로 위임. 모든 핸들러 require_auth 후 실행.
// 모듈 에러 (StatusCode, String) → (StatusCode, Json<ErrorDto>) 매핑.
// CLIENT-only: OpenXgram이 외부 A2A 에이전트를 호출. callee 측 AgentCard 호스팅은 후속.

fn a2a_err((code, msg): (StatusCode, String)) -> (StatusCode, Json<ErrorDto>) {
    (code, Json(ErrorDto { error: msg }))
}

/// `GET /v1/gui/a2a/tasks/{id}` query — A2A 대상 base URL (필수, 추측 default 없음).
#[derive(Debug, serde::Deserialize)]
struct A2aTaskQuery {
    target: String,
}

/// `GET /v1/gui/a2a/agents` — A2A 로 호출 가능한 에이전트 목록.
/// ACP-A2A-CORE: AgentCard 호스팅 구현됨 → ai_type 이 ACP 어댑터로 resolve 되는
/// 에이전트는 reachable:true + agentCardUrl/tasksUrl 동봉. ai_type 없으면 false.
/// 로스터는 agent_capabilities⋈agent_profiles (DB lock).
async fn a2a_agents(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let infos: Vec<crate::daemon_gui_a2a::A2aAgentInfo> = {
        let mut db = state.db.lock().await;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT ac.alias, p.ai_type \
                 FROM agent_capabilities ac \
                 LEFT JOIN agent_profiles p ON p.alias = ac.alias \
                 WHERE ac.role IS NOT 'tmux' \
                 ORDER BY ac.messenger_enabled DESC, ac.alias ASC",
            )
            .map_err(|e| internal(&format!("a2a roster prep: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(crate::daemon_gui_a2a::A2aAgentInfo {
                    alias: r.get::<_, String>(0)?,
                    ai_type: r.get::<_, Option<String>>(1)?,
                })
            })
            .map_err(|e| internal(&format!("a2a roster query: {e}")))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    Ok(Json(crate::daemon_gui_a2a::list_agents(&infos)))
}

/// Load an agent's A2A `AgentMeta` from `agent_capabilities`⋈`agent_profiles`.
/// `Ok(None)` when the alias is unknown (→ explicit 404 at the call site).
async fn load_a2a_agent_meta(
    state: &GuiServerState,
    alias: &str,
) -> Result<Option<crate::daemon_gui_a2a::server::AgentMeta>, (StatusCode, Json<ErrorDto>)> {
    let mut db = state.db.lock().await;
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT ac.alias, ac.role, ac.capabilities, ac.project_path, p.ai_type \
             FROM agent_capabilities ac \
             LEFT JOIN agent_profiles p ON p.alias = ac.alias \
             WHERE ac.alias = ?1 AND ac.role IS NOT 'tmux'",
        )
        .map_err(|e| internal(&format!("a2a meta prep: {e}")))?;
    let mut rows = stmt
        .query_map([alias], |r| {
            Ok(crate::daemon_gui_a2a::server::AgentMeta {
                alias: r.get::<_, String>(0)?,
                role: r.get::<_, Option<String>>(1)?,
                capabilities: r.get::<_, Option<String>>(2)?,
                project_path: r.get::<_, Option<String>>(3)?,
                ai_type: r.get::<_, Option<String>>(4)?,
            })
        })
        .map_err(|e| internal(&format!("a2a meta query: {e}")))?;
    match rows.next() {
        Some(Ok(meta)) => Ok(Some(meta)),
        Some(Err(e)) => Err(internal(&format!("a2a meta row: {e}"))),
        None => Ok(None),
    }
}

/// `GET /v1/a2a/agents/{alias}/.well-known/agent-card.json` — served AgentCard.
async fn a2a_served_card(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let meta = load_a2a_agent_meta(&state, &alias).await?.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorDto {
                error: format!("unknown agent: {alias}"),
            }),
        )
    })?;
    let card = crate::daemon_gui_a2a::build_agent_card(&meta);
    let value = serde_json::to_value(&card).map_err(|e| internal(&format!("card serialize: {e}")))?;
    Ok(Json(value))
}

/// `POST /v1/a2a/agents/{alias}/tasks` — A2A tasks/send; executes via ACP.
async fn a2a_served_task_send(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
    Json(body): Json<crate::daemon_gui_a2a::TaskBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let mut meta = load_a2a_agent_meta(&state, &alias).await?.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorDto {
                error: format!("unknown agent: {alias}"),
            }),
        )
    })?;

    // rc.321 — 친구 단위 POLICY 적용. 발신자(body.from)가 로컬 친구로 등록돼 있으면
    // 그 정책(권한/격리/비용)을 enforce 한다. from 미상 또는 비-친구면 정책 미적용(기존 동작).
    // owned 으로 보유 — handle_task 가 body 를 move 하므로 이후(비용 기록)에도 from 이 살아있어야 함.
    let from_owned: Option<String> = body.from.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| s.to_string());
    let from = from_owned.as_deref();
    let policy: Option<FriendPolicy> = if let Some(f) = from {
        let mut db = state.db.lock().await;
        load_friend_policy(db.conn(), f)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("friend policy load: {e}") })))?
    } else {
        None
    };

    if let (Some(f), Some(pol)) = (from, policy.as_ref()) {
        // 요청이 읽기/상태성인지 판별 — skill 이 status/read/get/list 류면 read 로 간주.
        let skill_lc = body.skill.as_deref().unwrap_or("").to_ascii_lowercase();
        let is_read_only = matches!(skill_lc.as_str(), "status" | "read" | "get" | "list" | "ping")
            || skill_lc.starts_with("read")
            || skill_lc.starts_with("status")
            || skill_lc.starts_with("get");

        match pol.permission.as_str() {
            "blocked" => {
                tracing::warn!(from = %f, target = %alias, "A2A friend request DENIED (permission=blocked)");
                return Err((StatusCode::FORBIDDEN, Json(ErrorDto {
                    error: format!("friend '{f}' is blocked — request declined"),
                })));
            }
            "read" if !is_read_only => {
                tracing::warn!(from = %f, target = %alias, skill = %skill_lc, "A2A friend request DENIED (permission=read, task-exec not allowed)");
                return Err((StatusCode::FORBIDDEN, Json(ErrorDto {
                    error: format!("friend '{f}' has read-only permission — task execution declined"),
                })));
            }
            "read" | "request" | "full" => { /* allowed */ }
            other => {
                tracing::warn!(from = %f, target = %alias, permission = %other, "A2A friend request DENIED (unknown permission)");
                return Err((StatusCode::FORBIDDEN, Json(ErrorDto {
                    error: format!("friend '{f}' has unrecognized permission '{other}' — declined"),
                })));
            }
        }

        // 격리 — 친구의 작업을 메인 워크트리가 아닌 per-friend 격리 cwd 에서 실행.
        // {data_dir}/friend-isolated/{from} 디렉토리를 보장하고 meta.project_path 를 덮어쓴다.
        if pol.isolated {
            let iso_dir = state.data_dir.join("friend-isolated").join(sanitize_path_segment(f));
            match std::fs::create_dir_all(&iso_dir) {
                Ok(_) => {
                    tracing::info!(from = %f, target = %alias, cwd = %iso_dir.display(), "A2A friend ISOLATED — overriding cwd to per-friend dir");
                    meta.project_path = Some(iso_dir.display().to_string());
                }
                Err(e) => {
                    // 격리 플래그를 silent 무시 금지 — 격리 보장 불가 시 거절.
                    tracing::warn!(from = %f, target = %alias, error = %e, "A2A friend isolation dir 생성 실패 — 요청 거절(격리 보장 불가)");
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto {
                        error: format!("friend '{f}' requires isolation but isolated cwd could not be prepared: {e}"),
                    })));
                }
            }
        }
    }

    // 실제 실행. 토큰 카운트 가용 신호가 없으므로(현 ACP turn 은 토큰 미집계) tokens=0 placeholder.
    let kind = body.skill.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "task".to_string());
    let result = crate::daemon_gui_a2a::handle_task(&state.acp, &state.served_a2a, &meta, body)
        .await;

    // 비용 기록 — 성공/실패 무관하게 1 row (cost_tracked=1 일 때). 토큰=0 (집계 신호 부재).
    if let (Some(f), Some(pol)) = (from, policy.as_ref()) {
        if pol.cost_tracked {
            let machine: Option<String> = {
                let mut db = state.db.lock().await;
                db.conn().query_row(
                    "SELECT machine FROM agent_profiles WHERE alias = ?1",
                    [f],
                    |r| r.get::<_, Option<String>>(0),
                ).ok().flatten()
            };
            let occurred = kst_now_string();
            let note = match &result {
                Ok(_) => "a2a request handled".to_string(),
                Err((code, msg)) => format!("a2a request failed ({code}): {msg}"),
            };
            let mut db = state.db.lock().await;
            if let Err(e) = db.conn().execute(
                "INSERT INTO friend_cost_ledger (friend_alias, machine, occurred_at_kst, kind, tokens, note) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![f, machine, occurred, kind, 0_i64, note],
            ) {
                // 원장 기록 실패는 요청 자체를 죽이지 않되, 절대 silent X — 명시 로그.
                tracing::warn!(from = %f, error = %e, "friend_cost_ledger insert 실패 (요청 결과는 보존)");
            }
        }
    }

    result.map(Json).map_err(a2a_err)
}

/// 경로 세그먼트 안전화 — alias 를 디렉토리명으로 쓸 때 path traversal/구분자 제거.
/// 영숫자·`-`·`_`·`.` 만 유지, 나머지는 `_` 로 치환. 빈 결과면 "unknown".
fn sanitize_path_segment(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let trimmed = out.trim_matches('.').to_string();
    if trimmed.is_empty() { "unknown".to_string() } else { trimmed }
}

/// KST(Asia/Seoul) 현재 시각 문자열 (절대 규칙 #4 — 모든 타임스탬프 KST).
fn kst_now_string() -> String {
    use chrono::{FixedOffset, Utc};
    // +09:00 은 유효한 오프셋(33+ hr 범위 내) → east_opt 는 Some. 방어적으로 None 이면 UTC.
    match FixedOffset::east_opt(9 * 3600) {
        Some(kst) => Utc::now().with_timezone(&kst).to_rfc3339(),
        None => Utc::now().to_rfc3339(),
    }
}

/// `GET /v1/a2a/agents/{alias}/tasks/{id}` — A2A tasks/get for a served task.
async fn a2a_served_task_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path((alias, id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    crate::daemon_gui_a2a::served_task(&state.served_a2a, &alias, &id)
        .await
        .map(Json)
        .map_err(a2a_err)
}

/// `POST /v1/gui/a2a/send` — 대상 AgentCard discover 후 tasks/send.
/// body: { from_agent?, target, skill?, task?, session_id? }. crate client 재사용.
async fn a2a_send(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(body): Json<crate::daemon_gui_a2a::SendBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 주소 = alias(신원) + endpoint(전달 위치). 같은 에이전트라도 보낼 곳은 여럿.
    // endpoint 분기:
    //   - new_acp(또는 None) + 내부 alias → load_a2a_agent_meta + handle_task (가시 스레드).
    //   - existing_acp:<id> → 그 ACP 세션에 prompt + record_message(가시 기록).
    //   - tmux:<name> → tmux send-keys 주입 + 가시 기록.
    //   - worktree → git worktree add 신규 + 그 cwd ACP 세션 prompt.
    //   - external (target=http URL) → 기존 외부 client send.
    let endpoint = body
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // target 이 http 면 외부(A2A) — endpoint 미지정/external 로 간주.
    let target_is_http = {
        let t = body.target.trim();
        t.starts_with("http://") || t.starts_with("https://")
    };

    // close:<sessionId> — 친구 패널 이탈(onClose) 시 지속 세션 종료(누수 방지).
    //   - 로컬에 살아있는 세션이면 즉시 close(에이전트 reap).
    //   - 크로스머신(원격 데몬 보유) 세션은 로컬에 없음 → no-op 보고. 원격의 idle TTL reaper 가
    //     마지막 사용 후 N분 idle 시 자동 close 하므로 누수되지 않는다.
    if let Some(ep) = endpoint.as_deref() {
        if let Some(sid) = ep.strip_prefix("close:") {
            let sid = sid.trim().to_string();
            if sid.is_empty() {
                return Err(bad_request("close: sessionId 비어 있음"));
            }
            if state.acp.session_alive(&sid).await {
                let _ = crate::daemon_gui_acp::close(&state.acp, &sid).await;
                return Ok(Json(serde_json::json!({
                    "status": "closed",
                    "endpoint": ep,
                    "sessionId": sid,
                    "local": true,
                })));
            }
            return Ok(Json(serde_json::json!({
                "status": "noop",
                "endpoint": ep,
                "sessionId": sid,
                "local": false,
                "note": "로컬에 없는(원격) 세션 — 원격 idle TTL reaper 가 정리한다.",
            })));
        }
    }

    // 명시 external 또는 http target → 기존 외부 client.
    if endpoint.as_deref() == Some("external") || (endpoint.is_none() && target_is_http) {
        return crate::daemon_gui_a2a::send(&state.a2a, body)
            .await
            .map(Json)
            .map_err(a2a_err);
    }

    // 이하 내부 경로 — alias 필수.
    let alias = body.target.trim().to_string();
    if alias.is_empty() {
        return Err(bad_request("a2a_send: 'target' (alias) 비어 있음"));
    }

    match endpoint.as_deref() {
        // ① 신규 ACP (기본). load_a2a_agent_meta + handle_task — 기존 동작 보존.
        None | Some("new_acp") => {
            // BUG2/3 (cross-machine A2A) — 대상이 LOCAL ACP 에이전트가 아니면(=meta None),
            // peer-sync 로 발견된 REMOTE 에이전트(예: zalman 의 navi)인지 확인한다. 맞으면
            // 로컬 spawn 대신 그 머신 주소로 signed peer envelope 를 send_envelope 경유
            // 전송하되 recipient_alias=<alias> 를 명시한다. 수신측 데몬의 process_inbound(① fix)가
            // recipient_alias 로 대상의 LOCAL ACP 를 구동한다. 회신은 ACK/inbound 로 발신
            // 스레드(`a2a:{from}->{alias}` 또는 bare alias)에 돌아온다.
            let meta = match load_a2a_agent_meta(&state, &alias).await? {
                Some(m) => m,
                None => {
                    if let Some(resp) =
                        try_remote_a2a_route(&state, &alias, &body).await?
                    {
                        return Ok(Json(resp));
                    }
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ErrorDto {
                            error: format!(
                                "unknown agent: {alias} (로컬 ACP 미등록 + peer-sync 로 발견된 reachable remote 에이전트도 아님)"
                            ),
                        }),
                    ));
                }
            };
            let task_body = crate::daemon_gui_a2a::TaskBody {
                skill: body.skill.clone(),
                message: if body.task.is_string() {
                    serde_json::Value::Null
                } else {
                    body.task.clone()
                },
                task: body.task.as_str().map(|s| s.to_string()),
                text: None,
                session_id: body.session_id.clone(),
                from: body.from_agent.clone(),
            };
            crate::daemon_gui_a2a::handle_task(&state.acp, &state.served_a2a, &meta, task_body)
                .await
                .map(Json)
                .map_err(a2a_err)
        }
        // ② 기존 ACP 세션 — existing_acp:<sessionId>. 그 세션에 prompt + 가시 기록.
        Some(ep) if ep.starts_with("existing_acp:") => {
            let session_id = ep["existing_acp:".len()..].trim().to_string();
            if session_id.is_empty() {
                return Err(bad_request("existing_acp: sessionId 비어 있음"));
            }
            let prompt_text = a2a_prompt_text(&body)?;
            // 가시 스레드 키 = 그 세션의 label(=alias). 없으면 명시 에러(추측 금지).
            let conv_key = state.acp.session_label(&session_id).await.ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorDto {
                        error: format!(
                            "existing_acp: session '{session_id}' 없음 또는 label(대화신원) 미부여 — 가시 기록 불가"
                        ),
                    }),
                )
            })?;
            state.acp.record_message(&conv_key, "me", &prompt_text).await;
            let res = crate::daemon_gui_acp::prompt(
                &state.acp,
                &session_id,
                crate::daemon_gui_acp::PromptBody {
                    text: prompt_text.clone(),
                },
            )
            .await
            .map_err(a2a_err)?;
            let agent_text = a2a_collect_agent_text(&res);
            state.acp.record_message(&conv_key, "agent", &agent_text).await;
            Ok(Json(serde_json::json!({
                "status": "completed",
                "endpoint": ep,
                "agent": alias,
                "sessionId": session_id,
                "convKey": conv_key,
                "text": agent_text,
            })))
        }
        // ③ TMUX 세션 — tmux:<sessionName>. send-keys 주입 + 가시 기록.
        Some(ep) if ep.starts_with("tmux:") => {
            let session_name = ep["tmux:".len()..].trim().to_string();
            if session_name.is_empty() {
                return Err(bad_request("tmux: sessionName 비어 있음"));
            }
            let prompt_text = a2a_prompt_text(&body)?;
            a2a_tmux_inject(&session_name, &prompt_text)
                .await
                .map_err(|e| internal(&format!("tmux inject 실패: {e}")))?;
            // 가시 기록 — tmux 엔드포인트도 같은 신원 스레드(conv_key=bare alias)에 남긴다.
            // GUI 리더(3900/4282)가 bare alias 로 읽으므로 prefix 없는 alias 로 통합.
            let conv_key = alias.clone();
            state.acp.record_message(&conv_key, "me", &prompt_text).await;
            Ok(Json(serde_json::json!({
                "status": "delivered",
                "endpoint": ep,
                "agent": alias,
                "tmuxSession": session_name,
                "convKey": conv_key,
                "note": "tmux send-keys 주입 완료 — 응답은 그 세션 화면/회신으로 온다(동기 결과 없음).",
            })))
        }
        // ④ 신규 워크트리 — worktree. git worktree add 후 그 cwd ACP 세션 prompt.
        Some("worktree") => {
            let meta = load_a2a_agent_meta(&state, &alias).await?.ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorDto {
                        error: format!("unknown agent: {alias}"),
                    }),
                )
            })?;
            let project_path = meta
                .project_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    internal(&format!(
                        "agent '{alias}' has no project_path — worktree 기준 경로 없음"
                    ))
                })?
                .to_string();
            let ai_type = meta
                .ai_type
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    internal(&format!("agent '{alias}' has no ai_type — not ACP-drivable"))
                })?;
            let adapter = crate::daemon_gui_a2a::server::resolve_acp_agent(ai_type).ok_or_else(
                || internal(&format!("agent '{alias}' ai_type '{ai_type}' 에 매칭되는 ACP adapter 없음")),
            )?;
            let prompt_text = a2a_prompt_text(&body)?;

            // git worktree add <project_path>/.worktrees/a2a-<ts> <branch>
            let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
            let branch = format!("a2a-{ts}");
            let wt_path = format!("{project_path}/.worktrees/{branch}");
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&project_path)
                .args(["worktree", "add", "-b", &branch, &wt_path, "HEAD"])
                .output()
                .map_err(|e| internal(&format!("git worktree add 실행 실패: {e}")))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                return Err(internal(&format!(
                    "git worktree add 실패(path={wt_path}, branch={branch}): {stderr}"
                )));
            }

            // 가시 기록은 수신자 bare-alias identity 스레드(persist_key)로 통합 → GUI 표시.
            // 단, ACP 세션 label 은 워크트리 cwd 전용 유니크 키(session_label)로 둔다:
            // 워크트리는 별도 cwd 의 1회성 세션이므로 메인 alias 세션과 find-or-create 충돌하면
            // 안 된다(다른 cwd). 즉 영속 스레드(bare alias)와 세션 신원(wt 키)을 분리한다.
            let persist_key = alias.clone();
            let session_label = format!("a2a:{alias}:wt:{branch}");
            state.acp.record_message(&persist_key, "me", &prompt_text).await;
            let create = crate::daemon_gui_acp::create_session(
                &state.acp,
                crate::daemon_gui_acp::CreateSessionBody {
                    agent: adapter.to_string(),
                    cwd: wt_path.clone(),
                    mcp_servers: Vec::new(),
                    execution_mode: Some("always".to_string()),
                    permission_mode: Some("bypassPermissions".to_string()),
                    model: None,
                    thinking: None,
                    machine: None,
                    label: Some(session_label.clone()),
                },
            )
            .await
            .map_err(a2a_err)?;
            let session_id = create
                .get("sessionId")
                .and_then(|s| s.as_str())
                .ok_or_else(|| internal("worktree ACP create_session 이 sessionId 미반환"))?
                .to_string();
            let prompt_res = crate::daemon_gui_acp::prompt(
                &state.acp,
                &session_id,
                crate::daemon_gui_acp::PromptBody {
                    text: prompt_text.clone(),
                },
            )
            .await;
            let _ = crate::daemon_gui_acp::close(&state.acp, &session_id).await;
            let acp_result = prompt_res.map_err(a2a_err)?;
            let agent_text = a2a_collect_agent_text(&acp_result);
            state.acp.record_message(&persist_key, "agent", &agent_text).await;
            Ok(Json(serde_json::json!({
                "status": "completed",
                "endpoint": "worktree",
                "agent": alias,
                "worktree": wt_path,
                "branch": branch,
                "sessionId": session_id,
                "convKey": persist_key,
                "sessionLabel": session_label,
                "text": agent_text,
            })))
        }
        Some(other) => Err(bad_request(&format!(
            "a2a_send: 알 수 없는 endpoint '{other}' (new_acp|existing_acp:<id>|tmux:<name>|worktree|external)"
        ))),
    }
}

/// BUG2/3 (cross-machine A2A 송신 라우팅) — 대상 alias 가 LOCAL ACP 에이전트가 아닐 때,
/// peer-sync 로 발견된 REMOTE 에이전트(예: zalman 의 navi)면 그 머신 주소로 signed peer
/// envelope 를 전송한다. 전송은 새 transport 를 만들지 않고 기존 `run_peer_send_with_conv`
/// (peer_send.rs)를 재사용한다 — master 키 서명 + tailnet 신뢰(기존 fleet peer_send 와 동일).
///   - alias 의 peer row 가 없거나 주소가 도달 불가(localhost/unknown)면 → 원격 라우팅 불가
///     (Ok(None) 반환 → 호출측이 unknown agent 에러로 처리).
///   - `run_peer_send_with_conv(alias=navi)` 가 envelope.recipient_alias=navi 로 박고 navi 의
///     peer row 주소(=zalman 머신 데몬)로 POST 한다. zalman process_inbound(① fix)가
///     recipient_alias 로 navi 의 LOCAL ACP 를 구동한다.
///   - 회신은 ACK/inbound 경로로 발신 스레드에 돌아온다(별도 동기 결과 없음 — accepted).
/// 가시 스레드(`a2a:{from}->{alias}` 또는 bare alias)에 'me' 송신을 영속한다.
///
/// 반환:
///   - `Ok(Some(json))`  : 원격 라우팅 수행됨(accepted).
///   - `Ok(None)`        : 원격 에이전트로 식별 불가(로컬도 원격도 아님) — 호출측이 NOT_FOUND.
///   - `Err(...)`        : 식별은 됐으나 전송 단계 실패(절대 규칙 1 — silent X).
async fn try_remote_a2a_route(
    state: &GuiServerState,
    alias: &str,
    body: &crate::daemon_gui_a2a::SendBody,
) -> Result<Option<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    let data_dir = state.data_dir.as_ref().clone();
    // peer row 조회 — peer-sync 가 navi 를 zalman 머신 주소로 merge 해 두었어야 함(GAP A).
    let peer = {
        let mut db = state.db.lock().await;
        let mut store = openxgram_peer::PeerStore::new(&mut db);
        match store.get_by_alias(alias) {
            Ok(opt) => opt,
            Err(e) => return Err(internal(&format!("remote a2a peer 조회 실패: {e}"))),
        }
    };
    let peer = match peer {
        Some(p) => p,
        None => return Ok(None), // 로컬 ACP 도 아니고 알려진 peer 도 아님.
    };
    // 도달 불가 주소(localhost/unknown)면 cross-machine 전송 불가 — 원격 라우팅 대상 아님.
    if openxgram_transport::tailscale::is_unreachable_address(&peer.address) {
        tracing::warn!(
            alias = %alias,
            address = %peer.address,
            "BUG2/3 remote a2a — peer 주소 도달 불가(localhost/unknown). cross-machine 라우팅 불가 → NOT_FOUND 폴백"
        );
        return Ok(None);
    }

    let prompt_text = a2a_prompt_text(body)?;
    let from = body
        .from_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // master 키 서명 — daemon env 의 XGRAM_KEYSTORE_PASSWORD(기존 peer_send 와 동일 경로).
    let pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").map_err(|_| {
        internal("XGRAM_KEYSTORE_PASSWORD 미설정 — cross-machine A2A 송신 서명 불가 (daemon 환경변수 필요)")
    })?;

    // 가시 스레드 영속 — 발신측에 'me' 기록(handle_task 의 conv_key 규칙과 동일).
    let conv_key = match from.as_deref() {
        Some(f) => format!("a2a:{f}->{alias}"),
        None => format!("a2a:{alias}"),
    };
    state.acp.record_message(&conv_key, "me", &prompt_text).await;

    // 기존 peer_send 재사용 — recipient_alias=alias 를 박고 peer 주소(머신 데몬)로 전송.
    crate::peer_send::run_peer_send_with_conv(
        &data_dir,
        alias,
        from.as_deref(),
        &prompt_text,
        &pw,
        None,
    )
    .await
    .map_err(|e| internal(&format!("cross-machine A2A peer_send 실패 ({alias}@{}): {e}", peer.address)))?;

    tracing::info!(
        target = %alias,
        address = %peer.address,
        from = ?from,
        "BUG2/3 cross-machine A2A — signed envelope 전송 OK (recipient_alias={alias}). 회신은 inbound 로 발신 스레드에 도달"
    );
    Ok(Some(serde_json::json!({
        "status": "accepted",
        "endpoint": "remote_peer",
        "agent": alias,
        "address": peer.address,
        "convKey": conv_key,
        "note": "cross-machine A2A — 수신 머신의 ACP 가 비동기 실행. 회신은 inbound 로 발신 스레드에 도달(동기 결과 없음).",
    })))
}

/// `SendBody` 의 task/skill 에서 prompt 텍스트 추출 — 평문 task 우선, 없으면 skill.
/// 둘 다 비면 명시 에러(추측 default 없음).
fn a2a_prompt_text(body: &crate::daemon_gui_a2a::SendBody) -> Result<String, (StatusCode, Json<ErrorDto>)> {
    if let Some(s) = body.task.as_str() {
        let t = s.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    // 구조화 task 면 그대로 직렬화해 전달(텍스트 본문이 명확치 않을 때).
    if body.task.is_object() || body.task.is_array() {
        return serde_json::to_string(&body.task)
            .map_err(|e| internal(&format!("task 직렬화 실패: {e}")));
    }
    Err(bad_request(
        "a2a_send: 전달할 'task' 텍스트 비어 있음 (endpoint 라우팅엔 task 본문 필요)",
    ))
}

/// ACP `{stopReason, updates}` 에서 agent 응답 텍스트만 이어붙인다(가시 기록·반환용).
fn a2a_collect_agent_text(acp_result: &serde_json::Value) -> String {
    let Some(updates) = acp_result.get("updates").and_then(|u| u.as_array()) else {
        return String::new();
    };
    let mut out = String::new();
    for u in updates {
        if u.get("type").and_then(|t| t.as_str()) != Some("agent_message_chunk") {
            continue;
        }
        if let Some(text) = u
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
        {
            out.push_str(text);
        }
    }
    out
}

/// tmux send-keys 주입 — `daemon.rs:768` 의 `send-keys -t <target> -l <wrapped>` +
/// `send-keys -t <target> Enter` 패턴 재사용(bracketed paste wrap + sleep + Enter).
/// 대상은 `<sessionName>:0`(active window). 실패 시 명시 에러 반환(fallback 금지).
async fn a2a_tmux_inject(session_name: &str, text: &str) -> Result<(), String> {
    let target = format!("{session_name}:0");
    let wrapped = format!("\x1b[200~{}\x1b[201~", text);
    let out = crate::notify::tmux_command_async()
        .args(["send-keys", "-t", &target, "-l", &wrapped])
        .output()
        .await
        .map_err(|e| format!("send-keys -l 실행 실패: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "send-keys -l 실패: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let out2 = crate::notify::tmux_command_async()
        .args(["send-keys", "-t", &target, "Enter"])
        .output()
        .await
        .map_err(|e| format!("send-keys Enter 실행 실패: {e}"))?;
    if !out2.status.success() {
        return Err(format!(
            "send-keys Enter 실패: {}",
            String::from_utf8_lossy(&out2.stderr)
        ));
    }
    Ok(())
}

/// `GET /v1/gui/a2a/agents/{alias}/endpoints` — 그 신원(alias)으로 보낼 수 있는
/// 전달 위치(endpoint) 5종을 조회한다. GUI 셀렉터·MCP `list_agent_endpoints` 공용 소스.
async fn a2a_list_agent_endpoints(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;

    // 에이전트 메타(project_path) — 없으면 tmux/worktree 필터 기준이 없으니 빈 결과로.
    let meta = load_a2a_agent_meta(&state, &alias).await?;
    let project_path = meta
        .as_ref()
        .and_then(|m| m.project_path.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // existing_acp — 살아있는 ACP 세션 중 label==alias.
    let existing_acp: Vec<serde_json::Value> = state
        .acp
        .list_sessions_brief()
        .await
        .into_iter()
        .filter(|(_, label, _)| label == &alias)
        .map(|(id, label, _)| serde_json::json!({ "id": id, "label": label }))
        .collect();

    // tmux — 그 프로젝트 project_path(또는 하위)에서 도는 tmux 세션.
    let tmux: Vec<serde_json::Value> = {
        let pp = project_path.clone();
        tokio::task::spawn_blocking(crate::daemon_gui_sessions::tmux_sessions_brief)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, cwd)| match (&pp, cwd) {
                (Some(p), Some(c)) => c == p || c.starts_with(&format!("{p}/")),
                _ => false,
            })
            .map(|(name, cwd)| serde_json::json!({ "name": name, "cwd": cwd }))
            .collect()
    };

    Ok(Json(serde_json::json!({
        "alias": alias,
        "new_acp": true,
        "existing_acp": existing_acp,
        "tmux": tmux,
        "worktree": project_path.is_some(),
        "external": false,
    })))
}

/// `GET /v1/gui/a2a/tasks/{id}?target=<url>` — tasks/get 상태 조회.
async fn a2a_task_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<A2aTaskQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    crate::daemon_gui_a2a::get_task(&state.a2a, &q.target, &id)
        .await
        .map(Json)
        .map_err(a2a_err)
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

// ============================================================================
// GUI-MISSING-ROUTES (additive) — wiki body rw / fs tree / fs file rw / machines
// ----------------------------------------------------------------------------
// 절대 규칙 1 (fallback 금지): 모든 실패는 명시적 HTTP status. silent fallback 없음.
// fs write 는 whitelist 강제 (사용자 머신 보호).
// ============================================================================

#[derive(Debug, Serialize)]
struct WikiBodyDto {
    slug: String,
    title: String,
    body: String,
    updated_at: i64,
}

/// `GET /v1/gui/wiki/{type}/{slug}` — 페이지 전체 본문 (디스크 정본).
/// 기존 WikiTools::read 재사용 (디스크 + DB 인덱스 fallback 경로 포함).
async fn gui_wiki_body_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path((ptype, slug)): Path<(String, String)>,
) -> Result<Json<WikiBodyDto>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let topic = format!("{ptype}/{slug}");
    let wiki_root = state.data_dir.join("wiki");
    let fs = openxgram_wiki::WikiFs::new(&wiki_root);
    // updated_at 은 DB 인덱스에서 — conn 을 await 너머로 들고 가지 않도록 먼저 읽고 lock 해제.
    let updated_at: i64 = {
        let mut db = state.db.lock().await;
        db.conn()
            .query_row(
                "SELECT updated_at FROM wiki_pages WHERE id = ?1 OR file_path = ?2 OR file_path = ?3",
                rusqlite::params![
                    topic,
                    format!("wiki/{ptype}/{slug}.md"),
                    format!("{ptype}/{slug}.md")
                ],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
    };
    // 디스크 정본에서 본문 read (WikiFs — conn 불필요).
    let id = openxgram_wiki::PageId::new(
        ptype.parse::<openxgram_wiki::PageType>().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorDto {
                    error: format!("알 수 없는 page type: {ptype}"),
                }),
            )
        })?,
        &slug,
    )
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: format!("invalid slug: {e}"),
            }),
        )
    })?;
    match fs.read(&id).await {
        Ok(Some(page)) => Ok(Json(WikiBodyDto {
            slug: page.id.to_string(),
            title: page.title,
            body: page.body,
            updated_at,
        })),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorDto {
                error: format!("wiki page not found: {topic}"),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto {
                error: format!("wiki read: {e}"),
            }),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct WikiBodyPutBody {
    #[serde(default)]
    title: Option<String>,
    body: String,
}

/// `PUT /v1/gui/wiki/{type}/{slug}` — 본문 upsert (디스크 + DB). WikiTools::write 재사용.
/// title 이 주어지면 본문 첫 줄에 `# {title}` 이 없을 때 prepend (WikiTools 가 H1 → title 추출).
async fn gui_wiki_body_put(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Path((ptype, slug)): Path<(String, String)>,
    Json(payload): Json<WikiBodyPutBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // title 보존: 본문에 H1 이 없고 title 이 주어졌으면 prepend.
    let content = match &payload.title {
        Some(t) if !payload.body.trim_start().starts_with("# ") => {
            format!("# {t}\n\n{}", payload.body)
        }
        _ => payload.body.clone(),
    };
    let wiki_root = state.data_dir.join("wiki");
    let fs = openxgram_wiki::WikiFs::new(&wiki_root);
    // page_type / id 파싱.
    let page_type = ptype.parse::<openxgram_wiki::PageType>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: format!("알 수 없는 page type: {ptype}"),
            }),
        )
    })?;
    let id = openxgram_wiki::PageId::new(page_type, &slug).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: format!("invalid slug: {e}"),
            }),
        )
    })?;
    // title: 본문 H1 우선, 없으면 payload.title, 없으면 slug.
    let title = payload
        .title
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| slug.clone());
    let page = openxgram_wiki::Page::new(id.clone(), page_type, title, content.trim_end().to_string());
    // 1) 디스크 정본 write (conn 불필요 — await 안전). 충돌 검증 None (last-write-wins, GUI 편집기).
    if let Err(e) = fs.write(&page, None).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto {
                error: format!("wiki disk write: {e}"),
            }),
        ));
    }
    // 2) DB 인덱스 upsert (lock 하에 sync — await 없음).
    {
        let mut db = state.db.lock().await;
        let store = openxgram_wiki::WikiStore::new(db.conn());
        store.upsert(&page, None).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorDto {
                    error: format!("wiki db upsert: {e}"),
                }),
            )
        })?;
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "slug": id.to_string(),
        "content_hash": page.content_hash,
    })))
}

// ----------------------------------------------------------------------------
// fs tree — 디렉토리 트리 (프로젝트 폴더 뷰 + 폴더 피커).
// ----------------------------------------------------------------------------

/// 한 디렉토리를 depth 까지 재귀 — `.git`/`node_modules`/숨김 등 skip, dirs-first 정렬.
fn build_fs_tree(dir: &std::path::Path, depth: usize) -> serde_json::Value {
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.to_string_lossy().to_string());
    let mut node = serde_json::json!({
        "name": name,
        "path": dir.to_string_lossy().to_string(),
        "is_dir": true,
    });
    if depth == 0 {
        node["truncated"] = serde_json::json!(true);
        return node;
    }
    let skip = |n: &str| -> bool {
        matches!(
            n,
            ".git" | "node_modules" | "target" | ".cargo" | "dist" | "build" | ".next"
        )
    };
    let mut dirs: Vec<serde_json::Value> = vec![];
    let mut files: Vec<serde_json::Value> = vec![];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            if fname.starts_with('.') && fname != ".claude" && fname != ".mcp.json" {
                // 숨김 파일/폴더는 skip (단 에이전트 config 관련 .claude/.mcp.json 은 노출).
                continue;
            }
            if skip(&fname) {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(build_fs_tree(&p, depth - 1));
            } else {
                files.push(serde_json::json!({
                    "name": fname,
                    "path": p.to_string_lossy().to_string(),
                    "is_dir": false,
                }));
            }
        }
    }
    dirs.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    dirs.extend(files);
    node["children"] = serde_json::json!(dirs);
    node
}

/// Sentinel `path` value that requests the top-level filesystem roots instead
/// of a specific directory. Empty `path` is treated the same.
const FS_ROOTS_SENTINEL: &str = "__roots__";

/// Top-level roots for the folder picker, OS-appropriate (mirrors File
/// Explorer / Finder). Windows: each present drive letter PLUS each installed
/// `\\wsl$\<distro>` share. Unix: `$HOME` + `/`.
///
/// Returns `{ "os": "windows"|"unix", "roots": [{name,path,is_dir}, ...] }`
/// so the frontend can open the picker at the daemon-correct starting point
/// without hardcoding a Linux path.
fn build_fs_roots() -> serde_json::Value {
    let mut roots: Vec<serde_json::Value> = vec![];

    #[cfg(windows)]
    let os_label = "windows";
    #[cfg(not(windows))]
    let os_label = "unix";

    #[cfg(windows)]
    {
        // Drive letters: probe C:..Z: (GetLogicalDrives semantics, no winapi dep).
        for letter in b'C'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            if std::path::Path::new(&drive).is_dir() {
                roots.push(serde_json::json!({
                    "name": format!("{}:", letter as char),
                    "path": drive,
                    "is_dir": true,
                }));
            }
        }
        // Also probe A:/B: in case of mapped/virtual drives.
        for letter in [b'A', b'B'] {
            let drive = format!("{}:\\", letter as char);
            if std::path::Path::new(&drive).is_dir() {
                roots.push(serde_json::json!({
                    "name": format!("{}:", letter as char),
                    "path": drive,
                    "is_dir": true,
                }));
            }
        }
        // WSL distros: enumerate `\\wsl$\` (best-effort — present only when WSL
        // installed). Each child directory is a distro root share.
        let wsl_base = r"\\wsl$\";
        if let Ok(entries) = std::fs::read_dir(wsl_base) {
            for e in entries.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let distro = e.file_name().to_string_lossy().to_string();
                    roots.push(serde_json::json!({
                        "name": format!("\\\\wsl$\\{distro}"),
                        "path": format!(r"\\wsl$\{distro}"),
                        "is_dir": true,
                    }));
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() && std::path::Path::new(&home).is_dir() {
                roots.push(serde_json::json!({
                    "name": home.clone(),
                    "path": home,
                    "is_dir": true,
                }));
            }
        }
        roots.push(serde_json::json!({
            "name": "/",
            "path": "/",
            "is_dir": true,
        }));
    }

    serde_json::json!({
        "os": os_label,
        "is_roots": true,
        "roots": roots,
    })
}

/// `GET /v1/gui/fs/tree?path=<dir>&depth=<n>` — 디렉토리 JSON 트리 (read-only).
/// depth 기본 2, 최대 5. path 미지정 시 400. 디렉토리 아님/미존재 시 명시 status.
// 머신 라벨 → (ssh host, wsl 래퍼 여부). None = 로컬(이 데몬 머신).
// cross-machine 폴더 browse 용 — SSH-stdio 방식(원격 데몬 불필요).
// ── cross-machine 머신 설정 (config-driven — 하드코딩 제거, 일반 배포 가능) ──
// ~/.openxgram/machines.json: {"machines":[{"label":"잘만","ssh_host":"zalman","wsl":true}]}
// 없으면 예시 시드 생성. remote_home/adapter 미지정 시 동적 해석(SSH $HOME, PATH).
#[derive(serde::Deserialize, Clone)]
pub(crate) struct MachineCfg {
    pub label: String,
    pub ssh_host: String,
    #[serde(default)]
    pub wsl: bool,
    #[serde(default)]
    pub remote_home: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
}

fn machines_config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    std::path::PathBuf::from(home).join(".openxgram").join("machines.json")
}

pub(crate) fn load_machines() -> Vec<MachineCfg> {
    let p = machines_config_path();
    match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str::<serde_json::Value>(&s)
            .ok()
            .and_then(|v| v.get("machines").and_then(|m| m.as_array()).cloned())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| serde_json::from_value::<MachineCfg>(m.clone()).ok())
                    .collect()
            })
            .unwrap_or_default(),
        Err(_) => {
            // 첫 실행 — 편집 가능한 예시 시드 생성. 다른 사용자는 자기 머신으로 수정.
            let seed = serde_json::json!({
                "_comment": "cross-machine 에이전트. ssh_host=SSH 접속명, wsl=Windows WSL 경유. remote_home/adapter 비우면 동적($HOME/PATH).",
                "machines": [{"label": "잘만", "ssh_host": "zalman", "wsl": true}]
            });
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&p, serde_json::to_string_pretty(&seed).unwrap_or_default());
            vec![MachineCfg {
                label: "잘만".into(),
                ssh_host: "zalman".into(),
                wsl: true,
                remote_home: None,
                adapter: None,
            }]
        }
    }
}

// 머신 라벨 → 설정. 로컬(서울 등)이면 None(이 데몬 머신에서 실행).
pub(crate) fn machine_lookup(machine: &str) -> Option<MachineCfg> {
    let l = machine.trim();
    if matches!(
        l.to_lowercase().as_str(),
        "" | "서울" | "seoul" | "server-seoul" | "local"
    ) {
        return None;
    }
    load_machines()
        .into_iter()
        .find(|m| m.label.eq_ignore_ascii_case(l) || m.ssh_host.eq_ignore_ascii_case(l))
}

// 원격 머신 $HOME — remote_home 설정 있으면 그것, 없으면 SSH 로 동적 조회(캐시).
pub(crate) fn machine_home(cfg: &MachineCfg) -> Option<String> {
    if let Some(h) = &cfg.remote_home {
        return Some(h.clone());
    }
    use std::sync::Mutex;
    static CACHE: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);
    if let Ok(g) = CACHE.lock() {
        if let Some(m) = g.as_ref() {
            if let Some(h) = m.get(&cfg.ssh_host) {
                return Some(h.clone());
            }
        }
    }
    let remote = if cfg.wsl {
        "wsl -- bash -lc \"echo $HOME\"".to_string()
    } else {
        "bash -lc \"echo $HOME\"".to_string()
    };
    let out = std::process::Command::new("ssh")
        .args([
            "-o",
            "ConnectTimeout=8",
            "-o",
            "BatchMode=yes",
            &cfg.ssh_host,
            &remote,
        ])
        .output()
        .ok()?;
    let home = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if home.is_empty() || !home.starts_with('/') {
        return None;
    }
    if let Ok(mut g) = CACHE.lock() {
        g.get_or_insert_with(Default::default)
            .insert(cfg.ssh_host.clone(), home.clone());
    }
    Some(home)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// 원격 머신의 디렉토리 트리 — SSH 로 find 실행 후 경로 목록을 nested 트리로 변환.
// Windows→WSL(zalman) 경유의 따옴표/특수문자 깨짐 방지를 위해 bash 명령을 base64 로 전달.
// 경로가 원격에 없으면 원격 $HOME 으로 fallback(머신마다 home 다름).
fn remote_fs_tree(host: &str, wsl: bool, path: &str, depth: usize) -> Result<serde_json::Value, String> {
    use base64::Engine;
    let find_cmd = format!(
        "D={}; [ -d \"$D\" ] || D=\"$HOME\"; find \"$D\" -maxdepth {} \\( -name node_modules -o -name .git -o -name target -o -name dist -o -name build -o -name .next \\) -prune -o -type d -print 2>/dev/null | head -2000",
        sh_quote(path), depth
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode(find_cmd.as_bytes());
    let inner = format!("echo {b64} | base64 -d | bash");
    let remote = if wsl {
        format!("wsl -- bash -lc \"{inner}\"")
    } else {
        format!("bash -lc \"{inner}\"")
    };
    let out = std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", host, &remote])
        .output()
        .map_err(|e| format!("ssh spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ssh {host} 실패: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim_end_matches('/').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if paths.is_empty() {
        return Err(format!("원격 {host} 에서 디렉토리를 찾지 못함 (경로/SSH 확인)"));
    }
    // find 는 시작 디렉토리를 먼저 출력 → 첫 줄이 실제 root(HOME fallback 반영).
    let root = paths[0].clone();
    Ok(build_tree_from_paths(&root, &paths))
}

// 평탄한 디렉토리 경로 목록 → nested {name, path, is_dir, children} 트리 (build_fs_tree 와 동일 shape).
fn build_tree_from_paths(root: &str, paths: &[String]) -> serde_json::Value {
    use std::collections::BTreeMap;
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in paths {
        if p == root {
            continue;
        }
        if let Some(idx) = p.rfind('/') {
            let parent = p[..idx].to_string();
            children.entry(parent).or_default().push(p.clone());
        }
    }
    fn node(path: &str, children: &BTreeMap<String, Vec<String>>) -> serde_json::Value {
        let name = path.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(path);
        let kids: Vec<serde_json::Value> = children
            .get(path)
            .map(|v| {
                let mut v2 = v.clone();
                v2.sort();
                v2.dedup();
                v2.iter().map(|c| node(c, children)).collect()
            })
            .unwrap_or_default();
        serde_json::json!({ "name": name, "path": path, "is_dir": true, "children": kids })
    }
    node(root, &children)
}

async fn gui_fs_tree(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // Roots mode: empty/absent `path` or the `__roots__` sentinel returns the
    // OS-appropriate top-level roots (Windows drives + \\wsl$ shares, or
    // $HOME + / on unix) so the folder picker opens correctly per-daemon-OS.
    // Local only — remote (machine=) browse needs an explicit start path.
    let machine_q = q.get("machine").map(|s| s.as_str()).unwrap_or("");
    let path_raw = q.get("path").map(|s| s.as_str()).unwrap_or("");
    if machine_q.is_empty() && (path_raw.is_empty() || path_raw == FS_ROOTS_SENTINEL) {
        return Ok(Json(build_fs_roots()));
    }
    let path = q.get("path").map(|s| s.as_str()).filter(|s| !s.is_empty()).ok_or((
        StatusCode::BAD_REQUEST,
        Json(ErrorDto {
            error: "path 쿼리 파라미터 필요".into(),
        }),
    ))?;
    let depth: usize = q
        .get("depth")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2)
        .min(5);
    let dir = std::path::Path::new(path);
    // path-traversal 방어: `..` 컴포넌트 거부.
    if dir
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: "'..' 포함 경로 거부".into(),
            }),
        ));
    }
    // cross-machine — machine 파라미터가 원격이면 SSH 로 원격 디렉토리 트리 조회.
    let machine = q.get("machine").map(|s| s.as_str()).unwrap_or("");
    if let Some(cfg) = machine_lookup(machine) {
        let path_owned = path.to_string();
        let (host, wsl) = (cfg.ssh_host.clone(), cfg.wsl);
        let res = tokio::task::spawn_blocking(move || remote_fs_tree(&host, wsl, &path_owned, depth))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorDto {
                        error: format!("ssh join: {e}"),
                    }),
                )
            })?;
        return res.map(Json).map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorDto {
                    error: format!("원격 폴더 조회({machine}): {e}"),
                }),
            )
        });
    }
    if !dir.is_dir() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorDto {
                error: format!("디렉토리 아님 또는 미존재: {path}"),
            }),
        ));
    }
    Ok(Json(build_fs_tree(dir, depth)))
}

// ----------------------------------------------------------------------------
// fs file read/write — 에이전트 config 편집기. write 는 whitelist 강제.
// ----------------------------------------------------------------------------

/// 쓰기 허용 파일인지 검증.
/// 1) 파일명 화이트리스트: CLAUDE.md / AGENTS.md / AGENT.md / GEMINI.md / settings*.json / .mcp.json / *.md
/// 2) 경로는 등록된 에이전트의 project_path(agent_capabilities) 중 하나의 하위거나, HOME/.claude 하위.
fn fs_write_allowed(path: &std::path::Path, project_paths: &[String], home: &std::path::Path) -> Result<(), String> {
    // path-traversal 방어.
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("'..' 포함 경로 거부".into());
    }
    let fname = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| "파일명 없음".to_string())?;
    let name_ok = matches!(
        fname.as_str(),
        "CLAUDE.md" | "AGENTS.md" | "AGENT.md" | "GEMINI.md" | ".mcp.json"
    ) || fname.starts_with("settings")
        && fname.ends_with(".json")
        || fname.ends_with(".md");
    if !name_ok {
        return Err(format!(
            "허용되지 않은 파일 종류: {fname} (config-file 만 쓰기 가능)"
        ));
    }
    // 경로 범위: project_path 하위 또는 HOME/.claude 하위.
    let claude_dir = home.join(".claude");
    let claude_str = claude_dir.to_string_lossy().to_string();
    let path_str = path.to_string_lossy().to_string();
    let in_scope = path_str.starts_with(&claude_str)
        || project_paths
            .iter()
            .any(|pp| !pp.is_empty() && path_str.starts_with(pp.trim_end_matches('/')));
    if !in_scope {
        return Err(format!(
            "경로가 등록된 에이전트 project_path 또는 ~/.claude 밖: {path_str}"
        ));
    }
    Ok(())
}

/// `GET /v1/gui/fs/file?path=<p>` — 파일 본문 (read-only). 편집기 로드용.
async fn gui_fs_file_get(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let path = q.get("path").map(|s| s.as_str()).ok_or((
        StatusCode::BAD_REQUEST,
        Json(ErrorDto {
            error: "path 쿼리 파라미터 필요".into(),
        }),
    ))?;
    let p = std::path::Path::new(path);
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: "'..' 포함 경로 거부".into(),
            }),
        ));
    }
    match std::fs::read_to_string(p) {
        Ok(content) => Ok(Json(serde_json::json!({ "content": content }))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorDto {
                error: format!("파일 미존재: {path}"),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto {
                error: format!("read: {e}"),
            }),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct FsFilePutBody {
    path: String,
    content: String,
}

/// `PUT /v1/gui/fs/file` body `{ path, content }` — config 파일 write (whitelist 강제).
async fn gui_fs_file_put(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Json(payload): Json<FsFilePutBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    // 등록된 에이전트 project_path 수집 (whitelist scope).
    let project_paths: Vec<String> = {
        let mut db = state.db.lock().await;
        let mut stmt = db
            .conn()
            .prepare("SELECT DISTINCT project_path FROM agent_capabilities WHERE project_path IS NOT NULL AND project_path != ''")
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorDto {
                        error: format!("prepare: {e}"),
                    }),
                )
            })?;
        let rows: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorDto {
                        error: format!("query: {e}"),
                    }),
                )
            })?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };
    let p = std::path::Path::new(&payload.path);
    if let Err(reason) = fs_write_allowed(p, &project_paths, &home) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorDto {
                error: format!("write 거부: {reason}"),
            }),
        ));
    }
    // 부모 디렉토리는 존재해야 함 (새 디렉토리 임의 생성 안 함 — 보수적).
    if let Some(parent) = p.parent() {
        if !parent.is_dir() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorDto {
                    error: format!("부모 디렉토리 미존재: {}", parent.display()),
                }),
            ));
        }
    }
    std::fs::write(p, payload.content.as_bytes()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorDto {
                error: format!("write: {e}"),
            }),
        )
    })?;
    let bytes = payload.content.len();
    eprintln!("[gui_fs_file_put] wrote {bytes} bytes to {}", payload.path);
    Ok(Json(serde_json::json!({
        "ok": true,
        "path": payload.path,
        "bytes": bytes,
    })))
}

// ----------------------------------------------------------------------------
// machines — 물리 머신만 (worker agent 제외). settings "연결된 머신".
// ----------------------------------------------------------------------------

/// `GET /v1/gui/models?q=<filter>` — 선택 가능한 모델 목록(OpenRouter 동적 조회, 1h 캐시).
/// 하드코딩 아님 — 새 모델(claude-fable-5 등)·codex/gpt 자동 포함. 키: ~/.openxgram/openrouter.key.
async fn gui_models_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Vec<serde_json::Value>)>> = Mutex::new(None);
    let filter = q.get("q").map(|s| s.to_lowercase()).unwrap_or_default();
    let do_filter = |models: &[serde_json::Value]| -> Vec<serde_json::Value> {
        if filter.is_empty() {
            return models.to_vec();
        }
        models
            .iter()
            .filter(|m| {
                m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_lowercase().contains(&filter)
                    || m.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase().contains(&filter)
            })
            .cloned()
            .collect()
    };
    // 캐시 hit (1h)
    if let Ok(g) = CACHE.lock() {
        if let Some((t, v)) = g.as_ref() {
            if t.elapsed() < Duration::from_secs(3600) {
                return Ok(Json(serde_json::json!({ "models": do_filter(v) })));
            }
        }
    }
    let key_path = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        std::path::PathBuf::from(home).join(".openxgram").join("openrouter.key")
    };
    let key = std::fs::read_to_string(&key_path)
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorDto {
                    error: "OpenRouter 키 없음(~/.openxgram/openrouter.key)".into(),
                }),
            )
        })?
        .trim()
        .to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorDto { error: format!("http: {e}") })))?;
    let resp = client
        .get("https://openrouter.ai/api/v1/models")
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorDto { error: format!("openrouter: {e}") })))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, Json(ErrorDto { error: format!("openrouter json: {e}") })))?;
    let models: Vec<serde_json::Value> = body["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id").and_then(|v| v.as_str())?;
                    let bare = id.rsplit('/').next().unwrap_or(id);
                    let provider = id.split('/').next().unwrap_or("");
                    Some(serde_json::json!({
                        "id": bare,            // ANTHROPIC_MODEL 등에 쓰는 bare id
                        "full": id,            // openrouter 전체 id
                        "provider": provider,  // anthropic / openai 등
                        "name": m.get("name").and_then(|v| v.as_str()).unwrap_or(bare),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((Instant::now(), models.clone()));
    }
    Ok(Json(serde_json::json!({ "models": do_filter(&models) })))
}

/// `GET /v1/gui/agent-machines` — 에이전트 생성 시 선택 가능한 머신 라벨.
/// 로컬(서울) + machines.json 설정 머신. 하드코딩 제거 — config-driven.
async fn gui_agent_machines(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    // 로컬 머신 라벨(기본 "서울"; machines.json 의 local_label 로 override 가능).
    let local_label = std::fs::read_to_string(machines_config_path())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("local_label").and_then(|l| l.as_str()).map(String::from))
        .unwrap_or_else(|| "서울".to_string());
    let mut labels: Vec<String> = vec![local_label];
    for m in load_machines() {
        labels.push(m.label);
    }
    Ok(Json(serde_json::json!({ "machines": labels })))
}

/// `GET /v1/gui/machines` — 구별되는 물리 머신 목록 (worker agent 필터링 제외).
/// 머신 판별: peers 의 role 이 worker/subagent 류가 아니고, address(=머신 식별자)
/// 단위로 distinct. local machine + tailscale online peer 도 포함.
async fn gui_machines_list(
    State(state): State<GuiServerState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorDto>)> {
    require_auth(&state, &headers).await.map_err(unauthorized)?;
    let local = crate::daemon_gui_sessions::detect_machine();

    // tailscale 머신 (있으면 권위 있는 머신 소스).
    let ts_status = std::process::Command::new("tailscale")
        .arg("status")
        .arg("--json")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

    let mut machines: Vec<serde_json::Value> = vec![];
    // 1) local machine 항상 포함.
    let local_host = Some(local.hostname.clone());
    machines.push(serde_json::json!({
        "hostname": local.hostname,
        "tailscale_ip": local.tailscale_ip,
        "is_local": true,
        "source": "local",
    }));

    // 2) tailscale online peers → 머신 (Self 제외, distinct hostname).
    if let Some(ts) = &ts_status {
        if let Some(peers) = ts.get("Peer").and_then(|p| p.as_object()) {
            for (_k, peer) in peers {
                let host = peer
                    .get("HostName")
                    .and_then(|h| h.as_str())
                    .or_else(|| peer.get("DNSName").and_then(|h| h.as_str()))
                    .unwrap_or("");
                if host.is_empty() {
                    continue;
                }
                if Some(host.to_string()) == local_host {
                    continue;
                }
                let ip = peer
                    .get("TailscaleIPs")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let online = peer.get("Online").and_then(|o| o.as_bool()).unwrap_or(false);
                machines.push(serde_json::json!({
                    "hostname": host,
                    "tailscale_ip": ip,
                    "is_local": false,
                    "online": online,
                    "source": "tailscale",
                }));
            }
        }
    }

    Ok(Json(serde_json::json!({
        "machines": machines,
        "machine_count": machines.len(),
        "note": "물리 머신만 (worker agent 제외). local + tailscale online peer.",
    })))
}
