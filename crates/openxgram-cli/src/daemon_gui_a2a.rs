//! A2A (Google Agent2Agent) — daemon HTTP surface (`/v1/gui/a2a/*`).
//!
//! Phase 3 (ACP-A2A-CORE): wire the `openxgram-a2a` crate into the production
//! daemon so an OpenXgram agent (LLM via MCP, or the GUI) can **call another
//! agent** over the A2A protocol — AgentCard discovery
//! (`/.well-known/agent-card.json`) + JSON-RPC `tasks/send` / `tasks/get` /
//! `tasks/cancel`. It mirrors `daemon_gui_acp.rs` exactly (the proven B-2
//! pattern): one [`A2aHttpState`] field on `GuiServerState`, additive routes,
//! no existing handler touched.
//!
//! ⚠️ SCOPING — the `openxgram-a2a` crate is **CLIENT-only** (it ships no HTTP
//! server). The CLIENT half (call external/other A2A agents) lives in the top of
//! this module. The SERVER half — making OpenXgram's own agents **CALLABLE via
//! A2A** — is implemented in the [`server`] submodule below (ACP-A2A-CORE step
//! 1–4): each agent hosts an AgentCard at
//! `/v1/a2a/agents/{alias}/.well-known/agent-card.json` and a JSON-RPC-shaped
//! `tasks/send` handler at `POST /v1/a2a/agents/{alias}/tasks`. The task handler
//! **executes** by driving the target agent over ACP — it reuses the daemon's
//! existing [`crate::daemon_gui_acp::AcpHttpState`] (one ACP registry, never a
//! second) to spawn the agent's `ai_type` adapter in its `project_path`, prompt
//! it with the task, and return the collected agent text.
//!
//! 절대 규칙 1 (fallback 금지): every failure path returns an explicit HTTP
//! status + message; no silent default. No `.unwrap()`/`.expect()` here.

use axum::http::StatusCode;
use openxgram_a2a::mcp::SendTaskArgs;
use openxgram_a2a::{A2aError, A2aTools};
use serde::Deserialize;
use serde_json::{json, Value};

pub use server::{
    build_agent_card, handle_task, served_task, ServedA2aState, TaskBody, A2A_PROTOCOL_VERSION,
};

/// Explicit error type for A2A HTTP handlers → `(StatusCode, message)`.
pub type A2aHttpError = (StatusCode, String);

/// Daemon-held A2A state. Lives in `GuiServerState` (Clone-cheap).
///
/// The crate's [`A2aTools`] is stateless (a fresh `A2aClient` per call), so this
/// is a thin wrapper kept symmetric with [`crate::daemon_gui_acp::AcpHttpState`]
/// — future per-target bearer / target registry hangs off here.
#[derive(Clone, Default)]
pub struct A2aHttpState {
    /// Stateless A2A client tool surface (discover + tasks/send|get|cancel).
    tools: A2aTools,
}

impl A2aHttpState {
    /// Fresh A2A HTTP state.
    pub fn new() -> Self {
        Self {
            tools: A2aTools::new(),
        }
    }
}

