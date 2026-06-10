// Web GUI — Tauri invoke() shim.
//
// 기존 컴포넌트는 `import { invoke} from "@tauri-apps/api/core"` 사용.
// Web 빌드에서는 `import { invoke} from "@/api/client"` 로 1줄 교체.
//
// daemon HTTP API 는 REST 스타일 (GET/POST/PUT/DELETE + 경로). Tauri 의
// `invoke(name, args)` (모두 POST + command name) 패턴과 다르므로 이 모듈에서
// command name → {method, path-template} 로 변환한다. 경로의 {id} 같은
// placeholder 는 args 에서 동일 키로 채운다.
//
// 라우팅 표는 daemon_gui.rs (Router::new()) 와 1:1 대응.
// daemon 에 없는 명령은 Error 던짐 (UI 가 에러 메시지 표시).

// daemon 이 직접 /gui/ 정적 자산 서빙하므로 same-origin /v1/gui/* 그대로 호출.
// nginx reverse proxy 있으면 그쪽도 /v1/gui/* pass-through.
// 다른 호스트의 daemon 사용 시 Settings 탭에서 절대 URL 입력.
const DEFAULT_BASE = "/v1/gui";
const LEGACY_BASE = "/api/gui"; // pre-rc.26 default — 자동 마이그레이션

const URL_KEY = "xgram_daemon_url";
const TOKEN_KEY = "xgram_mcp_token";

export function getDaemonUrl(): string {
 try {
 const stored = localStorage.getItem(URL_KEY);
 // rc.26 마이그레이션: 옛 default 가 저장돼 있으면 무시 → 새 default.
 if (!stored || stored === LEGACY_BASE) return DEFAULT_BASE;
 return stored;
} catch {
 return DEFAULT_BASE;
}
}

export function setDaemonUrl(url: string): void {
 try {
 if (url.trim()) {
 localStorage.setItem(URL_KEY, url.trim());
} else {
 localStorage.removeItem(URL_KEY);
}
} catch {
 // ignored — private mode
}
}

export function getBearer(): string | null {
 // 우선순위: session_token (웹 GUI unlock) > mcp_token (CLI 발급).
 // 두 키가 분리된 이유: unlock 토큰은 daemon 프로세스 수명, mcp-token 은 영구.
 // require_auth 핸들러는 둘 다 받음.
 try {
 return (
 localStorage.getItem("xgram_session_token") ||
 localStorage.getItem(TOKEN_KEY)
);
} catch {
 return null;
}
}

export function setBearer(token: string): void {
 try {
 if (token.trim()) {
 localStorage.setItem(TOKEN_KEY, token.trim());
} else {
 localStorage.removeItem(TOKEN_KEY);
}
} catch {
 // ignored
}
}

type HttpMethod = "GET" | "POST" | "PUT" | "DELETE" | "PATCH";

interface Route {
 method: HttpMethod;
 /** path 템플릿; `{id}` 등은 args 에서 같은 키로 치환. */
 path: string;
 /** path placeholder 채운 후 남는 args 키를 body 로 보낼지 (POST/PUT 기본 true). */
 body?: boolean;
 /** 응답 본문이 비어있으면 이 값을 반환 (기본 undefined). */
 emptyAs?: unknown;
}

