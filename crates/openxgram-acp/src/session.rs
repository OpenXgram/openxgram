//! Per-session state and the prompt turn (§1.7, §2.3).
//!
//! An [`AcpSession`] holds the negotiated `sessionId`, its `cwd`, and a handle
//! back to the shared [`RpcPeer`]. `prompt` drives one turn: it sends
//! `session/prompt`, drains `session/update` notifications into `updates`, and
//! resolves with the `stopReason` the agent returns at turn end.

use serde_json::json;
use tokio::sync::mpsc;

use crate::rpc::RpcPeer;
use crate::types::{SessionPromptResponse, SessionUpdate, StopReason};
use crate::{AcpError, Result};

/// Parse one raw `session/update` params object (`{sessionId, update:{…}}`) into
/// a [`SessionUpdate`].
///
/// The discriminator (`sessionUpdate`) is nested under `update` on the real ACP
/// wire, so we deserialize that sub-object — not the whole params. Unparseable
/// frames are surfaced (logged with the raw discriminator) and skipped rather
/// than silently dropped (절대 규칙 1). Forward-compat: an *unknown* discriminator
/// still deserializes via [`SessionUpdate::Unknown`] and is logged.
fn parse_update(raw: &serde_json::Value) -> Option<SessionUpdate> {
    // Real wire shape: params = { sessionId, update: { sessionUpdate, … } }.
    // Tolerate a flat shape too (params already being the update body) for
    // robustness against transports that pre-unwrap.
    let body = raw.get("update").unwrap_or(raw);
    match serde_json::from_value::<SessionUpdate>(body.clone()) {
        Ok(SessionUpdate::Unknown) => {
            let disc = body
                .get("sessionUpdate")
                .and_then(|d| d.as_str())
                .unwrap_or("<missing>");
            tracing::warn!(
                target: "acp.session",
                "unknown session/update discriminator '{disc}' surfaced (not dropped): {body}"
            );
            Some(SessionUpdate::Unknown)
        }
        Ok(u) => Some(u),
        Err(e) => {
            tracing::warn!(
                target: "acp.session",
                "unparseable session/update skipped: {e}; raw={raw}"
            );
            None
        }
    }
}

/// Result of a completed prompt turn.
#[derive(Debug, Clone)]
pub struct PromptResult {
    /// Why the turn ended.
    pub stop_reason: StopReason,
    /// All `session/update` notifications observed during the turn, in order.
    pub updates: Vec<SessionUpdate>,
}

/// A live ACP session bound to one agent process.
pub struct AcpSession {
    session_id: String,
    cwd: String,
    peer: RpcPeer,
    updates: mpsc::UnboundedReceiver<serde_json::Value>,
}

impl AcpSession {
    /// Construct a session view. Registers a notification listener with the
    /// peer for this `sessionId`.
    pub(crate) async fn new(session_id: String, cwd: String, peer: RpcPeer) -> Self {
        let updates = peer.register_listener(session_id.clone()).await;
        Self {
            session_id,
            cwd,
            peer,
            updates,
        }
    }

    /// The session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The session working directory.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Run one prompt turn to completion.
    ///
    /// Sends `session/prompt` and concurrently drains `session/update`
    /// notifications until the prompt response (the `stopReason`) arrives. Full-
    /// duplex safe (§6): the update drain and the response await run on the same
    /// task via `tokio::select!`, never blocking the reader loop.
    pub async fn prompt(
        &mut self,
        blocks: Vec<crate::types::ContentBlock>,
    ) -> Result<PromptResult> {
        let params = json!({
            "sessionId": self.session_id,
            "prompt": blocks,
        });

        // Issue the request; its future resolves at turn end.
        let peer = self.peer.clone();
        let request = peer.request("session/prompt", params);
        tokio::pin!(request);

        let mut collected: Vec<SessionUpdate> = Vec::new();

        loop {
            tokio::select! {
                // Prompt resolved → turn ended.
                resp = &mut request => {
                    let value = resp?;
                    let parsed: SessionPromptResponse = serde_json::from_value(value)
                        .map_err(AcpError::Serde)?;
                    // Drain any remaining buffered updates without blocking, so
                    // updates that arrived just before the stopReason are not lost.
                    while let Ok(raw) = self.updates.try_recv() {
                        if let Some(u) = parse_update(&raw) {
                            collected.push(u);
                        }
                    }
                    return Ok(PromptResult {
                        stop_reason: parsed.stop_reason,
                        updates: collected,
                    });
                }
                // A streaming update arrived.
                maybe = self.updates.recv() => {
                    match maybe {
                        Some(raw) => {
                            // Forward-compat: skip updates we cannot parse rather
                            // than aborting the turn, but never the response path.
                            // The discriminator is nested under `update` on the
                            // wire — `parse_update` handles the unwrap + logging.
                            if let Some(u) = parse_update(&raw) {
                                collected.push(u);
                            }
                        }
                        None => {
                            // Listener closed but the prompt has not resolved.
                            // Keep awaiting the response on the next loop turn.
                        }
                    }
                }
            }
        }
    }

    /// Send a `session/cancel` notification. The agent must eventually resolve
    /// the in-flight `session/prompt` with `StopReason::Cancelled` (§1.6) — it
    /// must NOT surface as a JSON-RPC error.
    pub fn cancel(&self) -> Result<()> {
        self.peer
            .notify("session/cancel", json!({ "sessionId": self.session_id }))
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        // Best-effort listener cleanup. We cannot await in Drop; the peer holds
        // an Arc map, so spawn a detached removal.
        let peer = self.peer.clone();
        let sid = self.session_id.clone();
        tokio::spawn(async move {
            peer.drop_listener(&sid).await;
        });
    }
}
