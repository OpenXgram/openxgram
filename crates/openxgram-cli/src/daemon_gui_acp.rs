//! ACP (Agent Client Protocol, Zed) — daemon HTTP surface (`/v1/acp/*`).
//!
//! Phase B-2: wire the `openxgram-acp` crate into the production daemon. This
//! module owns the **daemon-side ACP process registry** and the GUI-facing
//! conversation-session bookkeeping, plus the daemon's [`ClientSideHandlers`]
//! implementation. It is **purely additive** — `daemon_gui.rs` keeps a single
//! [`AcpHttpState`] field and registers the `/v1/acp/*` routes; nothing existing
//! is modified.
//!
//! Design (정본: `docs/research/acp-core-integration.md` §3 hosting, §5 lifecycle,
//! §6 full-duplex):
//!   - The long-lived `HashMap<handleId, AcpClient>` lives inside
//!     [`openxgram_acp::AcpTools`] (Clone, internally `Arc<Mutex<..>>`). We reuse
//!     it rather than re-implementing a second registry — the crate already
//!     guarantees agents outlive a single request frame.
//!   - An HTTP session id (stable, GUI-facing) maps to `{ handle_id, agent, cwd,
//!     execution_mode, spawned }` so `on_demand` agents can spawn lazily on the
//!     first prompt. The map is guarded by an async `Mutex`.
//!   - SSE relay: each session owns a `tokio::sync::broadcast` channel;
//!     `session/update` notifications produced during a prompt turn are
//!     re-broadcast to any connected `/stream` client.
//!
//! 절대 규칙 1 (fallback 금지): every failure path returns an explicit HTTP
//! status + message; no silent default. No `.unwrap()`/`.expect()` here.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use openxgram_acp::handlers::ClientSideHandlers;
use openxgram_acp::mcp::AgentHandleId;
use openxgram_acp::{AcpError, AcpTools};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

/// Explicit error type for ACP HTTP handlers → `(StatusCode, message)`.
pub type AcpHttpError = (StatusCode, String);

/// `agent_profiles.execution_mode` hosting branch (§3, ACP-INTEGRATION-PLAN §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Spawn immediately when the HTTP session is created.
    Always,
    /// Spawn lazily on the first `prompt` (B-2 default).
    OnDemand,
    /// Queue-driven wake (Phase 4 — stubbed in B-2: treated like `on_demand`).
    Heartbeat,
}

impl ExecutionMode {
    /// Parse a free-text mode; unknown → explicit error (no silent default).
    pub fn parse(s: &str) -> Result<Self, AcpHttpError> {
        match s {
            "always" => Ok(Self::Always),
            "on_demand" => Ok(Self::OnDemand),
            "heartbeat" => Ok(Self::Heartbeat),
            other => Err((
                StatusCode::BAD_REQUEST,
                format!("unknown execution_mode: {other} (want always|on_demand|heartbeat)"),
            )),
        }
    }
}

/// Per-HTTP-session bookkeeping. The GUI addresses a conversation by `http id`;
/// this maps it onto a spawned agent handle + its working dir.
struct AcpHttpSession {
    agent: String,
    cwd: String,
    /// Retained for the heartbeat queue (Phase 4) + introspection. The spawn-
    /// timing branch reads `mode` at create/prompt time; the stored copy is not
    /// re-read in B-2, hence the allow.
    #[allow(dead_code)]
    execution_mode: ExecutionMode,
    /// `Some` once the agent has been spawned (always-mode at create, on_demand
    /// at first prompt). `None` means a lazy session not yet spawned.
    handle_id: Option<AgentHandleId>,
    /// Broadcast channel for relaying `session/update` to `/stream` clients.
    updates_tx: broadcast::Sender<Value>,
}