/// Map a crate-level [`A2aError`] onto an explicit HTTP status (no silent
/// fallback — every variant has a deliberate code).
fn a2a_status(e: &A2aError) -> StatusCode {
    match e {
        A2aError::InvalidUrl(_) => StatusCode::BAD_REQUEST,
        A2aError::AgentCardFetch { .. } => StatusCode::BAD_GATEWAY,
        A2aError::Http(_) => StatusCode::BAD_GATEWAY,
        A2aError::RpcError { .. } => StatusCode::BAD_GATEWAY,
        A2aError::InvalidRpcResponse => StatusCode::BAD_GATEWAY,
        A2aError::UnknownTaskState(_) => StatusCode::BAD_GATEWAY,
        A2aError::Serde(_) => StatusCode::INTERNAL_SERVER_ERROR,
        A2aError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn map_err(e: A2aError) -> A2aHttpError {
    (a2a_status(&e), format!("a2a error: {e}"))
}

// ── Request bodies ─────────────────────────────────────────────────────────

/// `POST /v1/gui/a2a/send` body.
///
/// `target` is the A2A agent base URL (origin or AgentCard `url`). We discover
/// its AgentCard first (to honestly verify it is reachable + pick a skill), then
/// `tasks/send`.
#[derive(Debug, Deserialize)]
pub struct SendBody {
    /// Optional originating OpenXgram agent (audit/log only in this phase —
    /// AgentCard hosting for callee-side identity is a follow-up).
    #[serde(default)]
    pub from_agent: Option<String>,
    /// Target A2A agent base URL (required — no guessed default).
    pub target: String,
    /// Skill id to invoke. If omitted, the target's first advertised skill is
    /// used; if the target advertises none, an explicit error is returned.
    #[serde(default)]
    pub skill: Option<String>,
    /// Free-form task params forwarded to the skill (default empty object).
    #[serde(default)]
    pub task: Value,
    /// Optional session/context id passed through to `tasks/send`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// 전달 엔드포인트 — 같은 신원(alias)이라도 보낼 곳(전달 위치)은 여럿이다.
    /// 값: `"new_acp"` | `"existing_acp:<sessionId>"` | `"tmux:<sessionName>"` |
    /// `"worktree"` | `"external"`. `None` 이면 `new_acp` 와 동일(기존 동작 보존).
    /// 라우팅 분기는 `daemon_gui.rs::a2a_send` 가 수행한다.
    #[serde(default)]
    pub endpoint: Option<String>,
}

// ── Handlers (free fns; daemon_gui.rs wraps them after require_auth) ────────

/// One roster entry for [`list_agents`] — alias + the data needed to decide A2A
/// reachability (an `ai_type` that maps to a known ACP adapter ⇒ drivable).
pub struct A2aAgentInfo {
    /// Agent alias (== AgentCard name + the `{alias}` route segment).
    pub alias: String,
    /// `agent_profiles.ai_type` (free text, e.g. `claude`/`codex`/`gemini`).
    /// `None`/empty ⇒ not ACP-drivable ⇒ `reachable:false`.
    pub ai_type: Option<String>,
}

/// `GET /v1/gui/a2a/agents` — list A2A-reachable OpenXgram agents.
///
/// Server-side AgentCard hosting now exists, so this is HONEST about who is
/// actually callable: an agent whose `ai_type` resolves to a known ACP adapter
/// (via [`server::resolve_acp_agent`]) is **reachable** — it advertises an
/// `agentCardUrl` (the hosted card route) and a `tasksUrl` (the `tasks/send`
/// endpoint). An agent with no drivable `ai_type` is reported `reachable:false`
/// (no fabricated reachability — 절대 규칙 1). The roster is supplied by the
/// caller (daemon_gui.rs reads `agent_capabilities`⋈`agent_profiles` under the
/// DB lock).
pub fn list_agents(agents_in: &[A2aAgentInfo]) -> Value {
    let agents: Vec<Value> = agents_in
        .iter()
        .map(|a| {
            let drivable = a
                .ai_type
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .and_then(server::resolve_acp_agent)
                .is_some();
            if drivable {
                json!({
                    "alias": a.alias,
                    "reachable": true,
                    "aiType": a.ai_type,
                    "agentCardUrl": server::agent_card_url(&a.alias),
                    "tasksUrl": server::agent_tasks_url(&a.alias),
                })
            } else {
                json!({
                    "alias": a.alias,
                    "reachable": false,
                    "aiType": a.ai_type,
                    "agentCardUrl": Value::Null,
                    "reason": "no ACP-drivable ai_type (cannot be executed via ACP)",
                })
            }
        })
        .collect();
    json!({
        "agents": agents,
        "note": "reachable:true 에이전트는 A2A로 호출 가능: agentCardUrl 에서 AgentCard 조회 후 \
tasksUrl 로 tasks/send. 실행은 ACP(에이전트 ai_type 어댑터를 project_path 에서 spawn)로 구동. \
외부 A2A 대상 호출은 /v1/gui/a2a/send 의 target(base URL) 사용.",
    })
}

/// `POST /v1/gui/a2a/send` — discover the target's AgentCard, then `tasks/send`.
///
/// Reuses [`A2aTools::discover`] + [`A2aTools::send_task`] (the crate's client) —
/// the JSON-RPC / AgentCard protocol is NOT reimplemented here.
pub async fn send(state: &A2aHttpState, body: SendBody) -> Result<Value, A2aHttpError> {
    if body.target.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "missing 'target' (A2A 에이전트 base URL)".to_string(),
        ));
    }

    // Discover first — honest reachability check + skill resolution.
    let card = state.tools.discover(&body.target).await.map_err(map_err)?;

    let skill = match body.skill {
        Some(s) => s,
        None => card
            .skills
            .first()
            .map(|sk| sk.id.clone())
            .ok_or_else(|| {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "target '{}' advertises no skills and no 'skill' was provided",
                        card.url
                    ),
                )
            })?,
    };

    let params = if body.task.is_null() {
        json!({})
    } else {
        body.task
    };

    let task = state
        .tools
        .send_task(SendTaskArgs {
            agent_url: body.target.clone(),
            skill: skill.clone(),
            params,
            session_id: body.session_id,
        })
        .await
        .map_err(map_err)?;

    let task_value = serde_json::to_value(&task).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("task serialize: {e}"),
        )
    })?;

    Ok(json!({
        "taskId": task.id,
        "skill": skill,
        "fromAgent": body.from_agent,
        "target": body.target,
        "task": task_value,
    }))
}

