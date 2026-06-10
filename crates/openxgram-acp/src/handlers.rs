//! Client-side callback handlers (§1.3, §2.3, §3.3).
//!
//! ACP agents call back into the client mid-turn:
//!   - `fs/read_text_file`        → `{ content }`
//!   - `fs/write_text_file`       → `{}`
//!   - `session/request_permission` → `{ outcome: { ... } }`
//!   - `terminal/*`               → (only if `terminal` capability advertised)
//!
//! Capability honesty (§6): the client must never *accept* a method whose
//! capability it did not advertise. The default impl here is **default-deny /
//! minimal** — it does not advertise `fs` or `terminal`, and denies permission
//! requests. Real fs + permission policy is Phase 4 (B-2+), not B-1.
//!
//! 절대 규칙 1 (fallback 금지): an unhandled method returns an explicit
//! [`AcpError::Protocol`] — never a silent empty success.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::{AcpError, Result};

/// Outcome of a `session/request_permission` callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Grant the requested option (the first/preferred one).
    Allow,
    /// Refuse.
    Deny,
}

/// What OpenXgram exposes back to the spawned ACP agent.
///
/// Implementors decide the policy. Object-safe (`async_trait`-free via boxed
/// futures) so it can live behind `Arc<dyn ClientSideHandlers>`.
pub trait ClientSideHandlers: Send + Sync {
    /// Dispatch one inbound agent→client request by method name. Returns the
    /// JSON `result` payload, or an error which the peer encodes as a JSON-RPC
    /// error response.
    fn handle<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>;

    /// Capabilities to advertise on `initialize`. The default impl reflects the
    /// MVP posture (no fs, no terminal). Override to enable callbacks.
    fn advertised_fs_read(&self) -> bool {
        false
    }

    /// Whether `fs/write_text_file` is advertised.
    fn advertised_fs_write(&self) -> bool {
        false
    }

    /// Whether `terminal/*` is advertised.
    fn advertised_terminal(&self) -> bool {
        false
    }
}

/// Default-deny minimal handler set (B-1 posture).
///
/// - advertises no `fs` / no `terminal`,
/// - denies every `session/request_permission`,
/// - rejects `fs/*` and `terminal/*` as not-advertised (capability honesty),
/// - returns an explicit protocol error for unknown methods.
#[derive(Debug, Clone)]
pub struct DefaultHandlers {
    permission: PermissionDecision,
}

impl Default for DefaultHandlers {
    fn default() -> Self {
        Self {
            permission: PermissionDecision::Deny,
        }
    }
}

impl DefaultHandlers {
    /// New default-deny handlers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the permission decision (e.g. allow-once for tests / dev).
    pub fn with_permission(mut self, decision: PermissionDecision) -> Self {
        self.permission = decision;
        self
    }

    /// Convenience constructor for an `Arc<dyn ClientSideHandlers>`.
    pub fn shared() -> Arc<dyn ClientSideHandlers> {
        Arc::new(Self::default())
    }

    fn handle_permission(&self, params: &Value) -> Result<Value> {
        match self.permission {
            PermissionDecision::Deny => Ok(json!({ "outcome": { "outcome": "cancelled" } })),
            PermissionDecision::Allow => {
                // Pick the first offered option id if present.
                let option_id = params
                    .get("options")
                    .and_then(|o| o.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|opt| opt.get("optionId"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("allow");
                Ok(json!({
                    "outcome": { "outcome": "selected", "optionId": option_id }
                }))
            }
        }
    }
}

impl ClientSideHandlers for DefaultHandlers {
    fn handle<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            match method {
                "session/request_permission" => self.handle_permission(&params),
                // Not advertised in B-1 → reject loudly (capability honesty).
                "fs/read_text_file" | "fs/write_text_file" => Err(AcpError::Protocol(format!(
                    "method {method} not advertised by client (default-deny)"
                ))),
                m if m.starts_with("terminal/") => Err(AcpError::Protocol(format!(
                    "terminal capability not advertised: {m}"
                ))),
                other => Err(AcpError::Protocol(format!(
                    "unhandled client-side method: {other}"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_denies_permission() {
        let h = DefaultHandlers::default();
        let r = h
            .handle("session/request_permission", json!({}))
            .await
            .expect("ok");
        assert_eq!(r["outcome"]["outcome"], "cancelled");
    }

    #[tokio::test]
    async fn allow_selects_first_option() {
        let h = DefaultHandlers::new().with_permission(PermissionDecision::Allow);
        let r = h
            .handle(
                "session/request_permission",
                json!({"options":[{"optionId":"opt-yes"}]}),
            )
            .await
            .expect("ok");
        assert_eq!(r["outcome"]["outcome"], "selected");
        assert_eq!(r["outcome"]["optionId"], "opt-yes");
    }

    #[tokio::test]
    async fn unadvertised_fs_is_rejected() {
        let h = DefaultHandlers::default();
        let err = h
            .handle("fs/read_text_file", json!({"path":"/etc/passwd"}))
            .await
            .unwrap_err();
        matches!(err, AcpError::Protocol(_));
        assert!(!h.advertised_fs_read());
    }
}