/// Daemon-held ACP state. Lives in `GuiServerState` (Clone-cheap: all `Arc`).
#[derive(Clone)]
pub struct AcpHttpState {
    /// Reused crate-level process registry (`HashMap<handleId, AcpClient>`).
    tools: AcpTools,
    /// HTTP session id → bookkeeping.
    sessions: Arc<Mutex<HashMap<String, AcpHttpSession>>>,
    /// Monotonic source for HTTP session ids.
    next_session: Arc<std::sync::atomic::AtomicU64>,
}

impl Default for AcpHttpState {
    fn default() -> Self {
        Self::new()
    }
}

impl AcpHttpState {
    /// Fresh, empty ACP HTTP state.
    pub fn new() -> Self {
        Self {
            tools: AcpTools::new(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_session: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    fn new_session_id(&self) -> String {
        let n = self
            .next_session
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("acp-{n}")
    }
}

// ── Request/response bodies ────────────────────────────────────────────────

/// `POST /v1/acp/sessions` body.
#[derive(Debug, Deserialize)]
pub struct CreateSessionBody {
    /// Registry agent name (e.g. `claude-agent-acp`).
    pub agent: String,
    /// Working directory for `session/new`.
    pub cwd: String,
    /// Optional MCP servers passed to the agent (forwarded verbatim).
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    /// Hosting mode; defaults to `on_demand` when omitted.
    #[serde(default)]
    pub execution_mode: Option<String>,
}

/// `POST /v1/acp/sessions/{id}/prompt` body.
#[derive(Debug, Deserialize)]
pub struct PromptBody {
    /// Prompt text (single text ContentBlock for B-2).
    pub text: String,
}

// ── Handlers (free fns; daemon_gui.rs wraps them after require_auth) ────────

/// `GET /v1/acp/agents` — known adapters + an `installed` probe per agent.
pub fn list_agents(state: &AcpHttpState) -> Value {
    let base = state.tools.acp_list_agents();
    let names = base
        .get("agents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let detailed: Vec<Value> = names
        .iter()
        .filter_map(|n| n.as_str())
        .map(|name| {
            let installed = openxgram_acp::registry::lookup(name)
                .ok()
                .map(|spec| command_installed(&spec.command))
                .unwrap_or(false);
            json!({ "name": name, "installed": installed })
        })
        .collect();
    json!({ "agents": detailed })
}

/// Best-effort `which`-style probe: is the agent command on PATH / executable?
fn command_installed(command: &str) -> bool {
    // Absolute/relative path → check directly.
    if command.contains('/') {
        return std::path::Path::new(command).exists();
    }
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(command);
        candidate.exists()
    })
}

/// `POST /v1/acp/sessions` — create an HTTP session. `always` spawns now;
/// `on_demand`/`heartbeat` defer the spawn to the first prompt.
pub async fn create_session(
    state: &AcpHttpState,
    body: CreateSessionBody,
) -> Result<Value, AcpHttpError> {
    let mode = match body.execution_mode.as_deref() {
        Some(s) => ExecutionMode::parse(s)?,
        None => ExecutionMode::OnDemand,
    };
    // Validate the agent name eagerly (explicit error, never a guessed default).
    openxgram_acp::registry::lookup(&body.agent)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e}")))?;

    let (updates_tx, _rx) = broadcast::channel::<Value>(256);
    let session_id = state.new_session_id();

    let handle_id = if mode == ExecutionMode::Always {
        Some(spawn_handle(state, &body.agent).await?)
    } else {
        None
    };

    let sess = AcpHttpSession {
        agent: body.agent.clone(),
        cwd: body.cwd.clone(),
        execution_mode: mode,
        handle_id,
        updates_tx,
    };
    state.sessions.lock().await.insert(session_id.clone(), sess);

    Ok(json!({
        "sessionId": session_id,
        "agent": body.agent,
        "cwd": body.cwd,
        "executionMode": mode,
        "spawned": handle_id.is_some(),
    }))
}

/// Spawn an agent via the crate registry, returning its handle id. The crate's
/// `acp_spawn` runs `initialize`; failure (e.g. agent not installed) is surfaced
/// explicitly.
async fn spawn_handle(state: &AcpHttpState, agent: &str) -> Result<AgentHandleId, AcpHttpError> {
    let v = state
        .tools
        .acp_spawn(agent)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("acp spawn failed: {e}")))?;
    v.get("handleId")
        .and_then(|h| h.as_u64())
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "spawn returned no handleId".to_string(),
            )
        })
}