/// `GET /v1/gui/a2a/tasks/{id}` — `tasks/get` status for a task on `target`.
///
/// The target base URL must be provided as a `target` query param: a bare task
/// id is not routable without knowing which agent owns it (no central task
/// registry in this client-only phase).
pub async fn get_task(
    state: &A2aHttpState,
    target: &str,
    task_id: &str,
) -> Result<Value, A2aHttpError> {
    if target.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "missing 'target' query param (A2A 에이전트 base URL)".to_string(),
        ));
    }
    let task = state.tools.get_task(target, task_id).await.map_err(map_err)?;
    let task_value = serde_json::to_value(&task).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("task serialize: {e}"),
        )
    })?;
    Ok(json!({ "taskId": task.id, "task": task_value }))
}

// ── SERVER side (ACP-A2A-CORE) — OpenXgram agents CALLABLE via A2A ──────────
//
// AgentCard hosting + a `tasks/send` handler that EXECUTES by driving the target
// agent over ACP. Reuses `crate::daemon_gui_acp::AcpHttpState` for execution
// (one ACP registry — never a second). Purely additive; the client routes above
// are untouched.
pub mod server {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::http::StatusCode;
    use serde::Deserialize;
    use serde_json::{json, Value};
    use tokio::sync::Mutex;

    use super::A2aHttpError;
    use crate::daemon_gui_acp::{self, AcpHttpState};
    use openxgram_a2a::agent_card::{AgentCapabilities, AgentCard, AgentSkill, Authentication};

    /// A2A protocol/AgentCard version we advertise for served cards.
    pub const A2A_PROTOCOL_VERSION: &str = "0.2.0";

    /// One agent's enough-to-build-a-card metadata, supplied by daemon_gui.rs
    /// from `agent_capabilities`⋈`agent_profiles` under the DB lock.
    pub struct AgentMeta {
        /// Alias = AgentCard `name` + the `{alias}` route segment.
        pub alias: String,
        /// `agent_capabilities.role` → AgentCard `description`.
        pub role: Option<String>,
        /// `agent_capabilities.capabilities` (raw stored form — JSON array or CSV).
        pub capabilities: Option<String>,
        /// `agent_profiles.ai_type` → ACP adapter (must resolve to be drivable).
        pub ai_type: Option<String>,
        /// `agent_capabilities.project_path` → ACP session cwd.
        pub project_path: Option<String>,
    }

    /// Hosted AgentCard route for `alias` (relative — A2A discovery is host-rooted).
    pub fn agent_card_url(alias: &str) -> String {
        format!("/v1/a2a/agents/{alias}/.well-known/agent-card.json")
    }

    /// Hosted `tasks/send` endpoint for `alias`.
    pub fn agent_tasks_url(alias: &str) -> String {
        format!("/v1/a2a/agents/{alias}/tasks")
    }

