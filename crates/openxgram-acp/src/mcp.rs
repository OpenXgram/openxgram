//! MCP tool methods — `AcpTools` (§2.2, §3.1).
//!
//! These are the domain methods (`acp_spawn`, `acp_prompt`, `acp_cancel`,
//! `acp_list_agents`, `acp_close`) that `openxgram-cli/src/mcp_serve.rs` will
//! wrap into the MCP JSON-RPC dispatch (with the `block_in_place(|| handle.
//! block_on(...))` bridge) in Phase B-2. This crate provides ONLY the tool
//! methods + the in-process agent registry; no MCP/JSON-RPC framing here.
//!
//! Long-lived spawned agents are held in a `HashMap<handle_id, AcpClient>`
//! behind an async `Mutex`, so they survive between separate MCP calls (§3.1 /
//! §5 — agents must outlive a single request frame). In B-1 this registry lives
//! in `AcpTools`; B-2 moves ownership into the daemon while keeping this API.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::client::AcpClient;
use crate::registry::{self, KNOWN_AGENTS};
use crate::types::ContentBlock;
use crate::{AcpError, Result};

/// Opaque handle id for a spawned agent.
pub type AgentHandleId = u64;

/// Per-spawn options threaded from the GUI composer chips (permission mode,
/// model, thinking). `extra_env` is appended to the agent's [`registry`] env
/// (e.g. `ANTHROPIC_MODEL`, `MAX_THINKING_TOKENS`) before the process is
/// launched; `permission_allow` swaps the default-deny permission handler for
/// an auto-allow one so the agent can actually execute its own tools.
#[derive(Debug, Clone, Default)]
pub struct SpawnOpts {
    /// `true` → auto-approve `session/request_permission` (Bypass Permissions).
    /// `false` (default) → default-deny posture (tool calls are cancelled).
    pub permission_allow: bool,
    /// Extra env pairs appended to the agent process environment.
    pub extra_env: Vec<(String, String)>,
    /// Cross-machine 원격 실행 — spawn 명령(command, args)을 registry spec 대신 override.
    /// 예: `("ssh", ["-T", "zalman", "wsl -- bash -lc \"...claude-agent-acp\""])`.
    /// 설정 시 env 는 원격 명령에 포함되므로 로컬 spec.env 는 건드리지 않는다.
    pub command_override: Option<(String, Vec<String>)>,
}

/// Stateful ACP tool surface: owns the spawned-agent registry.
#[derive(Clone, Default)]
pub struct AcpTools {
    inner: Arc<AcpToolsInner>,
}

#[derive(Default)]
struct AcpToolsInner {
    next_id: AtomicU64,
    agents: Mutex<HashMap<AgentHandleId, AcpClient>>,
}

impl AcpTools {
    /// New empty tool surface.
    pub fn new() -> Self {
        Self::default()
    }

    /// `acp_list_agents` — the built-in registry of known agent names.
    pub fn acp_list_agents(&self) -> Value {
        json!({ "agents": KNOWN_AGENTS })
    }

    /// `acp_spawn` — spawn a known agent by name, run `initialize`, register it,
    /// and return its handle id + negotiated capabilities. Uses the default
    /// (default-deny, no extra env) posture; see [`AcpTools::acp_spawn_with`] to
    /// thread composer-chip options.
    pub async fn acp_spawn(&self, agent_name: &str) -> Result<Value> {
        self.acp_spawn_with(agent_name, SpawnOpts::default()).await
    }