// daemon_gui.rs Router::new() 에 정의된 엔드포인트와 1:1 매핑.
const ROUTES: Record<string, Route> = {
 // 기본 상태
 status: { method: "GET", path: "/status"},
 is_initialized: { method: "GET", path: "/initialized"},
 health: { method: "GET", path: "/health"},

 // Peers
 peers_list: { method: "GET", path: "/peers", emptyAs: []},
 peer_add: { method: "POST", path: "/peers", body: true},
 // rc.229 fix#3 — on-demand 단일 agent enrich (4-metadata + worktree/subagent/ex_peer)
 agent_detail: { method: "GET", path: "/agent/{alias}/detail"},
 // Phase 2-A — 동적 설정 탐지 (ai_type/path_hint 는 query 로 전송)
 agent_config_chain: { method: "GET", path: "/agent/{alias}/config-chain"},
 // 기능 배선 — wiki 본문 CRUD · fs 트리/파일 · 머신 목록 (daemon_gui.rs 신규)
 wiki_body_get: { method: "GET", path: "/wiki/{ptype}/{slug}"},
 wiki_body_put: { method: "PUT", path: "/wiki/{ptype}/{slug}", body: true},
 // ACP 대화 영속화 — 새로고침/재시작 후 복원.
 acp_conv_list: { method: "GET", path: "/acp/conversations/{key}/messages", emptyAs: []},
 acp_conv_add: { method: "POST", path: "/acp/conversations/{key}/messages", body: true},
 acp_conv_clear: { method: "DELETE", path: "/acp/conversations/{key}/messages"},
 fs_tree: { method: "GET", path: "/fs/tree"},
 fs_file_get: { method: "GET", path: "/fs/file"},
 fs_file_put: { method: "PUT", path: "/fs/file", body: true},
 machines_list: { method: "GET", path: "/machines", emptyAs: []},
 // Phase 3 — A2A(에이전트↔에이전트)
 a2a_agents: { method: "GET", path: "/a2a/agents", emptyAs: []},
 a2a_send: { method: "POST", path: "/a2a/send", body: true},
 a2a_task_get: { method: "GET", path: "/a2a/tasks/{id}"},
 // Phase 2-D — 에이전트 프로필 (classification/execution_mode/ai_type/worktree/public + folder/group/role)
 agent_profile_get: { method: "GET", path: "/agent/{alias}/profile"},
 agent_profile_set: { method: "POST", path: "/agent/{alias}/profile", body: true},
 // rc.245 — 결정적 세션 매핑 사용자 override (PATCH; body.session_identifier = string|null)
 peer_set_session: { method: "PATCH", path: "/peers/{alias}/session", body: true},

 // Messenger v1.3 §3.2 — 머신×세션 통합 detector (M-1)
 sessions: { method: "GET", path: "/sessions"},
 machine_info: { method: "GET", path: "/machine"},
 // Messenger v1.3 §4.3 (S5) — 세션 라이브 터미널 출력
 session_screen: { method: "GET", path: "/sessions/{identifier}/screen"},
 session_input: { method: "POST", path: "/sessions/{identifier}/input", body: true},
 session_aliases: { method: "GET", path: "/sessions/aliases"},
 memory_l0_list: { method: "GET", path: "/memory/l0"},
 memory_l0_save: { method: "POST", path: "/memory/l0", body: true},
 memory_stats: { method: "GET", path: "/memory/stats"},
 memory_extract_now: { method: "POST", path: "/memory/extract-now"},
 memory_import_scan_paths: { method: "GET", path: "/memory/import/scan-paths"},
 memory_import_desktop: { method: "POST", path: "/memory/import/desktop", body: true},
 memory_import_prompt: { method: "GET", path: "/memory/import/prompt-template"},
 memory_migration_import: { method: "POST", path: "/memory/migration/import", body: true},
 memory_import_bundle: { method: "POST", path: "/memory/import/bundle", body: true},
 memory_export_session: { method: "GET", path: "/memory/export/session/{session_id}"},
 memory_export_wiki: { method: "GET", path: "/memory/export/wiki/{id}"},
 memory_migration_export: { method: "GET", path: "/memory/migration/export/{session_id}"},
 memory_webhook_token: { method: "GET", path: "/memory/import/webhook-token"},
 memory_webhook_rotate: { method: "POST", path: "/memory/import/webhook-token"},
 session_alias_set: { method: "POST", path: "/sessions/{identifier}/alias", body: true},
 // Messenger v1.3 §7.1·§7.3 — 헤더 통합 승인 큐 (L6 + V4)
 approvals: { method: "GET", path: "/approvals"},
 // Messenger v1.3 §2.4 + M-3 + L4 — 마스터+서브 지갑 (HD 영구 점유)
 wallets_list: { method: "GET", path: "/wallets"},
 wallet_create: { method: "POST", path: "/wallets", body: true},
 wallet_topup: { method: "POST", path: "/wallets/topup", body: true},
 // Messenger v1.3 L3·V1 / M-5·N1·N3·V4 — Role 정책 + 화이트리스트
 role_policies: { method: "GET", path: "/role-policies"},
 role_policy_set: { method: "POST", path: "/role-policies", body: true},
 whitelist: { method: "GET", path: "/whitelist"},
 // Messenger v1.3 S8·V6 / N4 / V11 / V12 / N7
 cross_machine_queue: { method: "GET", path: "/cross-machine-queue"},
 global_search: { method: "GET", path: "/search"},
 routing_rules_list: { method: "GET", path: "/routing-rules"},
 routing_rule_add: { method: "POST", path: "/routing-rules", body: true},
 routing_rule_delete: { method: "POST", path: "/routing-rules/{id}"},
 version_info: { method: "GET", path: "/version"},
 system_cron_protect: { method: "POST", path: "/system-cron/protect-attempt", body: true},
 // S7 첨부, M-5 사용자 화이트리스트
 attachment_upload: { method: "POST", path: "/attachments", body: true},
 attachment_get: { method: "GET", path: "/attachments/{hash}"},
 whitelist_patterns_list: { method: "GET", path: "/whitelist-patterns"},
 whitelist_pattern_add: { method: "POST", path: "/whitelist-patterns", body: true},
 // UI-MEMORY-SPEC v1.1 — 위키 CRUD
 wiki_pages_list: { method: "GET", path: "/wiki/pages"},
 wiki_page_get: { method: "GET", path: "/wiki/pages/{id}"},
 wiki_page_upsert: { method: "POST", path: "/wiki/pages", body: true},
 // Memory deep
 wiki_delete: { method: "POST", path: "/wiki/pages/{id}/delete"},
 wiki_lock: { method: "POST", path: "/wiki/pages/{id}/lock", body: true},
 wiki_history: { method: "GET", path: "/wiki/pages/{id}/history"},
 wiki_share: { method: "POST", path: "/wiki/pages/{id}/share", body: true},
 wiki_trash_list: { method: "GET", path: "/wiki/trash"},
 wiki_trash_restore: { method: "POST", path: "/wiki/trash/{id}/restore"},
 memory_patterns_list: { method: "GET", path: "/memory/patterns"},
 memory_pattern_add: { method: "POST", path: "/memory/patterns", body: true},
 memory_mistakes_list: { method: "GET", path: "/memory/mistakes"},
 memory_mistake_add: { method: "POST", path: "/memory/mistakes", body: true},
 wiki_new_alerts: { method: "GET", path: "/wiki/new-alerts"},
 // Identity deep
 identity_info: { method: "GET", path: "/identity/info"},
 identity_audit: { method: "GET", path: "/identity/audit"},
 identity_allowlist: { method: "GET", path: "/identity/allowlist"},
 identity_allowlist_add: { method: "POST", path: "/identity/allowlist", body: true},
 identity_settings: { method: "POST", path: "/identity/settings", body: true},
 identity_suspicious_dids: { method: "GET", path: "/identity/suspicious_dids"},
 identity_suspicious_dismiss: { method: "POST", path: "/identity/suspicious_dismiss", body: true},
 // Channel deep
 channel_people: { method: "GET", path: "/channel/people"},
 channel_routing: { method: "GET", path: "/channel/routing"},
 // Autonomy deep
 autonomy_history: { method: "GET", path: "/autonomy/history"},
 autonomy_limits: { method: "GET", path: "/autonomy/limits"},
 autonomy_vacation: { method: "GET", path: "/autonomy/vacation"},
 autonomy_vacation_set: { method: "POST", path: "/autonomy/vacation", body: true},
 // External + Ops
 external_directory: { method: "GET", path: "/external/directory"},
 ops_health: { method: "GET", path: "/ops/health"},
 // 세션별 채널 바인딩 (메신저 §5 탭 3)
 session_bindings_list: { method: "GET", path: "/sessions/{agent_id}/channel-bindings"},
 session_binding_add: { method: "POST", path: "/sessions/{agent_id}/channel-bindings", body: true},
 session_binding_delete: { method: "POST", path: "/sessions/{agent_id}/channel-bindings/{binding_id}"},
 // rc.170 — auto-echo enforcer visual verification (각 binding 의 매칭 session + would_echo)
 bindings_status: { method: "GET", path: "/bindings_status"},
 // rc.122 — 에이전트 메신저 등록 (agent_capabilities CRUD, 외부 채널 바인딩과 별개)
 agents_list: { method: "GET", path: "/agents"},
 agents_register: { method: "POST", path: "/agents", body: true},
 agents_delete: { method: "POST", path: "/agents/{alias}"},
 agents_auto_detect: { method: "POST", path: "/agents/auto-detect", body: true},
 // rc.129 — 지침 파일 (cwd/AGENT.md) inline 편집
 agents_instructions_get: { method: "GET", path: "/agents/instructions"},
 agents_instructions_save: { method: "POST", path: "/agents/instructions", body: true},
 // rc.132 — agent_templates 카탈로그 (msitarzewski/agency-agents)
 agent_templates_list: { method: "GET", path: "/agent-templates"},
 agent_templates_refresh: { method: "POST", path: "/agent-templates/refresh"},
 agent_templates_apply: { method: "POST", path: "/agent-templates/apply", body: true},
 // rc.126 — 워크플로우 오케스트레이션 (UI-MESSENGER-SPEC §20 W-1~W-10, backend 기존)
 workflows_list: { method: "GET", path: "/workflows"},
 workflow_upsert: { method: "POST", path: "/workflows", body: true},
 workflow_get: { method: "GET", path: "/workflows/{id}"},
 workflow_delete: { method: "POST", path: "/workflows/{id}"},
 workflow_run: { method: "POST", path: "/workflows/{id}/run"},
 workflow_runs: { method: "GET", path: "/workflows/{id}/runs"},
 workflow_run_approve: { method: "POST", path: "/workflows/runs/{run_id}/approve"},
 // rc.279 — Paperclip 오케스트레이션 GUI (org agents + invoke). backend rc.276/277.
 orchestration_agents: { method: "GET", path: "/orchestration/agents", emptyAs: []},
 orchestration_issues: { method: "GET", path: "/orchestration/issues", emptyAs: []},
 orchestration_goals: { method: "GET", path: "/orchestration/goals", emptyAs: []},
 orchestration_add_from_peer: { method: "POST", path: "/orchestration/agents/add-from-peer", body: true},
 orchestration_agent_invoke: { method: "POST", path: "/orchestration/agents/{alias}/invoke", body: true},
 notify_discord_channels: { method: "POST", path: "/notify/discord/channels", body: true},
 notify_discord_diagnostic: { method: "GET", path: "/notify/discord/diagnostic"},
 ops_diagnostic: { method: "GET", path: "/ops/diagnostic"},
 ops_machines: { method: "GET", path: "/ops/machines"},
 ops_backup_status: { method: "GET", path: "/ops/backup-status"},
 ops_backup_now: { method: "POST", path: "/ops/backup-now"},
 ops_update_check: { method: "GET", path: "/ops/update-check"},
 ops_update_apply: { method: "POST", path: "/ops/update-apply"},
 external_outbound_calls: { method: "GET", path: "/external/outbound-calls"},
 external_inbound_pending: { method: "GET", path: "/external/inbound-pending"},
 external_inbound_approve: { method: "POST", path: "/external/inbound/{id}/approve", body: true},
 external_inbound_reject: { method: "POST", path: "/external/inbound/{id}/reject", body: true},
 external_my_listings: { method: "GET", path: "/external/my-listings"},
 external_listing_add: { method: "POST", path: "/external/listings", body: true},
 external_reputation: { method: "GET", path: "/external/reputation"},
 external_protocols: { method: "GET", path: "/external/protocols"},
 // Identity 깊은
 identity_bip39: { method: "POST", path: "/identity/bip39", body: true},
 identity_sub_dids: { method: "GET", path: "/identity/sub-dids"},
 identity_sub_did_new: { method: "POST", path: "/identity/sub-dids", body: true},
 identity_sub_did_revoke: { method: "POST", path: "/identity/sub-dids/{id}/revoke"},
 identity_lockout_status: { method: "GET", path: "/identity/lockout-status"},
 // Vault MCP
 vault_mcp_servers_list: { method: "GET", path: "/vault/mcp-servers"},
 vault_mcp_server_add: { method: "POST", path: "/vault/mcp-servers", body: true},
 vault_tool_catalog: { method: "GET", path: "/vault/tool-catalog"},
 vault_tool_acl_set: { method: "POST", path: "/vault/tool-catalog", body: true},
 // Channel 모더레이션
 channel_blocks_list: { method: "GET", path: "/channel/moderation/blocks"},
 channel_block_add: { method: "POST", path: "/channel/moderation/blocks", body: true},
 channel_limits_list: { method: "GET", path: "/channel/moderation/limits"},
 channel_limit_set: { method: "POST", path: "/channel/moderation/limits", body: true},
 // Autonomy SelfTrigger + Reflection
 self_triggers_list: { method: "GET", path: "/autonomy/self-triggers"},
 self_trigger_add: { method: "POST", path: "/autonomy/self-triggers", body: true},
 reflection_runs_list: { method: "GET", path: "/autonomy/reflection-runs"},
 reflection_now: { method: "POST", path: "/autonomy/reflection-runs"},
 // Memory M-2 merge + M-10 edit lock
 wiki_merge_candidates: { method: "GET", path: "/wiki/merge-candidates"},
 wiki_edit_lock_get: { method: "GET", path: "/wiki/pages/{id}/edit-lock"},
 wiki_edit_lock_acquire: { method: "POST", path: "/wiki/pages/{id}/edit-lock"},
 // Peer keypair generate
 peer_keypair_generate: { method: "POST", path: "/peers/generate-keypair", body: true},

 // Channel
 channel_status: { method: "GET", path: "/channel/status"},

 // Vault
 vault_pending_list: { method: "GET", path: "/vault/pending", emptyAs: []},
 vault_pending_approve: {
 method: "POST",
 path: "/vault/pending/{id}/approve",
},
 vault_pending_deny: {
 method: "POST",
 path: "/vault/pending/{id}/deny",
 body: true,
},

 // Payment limit
 payment_get_daily_limit: { method: "GET", path: "/payment/daily-limit"},
 payment_set_daily_limit: {
 method: "PUT",
 path: "/payment/daily-limit",
 body: true,
},

 // Notify
 notify_status: { method: "GET", path: "/notify/status"},
 notify_discord_validate: {
 method: "POST",
 path: "/notify/discord/validate",
 body: true,
},
 notify_discord_guilds: {
 method: "POST",
 path: "/notify/discord/guilds",
 body: true,
},
 notify_discord_save: {
 method: "POST",
 path: "/notify/discord/save",
 body: true,
},
 notify_telegram_validate: {
 method: "POST",
 path: "/notify/telegram/validate",
 body: true,
},
 notify_telegram_detect_chat_saved: { method: "POST", path: "/notify/telegram/detect_chat_saved"},
 notify_telegram_detect_chat: {
 method: "POST",
 path: "/notify/telegram/detect_chat",
 body: true,
},
 notify_telegram_save: {
 method: "POST",
 path: "/notify/telegram/save",
 body: true,
},
 // rc.91 — 채널 테스트 + 권한 진단 + 초대 URL
 notify_channel_test: { method: "POST", path: "/notify/channel/test", body: true},
 notify_discord_permissions: { method: "GET", path: "/notify/discord/permissions"},
 notify_discord_invite_url: { method: "GET", path: "/notify/discord/invite_url"},
 // rc.92 — 멀티 디스코드 봇
 discord_bots_list: { method: "GET", path: "/discord/bots", emptyAs: []},
 discord_bots_add: { method: "POST", path: "/discord/bots", body: true},
 discord_bots_delete: { method: "POST", path: "/discord/bots/{id}"},
 channels_summary: { method: "GET", path: "/channels/summary"},
 discord_bot_channels: { method: "GET", path: "/discord/bot/channels"},

 // Schedule
 schedule_list: { method: "GET", path: "/schedule", emptyAs: []},
 schedule_create: { method: "POST", path: "/schedule", body: true},
 schedule_stats: { method: "GET", path: "/schedule/stats"},
 schedule_cancel: { method: "POST", path: "/schedule/{id}/cancel"},

 // Chain
 chain_list: { method: "GET", path: "/chain", emptyAs: []},
 chain_delete: { method: "DELETE", path: "/chain/{name}"},
 // chain_show 는 컴포넌트에서 직접 호출 안 함 (chain_list 가 dto 다 줌).

 // 메신저 v1.3 Step 0 — 메시지 송수신
 messages_recent: { method: "GET", path: "/messages", emptyAs: []},
 // rc.212 — peer 와의 전 session (outbox/inbox/Peer·/Claude Code·) 통합 chronological view
 peer_conversation: { method: "GET", path: "/peer_conversation/{alias}", emptyAs: []},
 peer_send: { method: "POST", path: "/peers/{alias}/send", body: true},
 peer_send_unsigned: { method: "POST", path: "/peers/{alias}/send-unsigned", body: true},
 // rc.228 — ex Peer thread 삭제 (self_alias↔other_alias 의 outbox/inbox sessions + messages + outbound_queue).
 ex_peer_delete: { method: "DELETE", path: "/peer/{self_alias}/ex_peer/{other_alias}"},
 workflow_approve_run: { method: "POST", path: "/workflows/runs/{run_id}/approve", body: true},
};

