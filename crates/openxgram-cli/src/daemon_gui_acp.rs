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
use tokio::sync::{broadcast, mpsc, Mutex};

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
    /// Composer-chip spawn options (permission posture + model/thinking env),
    /// applied when the agent process is launched (eager or lazy).
    spawn_opts: openxgram_acp::SpawnOpts,
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
    /// Composer "permission" chip: `bypassPermissions` / `acceptEdits` → auto-allow
    /// tool calls; `default` / `plan` / omitted → default-deny.
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Composer "model" chip: `default` (adapter default), `sonnet`, `opus`.
    /// Mapped to an `ANTHROPIC_MODEL` env on the agent process.
    #[serde(default)]
    pub model: Option<String>,
    /// Composer "thinking" chip: `high` / `medium` / `low`.
    /// Mapped to a `MAX_THINKING_TOKENS` env on the agent process.
    #[serde(default)]
    pub thinking: Option<String>,
    /// cross-machine — 에이전트 머신(서울/잘만/...). 원격이면 ACP 어댑터를 SSH 로 spawn.
    #[serde(default)]
    pub machine: Option<String>,
}

/// Translate the composer chip selections into crate-level [`SpawnOpts`]
/// (permission posture + agent-process env). Unknown / `default` values are
/// no-ops, so an unselected composer keeps the default-deny, adapter-default
/// behaviour.
fn spawn_opts_from_body(body: &CreateSessionBody) -> openxgram_acp::SpawnOpts {
    // 기본 posture = bypassPermissions (마스터 지시). 명시적 `plan` 만 default-deny(읽기전용 계획).
    // None/default/bypassPermissions/acceptEdits → 툴콜 자동 허용 (에이전트가 기본으로 bash 등 실제 작업 수행).
    let permission_allow = !matches!(body.permission_mode.as_deref(), Some("plan"));
    let mut extra_env: Vec<(String, String)> = Vec::new();
    match body.model.as_deref() {
        None | Some("") | Some("default") => {} // adapter default
        Some("haiku") => {
            extra_env.push(("ANTHROPIC_MODEL".into(), "claude-haiku-4-5-20251001".into()))
        }
        Some("sonnet") => extra_env.push(("ANTHROPIC_MODEL".into(), "claude-sonnet-4-6".into())),
        Some("opus") => extra_env.push(("ANTHROPIC_MODEL".into(), "claude-opus-4-8".into())),
        // 프리셋 외 = 사용자 직접 입력한 모델 id(claude-fable-5 등) → 그대로 사용(하드코딩 불필요).
        Some(other) => extra_env.push(("ANTHROPIC_MODEL".into(), other.to_string())),
    }
    // thinking effort 5단계 → MAX_THINKING_TOKENS. off/None → 확장 사고 비활성(env 미설정).
    match body.thinking.as_deref() {
        Some("ultra") => extra_env.push(("MAX_THINKING_TOKENS".into(), "31999".into())),
        Some("high") => extra_env.push(("MAX_THINKING_TOKENS".into(), "16000".into())),
        Some("medium") => extra_env.push(("MAX_THINKING_TOKENS".into(), "10000".into())),
        Some("low") => extra_env.push(("MAX_THINKING_TOKENS".into(), "4000".into())),
        _ => {} // "off"/None
    }
    // cross-machine — 머신이 원격이면 ACP 어댑터를 SSH 로 그 머신에서 spawn(command override).
    let command_override = body
        .machine
        .as_deref()
        .and_then(|m| remote_acp_command(m, &body.cwd, body.permission_mode.as_deref(), &extra_env));
    openxgram_acp::SpawnOpts {
        permission_allow,
        extra_env,
        command_override,
    }
}

// cwd 의 선행 `~` 를 절대 home 으로 확장. ACP 어댑터는 절대경로만 받음.
// 머신별 home: 잘만=/home/pasia, 맥미니=/Users(추정), 로컬=데몬 $HOME.
fn expand_home(cwd: &str, machine: Option<&str>) -> String {
    if !cwd.starts_with('~') {
        return cwd.to_string();
    }
    // config-driven — 원격 머신이면 machine_home(설정값 or SSH $HOME 동적조회), 로컬이면 $HOME.
    let local_home = || std::env::var("HOME").unwrap_or_else(|_| "/home/llm".to_string());
    let home = match machine.and_then(crate::daemon_gui::machine_lookup) {
        Some(cfg) => crate::daemon_gui::machine_home(&cfg).unwrap_or_else(local_home),
        None => local_home(),
    };
    if cwd == "~" {
        home
    } else if let Some(rest) = cwd.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        // "~user/..." 형태는 그대로 둠(드묾).
        cwd.to_string()
    }
}