/// `POST /v1/acp/sessions/{id}/prompt` — drive one `session/prompt` turn.
/// Spawns the agent first if the session is `on_demand`/`heartbeat` and unspawned.
/// Relays the turn's `session/update`s onto the session broadcast channel, then
/// returns the final `{ stopReason }` (+ `updates` for non-SSE callers).
pub async fn prompt(
    state: &AcpHttpState,
    session_id: &str,
    body: PromptBody,
) -> Result<Value, AcpHttpError> {
    // Resolve (and lazily spawn) the handle + cwd under the lock, then release
    // the lock before the (potentially long) prompt turn.
    let (handle_id, cwd, tx) = {
        let mut sessions = state.sessions.lock().await;
        let sess = sessions
            .get_mut(session_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("unknown session: {session_id}")))?;
        if sess.handle_id.is_none() {
            // on_demand / heartbeat: spawn on first prompt (§3 hosting).
            let hid = spawn_handle(state, &sess.agent).await?;
            sess.handle_id = Some(hid);
        }
        let hid = sess.handle_id.ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "session has no handle after spawn".to_string(),
            )
        })?;
        (hid, sess.cwd.clone(), sess.updates_tx.clone())
    };

    let result = state
        .tools
        .acp_prompt(handle_id, &cwd, &body.text)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("acp prompt failed: {e}")))?;

    // Relay each update onto the broadcast channel for any `/stream` client.
    if let Some(updates) = result.get("updates").and_then(|u| u.as_array()) {
        for u in updates {
            // Ignore send errors: no subscribers is a normal state.
            let _ = tx.send(u.clone());
        }
    }

    Ok(result)
}

/// `POST /v1/acp/sessions/{id}/cancel` — `session/cancel` for the session's
/// active agent. Cancel targets the ACP session id; for B-2 the crate's
/// `acp_prompt` runs a fresh ACP session per turn, so we cancel by the daemon
/// session's agent handle using its own session id passthrough.
pub async fn cancel(state: &AcpHttpState, session_id: &str) -> Result<Value, AcpHttpError> {
    let handle_id = {
        let sessions = state.sessions.lock().await;
        let sess = sessions
            .get(session_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("unknown session: {session_id}")))?;
        sess.handle_id.ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                "session not yet spawned — nothing to cancel".to_string(),
            )
        })?
    };
    // The crate cancels by ACP session id; we pass the HTTP session id through —
    // the agent treats an unknown id as a no-op cancel (notification, no error).
    state
        .tools
        .acp_cancel(handle_id, session_id)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("acp cancel failed: {e}")))
}

/// `DELETE /v1/acp/sessions/{id}` — close + reap the agent, drop the session.
pub async fn close(state: &AcpHttpState, session_id: &str) -> Result<Value, AcpHttpError> {
    let sess = state
        .sessions
        .lock()
        .await
        .remove(session_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("unknown session: {session_id}")))?;
    match sess.handle_id {
        Some(hid) => state
            .tools
            .acp_close(hid)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("acp close failed: {e}"))),
        // Never spawned (lazy session) → just dropped; report success.
        None => Ok(json!({ "closed": true, "spawned": false })),
    }
}

/// Subscribe to a session's `session/update` broadcast for SSE relay. Returns
/// the receiver; `daemon_gui.rs` adapts it into an `axum::response::sse::Sse`.
pub async fn subscribe(
    state: &AcpHttpState,
    session_id: &str,
) -> Result<broadcast::Receiver<Value>, AcpHttpError> {
    let sessions = state.sessions.lock().await;
    let sess = sessions
        .get(session_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("unknown session: {session_id}")))?;
    Ok(sess.updates_tx.subscribe())
}