// ── ACP (Agent Client Protocol, Phase B-3) ────────────────────────────────
//
// ACP 라우트는 `/v1/gui/*` 가 아니라 `/v1/acp/*` (daemon_gui.rs Router 참고).
// invoke() shim 은 `/v1/gui` base 에 고정돼 있어 재사용 못 하므로, gui base 에서
// `/v1/acp` base 를 파생하는 전용 helper 를 둔다. SSE 스트림도 EventSource 가
// Authorization 헤더를 못 실으므로 fetch + ReadableStream 으로 직접 구동한다.
// daemon 의 정확한 요청/응답 필드명(camelCase)은 daemon_gui_acp.rs 와 1:1 매칭.

/** gui base(`…/v1/gui`) → acp base(`…/v1/acp`) 파생. 미인식 형태면 `/v1/acp` 폴백. */
export function getAcpBase(): string {
  const gui = getDaemonUrl().replace(/\/+$/, "");
  if (gui.endsWith("/v1/gui")) return gui.slice(0, -"/gui".length) + "/acp";
  if (gui.endsWith("/gui")) return gui.slice(0, -"/gui".length) + "/acp";
  // 절대 URL 등 비표준 base — same-origin /v1/acp 로.
  return "/v1/acp";
}

function authHeaders(json: boolean): Record<string, string> {
  const h: Record<string, string> = {};
  const token = getBearer();
  if (token) h["Authorization"] = `Bearer ${token}`;
  if (json) h["Content-Type"] = "application/json";
  return h;
}