    /// `acp_spawn_with` — like [`AcpTools::acp_spawn`] but applies [`SpawnOpts`]:
    /// appends `extra_env` to the agent process env and, when `permission_allow`
    /// is set, installs an auto-allow permission handler so the agent's own tool
    /// calls are approved instead of cancelled.
    pub async fn acp_spawn_with(&self, agent_name: &str, opts: SpawnOpts) -> Result<Value> {
        let mut spec = registry::lookup(agent_name)?;
        if let Some((cmd, args)) = opts.command_override {
            // 원격 실행 — command/args override. env 는 원격 명령에 baked-in.
            spec.command = cmd;
            spec.args = args;
        } else {
            for (k, v) in &opts.extra_env {
                spec.env.push((k.clone(), v.clone()));
            }
        }
        let client = if opts.permission_allow {
            AcpClient::spawn_allow(spec).await?
        } else {
            AcpClient::spawn_minimal(spec).await?
        };
        let caps = serde_json::to_value(client.agent_capabilities())?;

        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        self.inner.agents.lock().await.insert(id, client);

        Ok(json!({
            "handleId": id,
            "agent": agent_name,
            "agentCapabilities": caps,
        }))
    }

    /// `acp_prompt` — run one prompt turn against a spawned agent. `text` is sent
    /// as a single text ContentBlock; `cwd` is the session working directory.
    ///
    /// Non-streaming: the returned `{stopReason, updates}` is delivered only at
    /// turn end. Used by the MCP JSON-RPC path (`mcp_serve.rs`), which has no live
    /// channel. For live per-chunk delivery use [`AcpTools::acp_prompt_streaming`].
    pub async fn acp_prompt(
        &self,
        handle_id: AgentHandleId,
        cwd: &str,
        text: &str,
    ) -> Result<Value> {
        self.acp_prompt_streaming(handle_id, cwd, text, None).await
    }

    /// `acp_prompt_streaming` — like [`AcpTools::acp_prompt`] but forwards each
    /// `session/update` body to `on_update` **live**, as it arrives during the
    /// turn (before the `stopReason`). The returned value is identical to
    /// `acp_prompt` (`{stopReason, updates}`) so non-SSE callers still get the
    /// full collected list; the live forwarding is purely additive.
    pub async fn acp_prompt_streaming(
        &self,
        handle_id: AgentHandleId,
        cwd: &str,
        text: &str,
        on_update: Option<tokio::sync::mpsc::UnboundedSender<Value>>,
    ) -> Result<Value> {
        let agents = self.inner.agents.lock().await;
        let client = agents.get(&handle_id).ok_or(AcpError::SessionClosed)?;
        let result = client
            .prompt_streaming(cwd.to_string(), vec![ContentBlock::text(text)], on_update)
            .await?;
        let updates = serde_json::to_value(&result.updates)?;
        let stop_reason = serde_json::to_value(result.stop_reason)?;
        Ok(json!({
            "stopReason": stop_reason,
            "updates": updates,
        }))
    }

    /// `acp_cancel` — send `session/cancel` for a session on a spawned agent.
    pub async fn acp_cancel(&self, handle_id: AgentHandleId, session_id: &str) -> Result<Value> {
        let agents = self.inner.agents.lock().await;
        let client = agents.get(&handle_id).ok_or(AcpError::SessionClosed)?;
        client.cancel(session_id)?;
        Ok(json!({ "cancelled": true }))
    }

    /// `acp_close` — kill + reap a spawned agent, removing it from the registry.
    pub async fn acp_close(&self, handle_id: AgentHandleId) -> Result<Value> {
        let client = self.inner.agents.lock().await.remove(&handle_id);
        match client {
            Some(c) => {
                c.close().await?;
                Ok(json!({ "closed": true, "handleId": handle_id }))
            }
            None => Err(AcpError::SessionClosed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_agents_includes_known() {
        let t = AcpTools::new();
        let v = t.acp_list_agents();
        let arr = v["agents"].as_array().expect("array");
        assert!(arr.iter().any(|a| a == "claude-agent-acp"));
    }

    #[tokio::test]
    async fn spawn_unknown_agent_errors() {
        let t = AcpTools::new();
        let err = t.acp_spawn("totally-unknown").await.unwrap_err();
        matches!(err, AcpError::UnknownAgent(_));
    }

    #[tokio::test]
    async fn prompt_unknown_handle_errors() {
        let t = AcpTools::new();
        let err = t.acp_prompt(999, "/tmp", "hi").await.unwrap_err();
        matches!(err, AcpError::SessionClosed);
    }
}