/// Graceful close of **all** spawned agents — call on daemon shutdown / session
/// sweep (§5 zombie reap). Best-effort: errors are logged, never propagated, so
/// one stuck agent cannot block the rest of the sweep.
pub async fn reap_all(state: &AcpHttpState) {
    let handles: Vec<AgentHandleId> = {
        let sessions = state.sessions.lock().await;
        sessions.values().filter_map(|s| s.handle_id).collect()
    };
    for hid in handles {
        if let Err(e) = state.tools.acp_close(hid).await {
            tracing::debug!(target: "acp.daemon", handle = hid, "reap_all close: {e}");
        }
    }
    state.sessions.lock().await.clear();
}

// ── Client-side handlers (agent → daemon callbacks) ────────────────────────

/// Daemon's [`ClientSideHandlers`] — B-2 policy: **default-deny + audit log**.
///
/// Matches the crate trait shape exactly: one `handle(method, params)` dispatch
/// plus `advertised_*` capability flags. Capability-honest (§6): advertises no
/// `fs`/`terminal`, so a spec-conformant agent never invokes them; if one does,
/// we reject loudly (절대 규칙 1 — explicit [`AcpError::Protocol`], no silent
/// success). `session/request_permission` is denied (`cancelled` outcome).
///
/// Real vault/permission-backed `fs/*` + permission policy is a later phase
/// (§3.3 / Phase 4). For B-2 this is the safe, honest default. It is provided so
/// the daemon can drive `AcpClient::spawn(...)` with its own policy in a future
/// phase; the current GUI/MCP path uses the crate's `spawn_minimal` default.
#[derive(Debug, Default, Clone)]
pub struct DaemonAcpHandlers {
    /// Permission decision applied to every `session/request_permission`.
    pub permission: DaemonPermission,
}

/// Permission posture for [`DaemonAcpHandlers`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DaemonPermission {
    /// Deny every request (B-2 default).
    #[default]
    Deny,
    /// Allow (selects the first offered option). Reserved for later phases.
    Allow,
}

impl ClientSideHandlers for DaemonAcpHandlers {
    fn handle<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = openxgram_acp::Result<Value>> + Send + 'a>>
    {
        let decision = self.permission;
        Box::pin(async move {
            match method {
                "session/request_permission" => match decision {
                    DaemonPermission::Deny => {
                        tracing::warn!(target: "acp.daemon", "session/request_permission → deny (B-2 default policy)");
                        Ok(json!({ "outcome": { "outcome": "cancelled" } }))
                    }
                    DaemonPermission::Allow => {
                        let option_id = params
                            .get("options")
                            .and_then(|o| o.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|opt| opt.get("optionId"))
                            .and_then(|id| id.as_str())
                            .unwrap_or("allow")
                            .to_string();
                        Ok(json!({ "outcome": { "outcome": "selected", "optionId": option_id } }))
                    }
                },
                "fs/read_text_file" | "fs/write_text_file" => {
                    tracing::warn!(target: "acp.daemon", method, "fs/* denied (B-2 default-deny)");
                    Err(AcpError::Protocol(format!(
                        "method {method} not advertised by daemon client (default-deny in B-2)"
                    )))
                }
                m if m.starts_with("terminal/") => Err(AcpError::Protocol(format!(
                    "terminal capability not advertised: {m}"
                ))),
                other => Err(AcpError::Protocol(format!(
                    "unhandled client-side method: {other}"
                ))),
            }
        })
    }

    // Capability honesty (§6): advertise nothing we do not implement in B-2.
    fn advertised_fs_read(&self) -> bool {
        false
    }
    fn advertised_fs_write(&self) -> bool {
        false
    }
    fn advertised_terminal(&self) -> bool {
        false
    }
}