/** ACP REST 호출 (SSE 제외). 비-2xx → Error throw (조용한 폴백 없음 — 절대규칙 1). */
export async function acpFetch<T>(
  method: HttpMethod,
  path: string,
  body?: unknown,
): Promise<T> {
  const url = `${getAcpBase()}${path}`;
  let res: Response;
  try {
    res = await fetch(url, {
      method,
      headers: authHeaders(body !== undefined),
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
  } catch (e) {
    throw new Error(`daemon 연결 실패 (${url}): ${(e as Error).message}`);
  }
  if (res.status === 401) throw new Error("미인증 — 다시 로그인하세요");
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
  }
  const text = await res.text();
  if (!text) return undefined as unknown as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    return text as unknown as T;
  }
}

/**
 * ACP `session/update` SSE 스트림 구독. EventSource 는 Bearer 헤더를 못 싣기에
 * fetch + ReadableStream 으로 `event: session_update` 프레임을 파싱한다.
 * @returns 구독 취소 함수 (AbortController.abort).
 */
export function acpStream(
  sessionId: string,
  onUpdate: (payload: unknown) => void,
  onError: (msg: string) => void,
): () => void {
  const ctrl = new AbortController();
  const url = `${getAcpBase()}/sessions/${encodeURIComponent(sessionId)}/stream`;
  (async () => {
    let res: Response;
    try {
      res = await fetch(url, { headers: authHeaders(false), signal: ctrl.signal });
    } catch (e) {
      if (!ctrl.signal.aborted) onError(`스트림 연결 실패: ${(e as Error).message}`);
      return;
    }
    if (!res.ok || !res.body) {
      onError(`스트림 HTTP ${res.status}`);
      return;
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = "";
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        // SSE 프레임은 빈 줄(\n\n)로 구분. event:/data: 라인 파싱.
        let sep: number;
        while ((sep = buf.indexOf("\n\n")) !== -1) {
          const frame = buf.slice(0, sep);
          buf = buf.slice(sep + 2);
          const dataLines: string[] = [];
          for (const line of frame.split("\n")) {
            if (line.startsWith("data:")) dataLines.push(line.slice(5).trimStart());
          }
          if (dataLines.length === 0) continue;
          const data = dataLines.join("\n");
          try {
            onUpdate(JSON.parse(data));
          } catch {
            // 비-JSON keep-alive/주석 프레임 — 무시.
          }
        }
      }
    } catch (e) {
      if (!ctrl.signal.aborted) onError(`스트림 중단: ${(e as Error).message}`);
    }
  })();
  return () => ctrl.abort();
}

