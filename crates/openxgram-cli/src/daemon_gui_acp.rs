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
    /// 대화 신원(에이전트 alias) — 세션 재사용(find-or-create) 키. UI 전환 후 복귀 시
    /// 같은 label 의 세션을 찾아 재연결한다. `None` 이면 picker 진입(재사용 안 함).
    label: Option<String>,
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
    /// 마지막 사용 시각 — A2A 지속 세션 idle TTL reaper 가 읽는다. create/prompt 시 갱신.
    /// 누수 방지 안전망(`reap_idle_a2a`)이 이 값으로 idle 초과 세션을 close 한다.
    last_used: std::time::Instant,
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
    /// 증분 영속용 DB 핸들 — 진행 중 툴 호출을 스트리밍 중 `acp_messages` 에 즉시 기록한다
    /// (나갔다 와도 실시간 단계 복원). `new()` 기본 None, `with_db()` 로 주입. None 이면 증분 skip.
    db: Option<Arc<Mutex<openxgram_db::Db>>>,
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
            db: None,
        }
    }

    /// 증분 영속용 DB 핸들 주입(GuiServerState 구성 시 1회). 진행 중 툴 호출 실시간 기록 활성화.
    pub fn with_db(mut self, db: Arc<Mutex<openxgram_db::Db>>) -> Self {
        self.db = Some(db);
        self
    }

    /// 임의 메시지를 `acp_messages` 에 기록 — ACP HTTP 핸들러(acp_session_prompt) 밖에서 생성된
    /// 대화(예: A2A 위임 교환)를 사용자 가시 스레드로 영속화한다. db 미주입(None)이면 no-op.
    pub async fn record_message(&self, conv_key: &str, role: &str, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let Some(db) = self.db.as_ref() else {
            return;
        };
        let now = chrono::Utc::now().to_rfc3339();
        let mut g = db.lock().await;
        if let Err(e) = g.conn().execute(
            "INSERT INTO acp_messages (conv_key, role, text, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![conv_key, role, text.trim(), now],
        ) {
            tracing::error!(target: "acp.daemon", conv_key = %conv_key, "a2a record_message 기록 실패: {e}");
        }
    }

    fn new_session_id(&self) -> String {
        let n = self
            .next_session
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("acp-{n}")
    }

    /// 세션의 대화 신원(`label` = conv_key). 데몬이 턴 결과를 권위있게
    /// `acp_messages` 에 기록할 때 사용한다(UI 가 turn 중/후 이탈해도 영속화 보장).
    /// `None` 이면 picker 진입 등 비영속 세션 — 기록하지 않는다.
    pub async fn session_label(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .and_then(|s| s.label.clone())
            .filter(|l| !l.is_empty())
    }

    /// 살아있는 ACP 세션 목록을 `(sessionId, label, cwd)` 로 반환 — A2A `existing_acp`
    /// 엔드포인트 조회(`list_agent_endpoints`)에서 label==alias 세션을 고르는 데 쓴다.
    /// label 없는(picker 진입 등) 세션은 빈 문자열로 노출되니 호출자가 필터한다.
    pub async fn list_sessions_brief(&self) -> Vec<(String, String, String)> {
        let sessions = self.sessions.lock().await;
        sessions
            .iter()
            .map(|(id, s)| (id.clone(), s.label.clone().unwrap_or_default(), s.cwd.clone()))
            .collect()
    }

    /// 세션이 현재 살아있는지(레지스트리에 존재) — A2A 지속 세션 resume 판정용.
    /// `body.session_id` 가 살아있으면 create 생략하고 그 세션에 이어 prompt 한다.
    pub async fn session_alive(&self, session_id: &str) -> bool {
        self.sessions.lock().await.contains_key(session_id)
    }

    /// A2A 지속 세션 idle TTL 안전망 — label 이 `a2a:` 로 시작하는(친구 대화) 세션 중
    /// 마지막 사용 이후 `idle` 초과한 것을 close 한다. 누수 방지 reaper 가 주기적으로 호출.
    /// close 자체에 `self`(state)가 필요하므로 reap 대상 id 만 lock 안에서 모으고 lock 해제 후 close.
    pub async fn reap_idle_a2a(&self, idle: std::time::Duration) {
        let now = std::time::Instant::now();
        let stale: Vec<String> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .filter(|(_, s)| {
                    s.label
                        .as_deref()
                        .map(|l| l.starts_with("a2a:"))
                        .unwrap_or(false)
                        && now.duration_since(s.last_used) >= idle
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for sid in stale {
            match close(self, &sid).await {
                Ok(_) => tracing::info!(target: "acp.daemon", session = %sid, "a2a idle reaper: 지속 세션 close(idle TTL 초과)"),
                Err(e) => tracing::debug!(target: "acp.daemon", session = %sid, "a2a idle reaper close 실패(계속): {e:?}"),
            }
        }
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
    /// 대화 신원(에이전트 alias). 세션 지속(find-or-create) 키 — 같은 label 이면 기존 세션 재연결.
    /// 어댑터+cwd 가 아닌 신원으로 키잉해야 cwd 공유 에이전트 간 세션 병합을 막는다.
    #[serde(default)]
    pub label: Option<String>,
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
        // 프리셋 외 = 드롭다운(OpenRouter 목록)/직접 입력한 모델 id(claude-fable-5 등).
        // OpenRouter 표기는 버전에 점을 쓰지만(claude-opus-4.8), Claude Code 구독은 하이픈
        // id(claude-opus-4-8)만 받는다 → claude-* 모델은 점→하이픈 정규화 후 주입.
        // (점 형식 그대로면 "selected model may not exist" 에러. 비-claude id 는 손대지 않음.)
        Some(other) => {
            let norm = if other.starts_with("claude") {
                other.replace('.', "-")
            } else {
                other.to_string()
            };
            extra_env.push(("ANTHROPIC_MODEL".into(), norm));
        }
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

    // 세션 지속 — 같은 에이전트(label=대화 신원)의 살아있는 세션이 있으면 재사용(find-or-create).
    // UI 가 다른 대화로 전환했다 돌아와도 같은 sessionId 로 재연결되어, 시켜둔 작업이 안 멈춘다.
    // 키는 반드시 에이전트 신원(label) — adapter+cwd 만으로 키잉하면 cwd 를 공유하는(빈 cwd 다수)
    // 서로 다른 에이전트가 한 세션으로 병합되는 사고가 난다. label 미지정(picker 진입)이면 재사용 안 함.
    if let Some(lbl) = body.label.as_deref().filter(|s| !s.is_empty()) {
        let mut sessions = state.sessions.lock().await;
        let reuse = sessions
            .iter()
            .find(|(_, s)| s.label.as_deref() == Some(lbl))
            .map(|(sid, _)| sid.clone());
        if let Some(sid) = reuse {
            if let Some(s) = sessions.get_mut(&sid) {
                // idle reaper 안전망 갱신 — 재사용도 활성 사용으로 본다.
                s.last_used = std::time::Instant::now();
                return Ok(json!({
                    "sessionId": sid,
                    "agent": s.agent,
                    "cwd": s.cwd,
                    "executionMode": s.execution_mode,
                    "spawned": s.handle_id.is_some(),
                    "reused": true,
                }));
            }
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
        label: body.label.clone(),
        execution_mode: mode,
        handle_id,
        spawn_opts,
        updates_tx,
        last_used: std::time::Instant::now(),
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
    let (handle_id, cwd, tx, conv_key) = {
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
        // idle TTL reaper 안전망용 — 매 prompt 마다 마지막 사용 시각 갱신.
        sess.last_used = std::time::Instant::now();
        // label = conv_key(대화 신원). 증분 툴 기록 + 맥락 복원 대상 키. picker(label 없음)면 None.
        (hid, sess.cwd.clone(), sess.updates_tx.clone(), sess.label.clone())
    };

    // 🔑 데몬측 맥락 복원 — 어댑터는 매 프롬프트마다 session/new(새 Claude Code 세션, 무상태)를 연다.
    // 즉 턴 간 메모리가 없다. 따라서 매 프롬프트에 그 conv_key 의 DB 기록을 prepend 해야 에이전트가
    // 맥락을 갖는다(stateless chat 모델 — 매 턴 전체 히스토리 전송). UI pendingContext 의존 제거.
    let restored = match (state.db.as_ref(), conv_key.as_deref()) {
        (Some(db), Some(key)) => build_resume_preamble(db, key, &body.text).await,
        _ => None,
    };
    let prompt_text: String = match restored {
        Some(preamble) => format!("{preamble}{}", body.text),
        None => body.text.clone(),
    };

    // Live relay: each `session/update` is forwarded onto the per-session
    // broadcast (→ SSE `/stream`) the instant it arrives during the turn, instead
    // of all-at-once after the turn ends. We bridge the crate's per-update mpsc
    // sender to the broadcast via a forwarding task.
    let (update_tx, mut update_rx) = mpsc::unbounded_channel::<Value>();
    let relay_tx = tx.clone();
    let rec_db = state.db.clone(); // 증분 영속용 DB(Option). None 이면 기록 skip.
    let rec_key = conv_key.clone(); // conv_key(label). None(picker)이면 skip.
    let relay = tokio::spawn(async move {
        // Ends when the turn finishes: the streaming prompt drops `update_tx`,
        // `recv()` returns `None`, the loop exits, the task completes.
        while let Some(u) = update_rx.recv().await {
            // 증분 영속 — 진행 중 툴 호출을 즉시 acp_messages 에 기록(나갔다 와도 실시간 단계 복원).
            if let (Some(db), Some(key)) = (rec_db.as_ref(), rec_key.as_ref()) {
                record_stream_tool(db, key, &u).await;
            }
            // Ignore send errors: no SSE subscriber is a normal state.
            let _ = relay_tx.send(u);
        }
    });

    let result = state
        .tools
        .acp_prompt_streaming(handle_id, &cwd, &prompt_text, Some(update_tx))
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

/// 세션이 새로 spawn 될 때(was_fresh) 첫 프롬프트에 prepend 할 이전 대화 맥락을 DB 에서 구성.
/// `acp_messages` 의 me/agent 행만 사용(tool/plan/note 는 노이즈라 제외). 최근 ~20k char 만(토큰 보호).
/// 기록이 없으면 None. 이게 UI 의존 재주입(pendingContext)을 대체하는 데몬 권위 맥락 복원의 핵심.
async fn build_resume_preamble(
    db: &Arc<Mutex<openxgram_db::Db>>,
    conv_key: &str,
    current: &str,
) -> Option<String> {
    let mut rows: Vec<(String, String)> = {
        let mut g = db.lock().await;
        let conn = g.conn();
        let mut stmt = conn
            .prepare("SELECT role, text FROM acp_messages WHERE conv_key = ?1 ORDER BY id")
            .ok()?;
        let collected: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![conv_key], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .ok()?
            .filter_map(|x| x.ok())
            .collect();
        collected
    };
    // 마지막 행이 현재 프롬프트(UI 가 전송 직전 기록한 'me')면 제외 — '현재 요청'으로 따로 붙어 중복되니까.
    if matches!(rows.last(), Some((role, text)) if role.as_str() == "me" && text.trim() == current.trim()) {
        rows.pop();
    }
    let mut lines: Vec<String> = Vec::new();
    for (role, text) in &rows {
        match role.as_str() {
            "me" => lines.push(format!("사용자: {text}")),
            "agent" => lines.push(format!("너(에이전트): {text}")),
            _ => {} // tool/plan/note 제외
        }
    }
    if lines.is_empty() {
        return None;
    }
    let mut body = lines.join("\n");
    let total = body.chars().count();
    if total > 20000 {
        let tail: String = body.chars().skip(total - 20000).collect();
        body = format!("…(이전 일부 생략)\n{tail}");
    }
    Some(format!(
        "[이전 대화 맥락 — 너는 이 대화를 이어가는 중이다. 아래는 우리의 지난 대화다.]\n{body}\n[위 맥락을 모두 기억하고, 아래 현재 요청에 이어서 답하라.]\n\n현재 요청: "
    ))
}

/// 진행 중 턴의 `tool_call`/`tool_call_update` 를 `acp_messages` 에 증분 기록한다.
/// - tool_call → INSERT(role='tool', text=`{"tid","title","status"}`).
/// - tool_call_update → tid 매칭으로 status in-place 갱신(json_set).
/// 최종 답변·계획은 턴 끝(daemon_gui.rs)에서 기록. 영속 실패는 조용히 무시(턴 흐름 안 막음).
async fn record_stream_tool(db: &Arc<Mutex<openxgram_db::Db>>, conv_key: &str, u: &Value) {
    match u.get("sessionUpdate").and_then(|s| s.as_str()) {
        Some("tool_call") => {
            let tid = u.get("toolCallId").and_then(|v| v.as_str()).unwrap_or("");
            let title = u
                .get("title")
                .and_then(|v| v.as_str())
                .or_else(|| u.get("kind").and_then(|v| v.as_str()))
                .unwrap_or("tool");
            let status = u.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
            let text = serde_json::json!({ "tid": tid, "title": title, "status": status }).to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let mut g = db.lock().await;
            let _ = g.conn().execute(
                "INSERT INTO acp_messages (conv_key, role, text, created_at) VALUES (?1, 'tool', ?2, ?3)",
                rusqlite::params![conv_key, text, now],
            );
        }
        Some("tool_call_update") => {
            let tid = u.get("toolCallId").and_then(|v| v.as_str()).unwrap_or("");
            if tid.is_empty() {
                return;
            }
            if let Some(st) = u.get("status").and_then(|v| v.as_str()) {
                let mut g = db.lock().await;
                let _ = g.conn().execute(
                    "UPDATE acp_messages SET text = json_set(text, '$.status', ?1) \
                     WHERE conv_key = ?2 AND role = 'tool' AND json_extract(text, '$.tid') = ?3",
                    rusqlite::params![st, conv_key, tid],
                );
            }
        }
        _ => {}
    }
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

/// 턴이 끝나 agent 응답이 `acp_messages` 에 영속된 직후 호출. 세션 broadcast 채널에
/// `conv_persisted` 마커를 쏜다 → SSE 로 연결된 모든 클라이언트가 권위 소스(DB)에서
/// 재동기화(loadHistory)하게 한다. 핵심: 사용자가 턴 도중/직후 다른 창에 갔다 와도
/// (loadHistory 가 1회성이라 놓치던) 완료 답변이 화면에 반드시 뜬다. 구독자 없으면 no-op.
pub async fn notify_conv_persisted(state: &AcpHttpState, session_id: &str) {
    let sessions = state.sessions.lock().await;
    if let Some(sess) = sessions.get(session_id) {
        let _ = sess
            .updates_tx
            .send(serde_json::json!({ "sessionUpdate": "conv_persisted" }));
    }
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
