//! [`AcpClient`] — owns one spawned ACP agent process and drives it (§2.3).
//!
//! Lifecycle:
//!   1. [`AcpClient::spawn`] — spawn the subprocess, start transport + peer,
//!      run `initialize` (version negotiation, store `agentCapabilities`).
//!   2. [`AcpClient::session_new`] — `session/new` → an [`AcpSession`].
//!   3. [`AcpClient::prompt`] / [`AcpClient::cancel`] — drive turns.
//!   4. [`AcpClient::close`] — graceful kill + zombie reap (§5 process lifecycle).
//!
//! Runtime hazards (§2.4 / §5): the reader/writer tasks are `tokio::spawn`ed on
//! the *outer* runtime so the agent outlives any single `block_on`. This struct
//! keeps the `Child` so the daemon registry (B-2) can hold it long-lived.

use std::sync::Arc;

use tokio::process::Child;

use crate::handlers::{ClientSideHandlers, DefaultHandlers, PermissionDecision};
use crate::registry::AgentSpec;
use crate::rpc::RpcPeer;
use crate::session::{AcpSession, PromptResult};
use crate::transport::{self, ChildPipes};
use crate::types::{
    AgentCapabilities, ClientCapabilities, ClientInfo, ContentBlock, InitializeRequest,
    InitializeResponse, SessionNewResponse,
};
use crate::{AcpError, Result, PROTOCOL_VERSION};

/// A live ACP client bound to one agent subprocess.
pub struct AcpClient {
    child: Child,
    peer: RpcPeer,
    agent_caps: AgentCapabilities,
}

impl AcpClient {
    /// Spawn the agent and run `initialize`.
    ///
    /// `client_info` identifies us to the agent; `client_caps` is what we
    /// advertise (must match what `handlers` will actually accept — capability
    /// honesty §6). On a protocol-version mismatch this fails loud with
    /// [`AcpError::InitFailed`] (no silent downgrade).
    pub async fn spawn(
        agent: AgentSpec,
        client_info: ClientInfo,
        client_caps: ClientCapabilities,
        handlers: Arc<dyn ClientSideHandlers>,
    ) -> Result<AcpClient> {
        let ChildPipes {
            child,
            stdin,
            stdout,
        } = transport::spawn_agent(&agent.command, &agent.args, &agent.env, None)?;

        let writer = transport::spawn_writer(stdin);
        let reader = transport::spawn_reader(stdout);
        let peer = RpcPeer::start(writer, reader, handlers);

        let init = InitializeRequest {
            protocol_version: PROTOCOL_VERSION,
            client_capabilities: client_caps,
            client_info,
        };
        let params = serde_json::to_value(&init)?;
        let raw = peer.request("initialize", params).await?;
        let resp: InitializeResponse = serde_json::from_value(raw)?;

        if resp.protocol_version != PROTOCOL_VERSION {
            return Err(AcpError::InitFailed {
                got: resp.protocol_version,
                want: PROTOCOL_VERSION,
            });
        }
        if !resp.auth_methods.is_empty() {
            return Err(AcpError::AuthRequired);
        }

        Ok(AcpClient {
            child,
            peer,
            agent_caps: resp.agent_capabilities,
        })
    }

    /// Convenience: spawn with default (minimal) client info and default-deny
    /// handlers, advertising no callbacks. Useful for the simplest prompt flow.
    pub async fn spawn_minimal(agent: AgentSpec) -> Result<AcpClient> {
        Self::spawn(
            agent,
            Self::default_client_info(),
            ClientCapabilities::default(),
            DefaultHandlers::shared(),
        )
        .await
    }

    /// Like [`AcpClient::spawn_minimal`] but installs an auto-allow permission
    /// handler — `session/request_permission` is approved (first offered option)
    /// instead of cancelled. Used for "Bypass Permissions" mode so the agent can
    /// actually execute its own tool calls.
    pub async fn spawn_allow(agent: AgentSpec) -> Result<AcpClient> {
        let handlers: Arc<dyn ClientSideHandlers> =
            Arc::new(DefaultHandlers::new().with_permission(PermissionDecision::Allow));
        Self::spawn(
            agent,
            Self::default_client_info(),
            ClientCapabilities::default(),
            handlers,
        )
        .await
    }

    /// The `ClientInfo` we advertise on `initialize`.
    fn default_client_info() -> ClientInfo {
        ClientInfo {
            name: "openxgram".into(),
            title: Some("OpenXgram".into()),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// The agent capabilities negotiated at `initialize`.
    pub fn agent_capabilities(&self) -> &AgentCapabilities {
        &self.agent_caps
    }

    /// Open a new session via `session/new`.
    pub async fn session_new(
        &self,
        cwd: impl Into<String>,
        mcp_servers: Vec<serde_json::Value>,
    ) -> Result<AcpSession> {
        let cwd = cwd.into();
        let params = serde_json::json!({ "cwd": cwd, "mcpServers": mcp_servers });
        let raw = self.peer.request("session/new", params).await?;
        let resp: SessionNewResponse = serde_json::from_value(raw)?;
        Ok(AcpSession::new(resp.session_id, cwd, self.peer.clone()).await)
    }

    /// One-shot helper: open a session, run a single prompt turn, return the
    /// result. The session is dropped (and its listener cleaned up) after.
    pub async fn prompt(
        &self,
        cwd: impl Into<String>,
        blocks: Vec<ContentBlock>,
    ) -> Result<PromptResult> {
        self.prompt_streaming(cwd, blocks, None).await
    }

    /// One-shot helper that additionally forwards each `session/update` **live**
    /// (as it arrives, before the turn's `stopReason`) to `on_update`.
    ///
    /// The returned [`PromptResult`] is identical to [`AcpClient::prompt`] — the
    /// live forwarding is purely additive. The `on_update` channel closes when
    /// this future resolves (turn end): the caller drops the sender, the receiver
    /// observes the end of the live stream.
    pub async fn prompt_streaming(
        &self,
        cwd: impl Into<String>,
        blocks: Vec<ContentBlock>,
        on_update: Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>,
    ) -> Result<PromptResult> {
        let mut session = self.session_new(cwd, Vec::new()).await?;
        session.prompt_streaming(blocks, on_update).await
    }

    /// Send `session/cancel` for a session id (notification).
    pub fn cancel(&self, session_id: &str) -> Result<()> {
        self.peer.notify(
            "session/cancel",
            serde_json::json!({ "sessionId": session_id }),
        )
    }

    /// Graceful shutdown: drop stdin (via peer writer going away is handled by
    /// the daemon), kill the child, and reap it so no zombie remains (§5).
    pub async fn close(mut self) -> Result<()> {
        // `kill_on_drop(true)` is set, but we kill+wait explicitly to reap the
        // process now rather than at an indeterminate Drop point.
        if let Err(e) = self.child.start_kill() {
            tracing::debug!(target: "acp.client", "start_kill: {e}");
        }
        match self.child.wait().await {
            Ok(_status) => Ok(()),
            Err(e) => Err(AcpError::Io(e)),
        }
    }
}