/** path 템플릿 치환 + 남은 args 반환. */
function renderPath(
 template: string,
 args: Record<string, unknown> | undefined,
): { path: string; remaining: Record<string, unknown>} {
 if (!args) return { path: template, remaining: {}};
 const remaining: Record<string, unknown> = { ...args};
 const path = template.replace(/\{(\w+)\}/g, (_m, key: string) => {
 const v = remaining[key];
 if (v === undefined || v === null) {
 throw new Error(`invoke: path placeholder {${key}} 누락`);
}
 delete remaining[key];
 return encodeURIComponent(String(v));
});
 return { path, remaining};
}

/**
 * invoke shim — Tauri 코어의 `invoke()` 와 동일한 signature.
 *
 * @param command daemon GUI 명령 (예: "peers_list", "vault_pending_approve").
 * @param args path placeholder + body. POST/PUT 에선 path placeholder 외 모든
 * 키가 JSON body 로 전송됨.
 * @throws Error("미인증 ...") on 401.
 * @throws Error("HTTP NNN: ...") on non-2xx.
 * @throws Error("invoke: ... 미지원 ...") on unknown command.
 */
export async function invoke<T>(
 command: string,
 args?: Record<string, unknown>,
): Promise<T> {
 const route = ROUTES[command];
 if (!route) {
 throw new Error(
 `invoke: 명령 '${command}' 은(는) Web GUI 에서 미지원. ` +
 `(daemon REST API 미존재 — Tauri 빌드만 가능)`,
);
}

 const { path, remaining} = renderPath(route.path, args);
 const base = getDaemonUrl().replace(/\/+$/, "");
 let url = `${base}${path}`;
 const headers: Record<string, string> = {};
 const token = getBearer();
 if (token) {
 headers["Authorization"] = `Bearer ${token}`;
}

 let body: string | undefined;
 if (route.body && (route.method === "POST" || route.method === "PUT" || route.method === "PATCH")) {
 headers["Content-Type"] = "application/json";
 body = JSON.stringify(remaining);
} else if (
 Object.keys(remaining).length > 0 &&
 (route.method === "POST" || route.method === "PUT" || route.method === "PATCH")
) {
 // body:true 가 false 여도 POST/PUT 에 잔여 args 있으면 body 로 전송 (안전 기본).
 headers["Content-Type"] = "application/json";
 body = JSON.stringify(remaining);
} else if (
 Object.keys(remaining).length > 0 &&
 (route.method === "GET" || route.method === "DELETE")
) {
 // GET/DELETE 의 잔여 args 는 query string 으로 전송.
 const qs = new URLSearchParams(
 Object.entries(remaining).map(([k, v]) => [k, String(v)]),
).toString();
 url += (url.includes("?") ? "&" : "?") + qs;
}

 let res: Response;
 try {
 res = await fetch(url, { method: route.method, headers, body});
} catch (e) {
 throw new Error(
 `daemon 연결 실패 (${url}) — daemon 가동 + URL 확인: ${(e as Error).message}`,
);
}

 if (res.status === 401) {
 // 세션 만료/위조(주로 daemon 재시작으로 토큰 무효) — Bearer 삭제 + 로그인 화면으로 자동 복귀.
 // 토큰이 없으면 App 은 LoginView 를 렌더하고 인증 호출을 안 하므로 reload 루프 없음.
 try {
 localStorage.removeItem(TOKEN_KEY);
 localStorage.removeItem("xgram_session_token");
 if (typeof location !== "undefined" && !new URLSearchParams(location.search).get("chat") && !new URLSearchParams(location.search).get("tmux")) {
 setTimeout(() => location.reload(), 50);
 }
} catch {
 // ignored
}
 throw new Error("미인증 — 다시 로그인하세요");
}
 if (!res.ok) {
 const text = await res.text().catch(() => "");
 throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
}

 const text = await res.text();
 if (!text) {
 return (route.emptyAs ?? (undefined as unknown)) as T;
}
 try {
 return JSON.parse(text) as T;
} catch {
 return text as unknown as T;
}
}