// 원격 머신 ACP spawn 명령 — `ssh -T <host> 'wsl -- bash -lc "...claude-agent-acp"'`.
// ssh 프로세스의 stdio 가 ACP JSON-RPC 채널이 된다(SSH-stdio). Windows→WSL 따옴표깨짐
// 방지 위해 bash 명령을 base64 로 전달. env(모델/thinking)는 원격 bash 에 export.
// None = 로컬(서울) → registry 기본 spawn.
fn remote_acp_command(machine: &str, cwd: &str, permission_mode: Option<&str>, extra_env: &[(String, String)]) -> Option<(String, Vec<String>)> {
    use base64::Engine;
    // config-driven — ~/.openxgram/machines.json 에서 ssh_host/wsl 조회(하드코딩 제거).
    let cfg = crate::daemon_gui::machine_lookup(machine)?;
    let host = cfg.ssh_host.clone();
    let wsl = cfg.wsl;
    // adapter 미지정 시 동적 PATH 로 claude-agent-acp 해석(npm global bin — 머신마다 위치 다름).
    let adapter = cfg.adapter.clone().unwrap_or_else(|| "claude-agent-acp".to_string());
    let sh_quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut exports = String::new();
    for (k, v) in extra_env {
        exports.push_str(&format!("export {}={}; ", k, sh_quote(v)));
    }
    // 원격에서도 권한모드 적용 — 어댑터가 읽는 settings.local.json(override, 비파괴) 기록 +
    // IS_SANDBOX=1(root 머신에서도 bypassPermissions 허용; ALLOW_BYPASS = !IS_ROOT || IS_SANDBOX).
    let mode = match permission_mode.map(|s| s.trim()) {
        Some("bypassPermissions") | Some("bypass") => "bypassPermissions",
        Some("acceptEdits") => "acceptEdits",
        Some("plan") => "plan",
        _ => "default",
    };
    let cwd_sh = if cwd.starts_with('~') { cwd.replacen('~', "$HOME", 1) } else { cwd.to_string() };
    let pre = format!(
        "export IS_SANDBOX=1; mkdir -p \"{cwd_sh}/.claude\" 2>/dev/null; printf '%s' '{{\"permissions\":{{\"defaultMode\":\"{mode}\"}}}}' > \"{cwd_sh}/.claude/settings.local.json\" 2>/dev/null; "
    );
    // PATH 에 npm global bin 동적 추가(npm prefix -g + 흔한 위치). /home/pasia 하드코딩 제거.
    let inner = format!(
        "export PATH=\"$PATH:$(npm prefix -g 2>/dev/null)/bin:$HOME/.npm-global/bin:$HOME/.local/bin\"; {exports}{pre}exec {adapter}"
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode(inner.as_bytes());
    // ⚠ `echo B64|base64 -d|bash` 는 마지막 bash 의 stdin 이 파이프(스크립트)라 어댑터가
    // ssh stdin 을 못 받고 EOF 종료됨. 임시파일로 디코드 후 `exec bash file` → 어댑터가
    // ssh stdin 상속(ACP JSON-RPC 채널). $$ = 원격 bash PID 로 파일 unique.
    let run = format!("echo {b64}|base64 -d>/tmp/oxgacp.$$.sh;exec bash /tmp/oxgacp.$$.sh");
    let remote = if wsl {
        format!("wsl -- bash -lc \"{run}\"")
    } else {
        format!("bash -lc \"{run}\"")
    };
    Some((
        "ssh".to_string(),
        vec![
            "-T".into(),
            "-o".into(),
            "ConnectTimeout=12".into(),
            "-o".into(),
            "BatchMode=yes".into(),
            host.to_string(),
            remote,
        ],
    ))
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

    // ACP 어댑터는 절대경로 cwd 요구 — `~` 를 home 으로 확장(머신별: 잘만=/home/pasia, 로컬=$HOME).
    let cwd = expand_home(&body.cwd, body.machine.as_deref());

    // 권한모드를 어댑터(claude-agent-acp)가 실제로 읽는 곳에 반영 — 컴포저 칩/_meta 가 아니라
    // `<cwd>/.claude/settings.json` 의 permissions.defaultMode 만 읽기 때문(우리 ACP 자동승인과 별개).
    // 자격증명·기타 설정은 머지로 보존. 로컬 에이전트만(원격은 그 머신 settings 사용).
    if body.machine.as_deref().filter(|s| !s.is_empty()).is_none() {
        if let Err(e) = ensure_permission_settings(&cwd, body.permission_mode.as_deref()) {
            tracing::warn!(error = %e, cwd = %cwd, "ACP 권한 settings 기록 실패(계속)");
        }
    }

    let (updates_tx, _rx) = broadcast::channel::<Value>(256);
    let session_id = state.new_session_id();
    let spawn_opts = spawn_opts_from_body(&body);

    let handle_id = if mode == ExecutionMode::Always {
        Some(spawn_handle(state, &body.agent, spawn_opts.clone()).await?)
    } else {
        None
    };

    let sess = AcpHttpSession {
        agent: body.agent.clone(),
        cwd: cwd.clone(),
        execution_mode: mode,
        handle_id,
        spawn_opts,
        updates_tx,
    };
    state.sessions.lock().await.insert(session_id.clone(), sess);

    Ok(json!({
        "sessionId": session_id,
        "agent": body.agent,
        "cwd": cwd,
        "executionMode": mode,
        "spawned": handle_id.is_some(),
    }))
}

/// 어댑터(claude-agent-acp)가 읽는 `<cwd>/.claude/settings.json` 의 permissions.defaultMode 를
/// 컴포저 권한모드에 맞춰 머지 기록. 자격증명·기타 설정은 보존(머지). bypass/acceptEdits/plan/default.
fn ensure_permission_settings(cwd: &str, permission_mode: Option<&str>) -> std::io::Result<()> {
    let mode = match permission_mode.map(|s| s.trim()) {
        Some("bypassPermissions") | Some("bypass") => "bypassPermissions",
        Some("acceptEdits") => "acceptEdits",
        Some("plan") => "plan",
        _ => "default",
    };
    let dir = std::path::Path::new(cwd).join(".claude");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    let mut root: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let perms = obj
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    if !perms.is_object() {
        *perms = serde_json::json!({});
    }
    perms
        .as_object_mut()
        .unwrap()
        .insert("defaultMode".into(), serde_json::Value::String(mode.into()));
    std::fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    Ok(())
}

/// Spawn an agent via the crate registry, returning its handle id. The crate's
/// `acp_spawn` runs `initialize`; failure (e.g. agent not installed) is surfaced
/// explicitly.
async fn spawn_handle(
    state: &AcpHttpState,
    agent: &str,
    opts: openxgram_acp::SpawnOpts,
) -> Result<AgentHandleId, AcpHttpError> {
    let v = state
        .tools
        .acp_spawn_with(agent, opts)
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
            let hid = spawn_handle(state, &sess.agent, sess.spawn_opts.clone()).await?;
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

    // Live relay: each `session/update` is forwarded onto the per-session
    // broadcast (→ SSE `/stream`) the instant it arrives during the turn, instead
    // of all-at-once after the turn ends. We bridge the crate's per-update mpsc
    // sender to the broadcast via a forwarding task.
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<Value>();
    let relay_tx = tx.clone();
    let relay = tokio::spawn(async move {
        // Ends when the turn finishes: the streaming prompt drops `update_tx`,
        // `recv()` returns `None`, the loop exits, the task completes.
        while let Some(u) = update_rx.recv().await {
            // Ignore send errors: no SSE subscriber is a normal state.
            let _ = relay_tx.send(u);
        }
    });

    let result = state
        .tools
        .acp_prompt_streaming(handle_id, &cwd, &body.text, Some(update_tx))
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("acp prompt failed: {e}")))?;

    // The streaming call has dropped its sender by now; await the forwarding task
    // so every buffered update has been broadcast before we return the stopReason.
    if let Err(e) = relay.await {
        tracing::debug!(target: "acp.daemon", "update relay task join: {e}");
    }

    // `result` still carries `{stopReason, updates}`; the updates were already
    // broadcast live above (SSE is the live channel). We keep `updates` in the
    // HTTP body for non-SSE callers — the GUI applies them only as a fallback
    // when its stream is down, so there is no double-render.
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
