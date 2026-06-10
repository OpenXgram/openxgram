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
//! ⚠️ HONEST SCOPING — the `openxgram-a2a` crate is **CLIENT-only**. This wiring
//! lets OpenXgram CALL external / other A2A agents. For OpenXgram's own agents to
//! be CALLABLE via A2A they must host an AgentCard at
//! `/.well-known/agent-card.json` and a JSON-RPC `tasks/*` endpoint — that
//! AgentCard **hosting** (server side) is a deliberate follow-up and is NOT
//! implemented here. The crate exposes no server. See `list_agents` below: until
//! a target registry / AgentCard hosting exists, registered OpenXgram peers are
//! reported with `reachable: false`.
//!
//! 절대 규칙 1 (fallback 금지): every failure path returns an explicit HTTP
//! status + message; no silent default. No `.unwrap()`/`.expect()` here.

use axum::http::StatusCode;
use openxgram_a2a::mcp::SendTaskArgs;
use openxgram_a2a::{A2aError, A2aTools};
use serde::Deserialize;
use serde_json::{json, Value};

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
}

// ── Handlers (free fns; daemon_gui.rs wraps them after require_auth) ────────

/// `GET /v1/gui/a2a/agents` — list A2A-reachable agents.
///
/// HONEST: the crate is client-only and OpenXgram agents do not yet host an
/// AgentCard, so there is no registry of A2A target endpoints. We return the
/// known OpenXgram peers (alias) with `reachable: false` + an explanatory note,
/// rather than fabricating reachable A2A agents. `peers` is supplied by the
/// caller (daemon_gui.rs reads the peer roster under the DB lock).
///
/// Once AgentCard hosting (follow-up) or a target-URL registry lands, this
/// returns the actually-reachable A2A endpoints with `reachable: true`.
pub fn list_agents(peers: &[String]) -> Value {
    let agents: Vec<Value> = peers
        .iter()
        .map(|alias| {
            json!({
                "alias": alias,
                "reachable": false,
                "agentCardUrl": Value::Null,
            })
        })
        .collect();
    json!({
        "agents": agents,
        "note": "A2A는 client-only로 연결됨: OpenXgram이 외부 A2A 에이전트를 호출할 수 있다. \
OpenXgram 에이전트가 A2A로 호출되려면(AgentCard 호스팅) 후속 작업 필요 → 현재 모든 peer는 reachable:false. \
외부 A2A 대상은 /v1/gui/a2a/send 의 target(base URL)로 직접 호출.",
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