    /// Map a free-text `ai_type` onto a known ACP adapter registry name, or
    /// `None` when it is not ACP-drivable (no silent default — the caller treats
    /// `None` as an explicit "cannot execute via ACP").
    ///
    /// Accepts both the bare model family (`claude`, `codex`, `gemini`,
    /// `opencode`) and the already-canonical adapter name (`claude-agent-acp`,
    /// `codex-acp`). Validation against the live registry happens in [`handle_task`].
    pub fn resolve_acp_agent(ai_type: &str) -> Option<&'static str> {
        match ai_type.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-agent-acp" | "claude-code" => Some("claude-agent-acp"),
            "codex" | "codex-acp" => Some("codex-acp"),
            "gemini" | "gemini-cli" => Some("gemini"),
            "opencode" => Some("opencode"),
            _ => None,
        }
    }

    /// Parse the stored `capabilities` form (JSON array of strings, or a CSV
    /// fallback) into a clean keyword list. Never fails — an unparseable value
    /// simply yields no skills (the role-derived default skill still applies).
    fn parse_capabilities(raw: Option<&str>) -> Vec<String> {
        let Some(raw) = raw else {
            return Vec::new();
        };
        let raw = raw.trim();
        if raw.is_empty() {
            return Vec::new();
        }
        if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(raw) {
            return arr
                .into_iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
        }
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Build the served [`AgentCard`] for `meta`. No secrets: only public roster
    /// fields (alias/role/capabilities) + the agent's own A2A task endpoint URL.
    /// Skills are derived from `capabilities` (one skill each), plus a baseline
    /// `chat` skill that maps the whole message onto an ACP prompt.
    pub fn build_agent_card(meta: &AgentMeta) -> AgentCard {
        let description = meta
            .role
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("OpenXgram agent (A2A-callable, executed via ACP)")
            .to_string();

        let mut skills: Vec<AgentSkill> = Vec::new();
        // Baseline skill: free-form prompt → ACP turn. Always present so a caller
        // can drive the agent without knowing its capability vocabulary.
        skills.push(AgentSkill {
            id: "chat".to_string(),
            name: "Chat / task".to_string(),
            description: "Send a free-form task; executed by driving the agent via ACP."
                .to_string(),
            input_modes: vec!["text".to_string()],
            output_modes: vec!["text".to_string()],
            extra: serde_json::Map::new(),
        });
        for cap in parse_capabilities(meta.capabilities.as_deref()) {
            skills.push(AgentSkill {
                id: cap.clone(),
                name: cap.clone(),
                description: format!("Capability '{cap}' (executed via ACP prompt)."),
                input_modes: vec!["text".to_string()],
                output_modes: vec!["text".to_string()],
                extra: serde_json::Map::new(),
            });
        }

        AgentCard {
            name: meta.alias.clone(),
            description,
            // The agent's A2A base URL = its tasks endpoint (host-relative;
            // discovery resolves it against the serving origin). No secrets.
            url: agent_tasks_url(&meta.alias),
            version: A2A_PROTOCOL_VERSION.to_string(),
            // No bearer/secret advertised in the public card.
            authentication: Authentication {
                schemes: vec!["none".to_string()],
                extra: serde_json::Map::new(),
            },
            skills,
            capabilities: AgentCapabilities {
                // ACP turns are non-streaming over this A2A surface in this step.
                streaming: false,
                push_notifications: false,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
        }
    }

    /// `POST /v1/a2a/agents/{alias}/tasks` body (A2A `tasks/send` params).
    ///
    /// Accepts either a structured `message` (A2A) or a flat `task`/`text`
    /// string; `skill` selects the advertised skill (currently informational —
    /// every skill maps onto an ACP prompt of the message text).
    #[derive(Debug, Deserialize)]
    pub struct TaskBody {
        /// Optional skill id from the AgentCard. Recorded on the task.
        #[serde(default)]
        pub skill: Option<String>,
        /// A2A message object (`{role, parts:[{text}]}` or `{text}`); optional.
        #[serde(default)]
        pub message: Value,
        /// Flat task text (alternative to `message`).
        #[serde(default)]
        pub task: Option<String>,
        /// Convenience alias for `task`.
        #[serde(default)]
        pub text: Option<String>,
        /// Optional A2A session/context id (reused as the ACP cwd is per-alias).
        #[serde(default)]
        pub session_id: Option<String>,
        /// 호출한 에이전트 alias(내부 위임 시 채워짐) — 가시 스레드 conv_key/표시에 사용. 외부 호출이면 None.
        #[serde(default)]
        pub from: Option<String>,
    }

    /// One tracked served task (in-memory, mirrors the ACP session bookkeeping).
    #[derive(Clone)]
    struct ServedTask {
        alias: String,
        skill: Option<String>,
        status: String,
        result: Value,
    }

    /// Daemon-held server-side A2A state: the tracked-task map for `tasks/get`.
    /// Execution itself reuses the shared [`AcpHttpState`] (passed in per call) —
    /// this state owns no ACP registry of its own.
    #[derive(Clone, Default)]
    pub struct ServedA2aState {
        tasks: Arc<Mutex<HashMap<String, ServedTask>>>,
        next: Arc<std::sync::atomic::AtomicU64>,
    }

    impl ServedA2aState {
        /// Fresh, empty served-task state.
        pub fn new() -> Self {
            Self {
                tasks: Arc::new(Mutex::new(HashMap::new())),
                next: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            }
        }

        fn new_task_id(&self) -> String {
            let n = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            format!("a2a-task-{n}")
        }
    }

    /// Extract the prompt text from a [`TaskBody`] (message parts ▸ task ▸ text).
    /// Explicit error when no text is found — no empty-prompt fallback.
    fn extract_prompt(body: &TaskBody) -> Result<String, A2aHttpError> {
        // A2A message: { role?, parts: [{ text }] } or { text }.
        if !body.message.is_null() {
            if let Some(parts) = body.message.get("parts").and_then(|p| p.as_array()) {
                let joined: String = parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("");
                if !joined.trim().is_empty() {
                    return Ok(joined);
                }
            }
            if let Some(t) = body.message.get("text").and_then(|t| t.as_str()) {
                if !t.trim().is_empty() {
                    return Ok(t.to_string());
                }
            }
            if let Some(s) = body.message.as_str() {
                if !s.trim().is_empty() {
                    return Ok(s.to_string());
                }
            }
        }
        if let Some(t) = body.task.as_deref().or(body.text.as_deref()) {
            if !t.trim().is_empty() {
                return Ok(t.to_string());
            }
        }
        Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "no task text found (provide message.parts[].text, task, or text)".to_string(),
        ))
    }

    /// Collect the agent's reply text from the ACP `{stopReason, updates}` value:
    /// concatenate every `agent_message_chunk` text block, in order.
    fn collect_agent_text(acp_result: &Value) -> String {
        let Some(updates) = acp_result.get("updates").and_then(|u| u.as_array()) else {
            return String::new();
        };
        let mut out = String::new();
        for u in updates {
            if u.get("sessionUpdate").and_then(|s| s.as_str()) != Some("agent_message_chunk") {
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

    /// `POST /v1/a2a/agents/{alias}/tasks` — THE CORE. Execute an A2A task by
    /// driving the agent (`alias`) over ACP, then return `{taskId, status,
    /// result}` and persist the task for `tasks/get`.
    ///
    /// Flow:
    ///   1. Resolve `meta.ai_type` → ACP adapter; absent ⇒ explicit error (no fake
    ///      result — 절대 규칙 1, the constraint's "can't run ⇒ A2A error").
    ///   2. Reuse [`AcpHttpState`]: create an `always`-mode ACP session (cwd =
    ///      project_path, agent = adapter), `prompt()` with the task text.
    ///   3. Collect the agent_message_chunk text from the turn's updates.
    ///   4. Track + return the task.
    pub async fn handle_task(
        acp: &AcpHttpState,
        served: &ServedA2aState,
        meta: &AgentMeta,
        body: TaskBody,
    ) -> Result<Value, A2aHttpError> {
        let ai_type = meta
            .ai_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("agent '{}' has no ai_type — not ACP-drivable", meta.alias),
                )
            })?;
        let adapter = resolve_acp_agent(ai_type).ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "agent '{}' ai_type '{ai_type}' has no known ACP adapter",
                    meta.alias
                ),
            )
        })?;
        let cwd = meta
            .project_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("agent '{}' has no project_path (ACP cwd)", meta.alias),
                )
            })?;

        let prompt_text = extract_prompt(&body)?;

        // 갭#1 — A2A 위임 교환을 사용자 가시 스레드로 영속화한다. conv_key = `a2a:{from}->{B}`
        // (호출자 alias 를 알면), 외부 호출이면 `a2a:{B}`. label 로도 부여 → 세션의 툴 호출이
        // prompt() 증분 기록(record_stream_tool)을 통해 같은 스레드에 실시간 영속된다.
        let conv_key = match body.from.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(from) => format!("a2a:{from}->{}", meta.alias),
            None => format!("a2a:{}", meta.alias),
        };
        // 호출자(A)의 요청을 'me' 로 기록 → 스레드 시작점.
        acp.record_message(&conv_key, "me", &prompt_text).await;

        // Reuse the shared ACP registry: create a session (always-mode spawns the
        // adapter now → explicit error if not installed), prompt, then close.
        let create = daemon_gui_acp::create_session(
            acp,
            daemon_gui_acp::CreateSessionBody {
                agent: adapter.to_string(),
                cwd: cwd.to_string(),
                mcp_servers: Vec::new(),
                execution_mode: Some("always".to_string()),
                // A2A 위임 = 받은 작업을 실제 수행해야 하므로 도구 실행 허용(default-deny 해제).
                permission_mode: Some("bypassPermissions".to_string()),
                model: None,
                thinking: None,
                machine: None,
                // 가시 스레드 키 — B 의 응답·툴이 이 conv_key 로 영속된다.
                label: Some(conv_key.clone()),
            },
        )
        .await?;
        let session_id = create
            .get("sessionId")
            .and_then(|s| s.as_str())
            .ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "ACP create_session returned no sessionId".to_string(),
                )
            })?
            .to_string();

        let prompt_res = daemon_gui_acp::prompt(
            acp,
            &session_id,
            daemon_gui_acp::PromptBody {
                text: prompt_text.clone(),
            },
        )
        .await;

        // Always attempt to reap the ACP session, regardless of prompt outcome.
        let _ = daemon_gui_acp::close(acp, &session_id).await;

        let acp_result = prompt_res?;
        let stop_reason = acp_result
            .get("stopReason")
            .cloned()
            .unwrap_or(Value::Null);
        let agent_text = collect_agent_text(&acp_result);

        // 갭#1 — B 의 최종 응답을 'agent' 로 기록 → 가시 스레드 완성(me → ▸단계(증분 툴) → agent).
        acp.record_message(&conv_key, "agent", &agent_text).await;

        let task_id = served.new_task_id();
        let result = json!({
            "stopReason": stop_reason,
            "text": agent_text,
            "messages": [{
                "role": "agent",
                "parts": [{ "type": "text", "text": agent_text }],
            }],
        });
        let tracked = ServedTask {
            alias: meta.alias.clone(),
            skill: body.skill.clone(),
            status: "completed".to_string(),
            result: result.clone(),
        };
        served
            .tasks
            .lock()
            .await
            .insert(task_id.clone(), tracked);

        Ok(json!({
            "taskId": task_id,
            "status": "completed",
            "skill": body.skill,
            "agent": meta.alias,
            "result": result,
        }))
    }

    /// `GET /v1/a2a/agents/{alias}/tasks/{id}` (`tasks/get`) — look up a tracked
    /// task. Explicit 404 when unknown, and explicit 409 when the id exists but
    /// belongs to a different alias (no cross-agent leakage).
    pub async fn served_task(
        served: &ServedA2aState,
        alias: &str,
        task_id: &str,
    ) -> Result<Value, A2aHttpError> {
        let tasks = served.tasks.lock().await;
        let t = tasks
            .get(task_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("unknown task: {task_id}")))?;
        if t.alias != alias {
            return Err((
                StatusCode::CONFLICT,
                format!("task {task_id} does not belong to agent {alias}"),
            ));
        }
        Ok(json!({
            "taskId": task_id,
            "agent": t.alias,
            "skill": t.skill,
            "status": t.status,
            "result": t.result,
        }))
    }
}
