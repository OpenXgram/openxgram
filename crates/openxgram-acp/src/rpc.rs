//! JSON-RPC 2.0 peer for ACP (full-duplex).
//!
//! ACP is bidirectional (§1.1): we send requests *and* the agent sends requests
//! back into us mid-turn. This peer therefore both:
//!   - issues requests (allocating ids, parking a `oneshot` per pending id),
//!   - serves inbound requests (routing to [`ClientSideHandlers`]),
//!   - fans out notifications (`session/update`) to a listener channel.
//!
//! Full-duplex safety (§6): a single reader task only *dispatches* — it never
//! awaits another agent response inside a handler. Inbound requests are handled
//! on spawned tasks so the reader keeps draining stdout while a handler runs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::handlers::ClientSideHandlers;
use crate::{AcpError, Result, JSONRPC_VERSION};

/// An outbound JSON-RPC request frame.
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: &'static str,
    /// Request id.
    pub id: i64,
    /// Method name.
    pub method: String,
    /// Parameters object.
    pub params: Value,
}

/// A response frame we send back to the agent for an inbound request.
#[derive(Debug, Clone, Serialize)]
pub struct RpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: &'static str,
    /// Mirrors the inbound request id.
    pub id: Value,
    /// Result payload (present on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error payload (present on failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>;

/// A live JSON-RPC peer bound to one agent process.
///
/// Cloneable handle (everything shared is `Arc`), so the client and each
/// session can hold one.
#[derive(Clone)]
pub struct RpcPeer {
    next_id: Arc<AtomicI64>,
    pending: PendingMap,
    writer: mpsc::UnboundedSender<Value>,
    handlers: Arc<dyn ClientSideHandlers>,
    /// Per-session notification listeners keyed by `sessionId`.
    listeners: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>>,
}

impl RpcPeer {
    /// Build a peer over an already-spawned writer channel + handler set, and
    /// start the inbound dispatch loop draining `reader`.
    pub fn start(
        writer: mpsc::UnboundedSender<Value>,
        reader: mpsc::UnboundedReceiver<Result<Value>>,
        handlers: Arc<dyn ClientSideHandlers>,
    ) -> Self {
        let peer = RpcPeer {
            next_id: Arc::new(AtomicI64::new(1)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            writer,
            handlers,
            listeners: Arc::new(Mutex::new(HashMap::new())),
        };
        peer.spawn_dispatch(reader);
        peer
    }

    /// Register an `mpsc` sender to receive `session/update` notifications for a
    /// given session id.
    pub async fn register_listener(&self, session_id: String) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.listeners.lock().await.insert(session_id, tx);
        rx
    }

    /// Remove a session's notification listener.
    pub async fn drop_listener(&self, session_id: &str) {
        self.listeners.lock().await.remove(session_id);
    }

    /// Issue a request and await its response. Resolves when the agent replies.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let frame = RpcRequest {
            jsonrpc: JSONRPC_VERSION,
            id,
            method: method.to_string(),
            params,
        };
        let value = serde_json::to_value(&frame)?;
        if self.writer.send(value).is_err() {
            self.pending.lock().await.remove(&id);
            return Err(AcpError::AgentExited { code: None });
        }

        match rx.await {
            Ok(result) => result,
            // sender dropped without sending → reader loop ended (agent gone).
            Err(_) => Err(AcpError::AgentExited { code: None }),
        }
    }

    /// Send a notification (no id, no response expected), e.g. `session/cancel`.
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        let frame = json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
            "params": params,
        });
        self.writer
            .send(frame)
            .map_err(|_| AcpError::AgentExited { code: None })
    }

    fn spawn_dispatch(&self, mut reader: mpsc::UnboundedReceiver<Result<Value>>) {
        let pending = self.pending.clone();
        let listeners = self.listeners.clone();
        let handlers = self.handlers.clone();
        let writer = self.writer.clone();

        tokio::spawn(async move {
            while let Some(item) = reader.recv().await {
                let frame = match item {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::debug!(target: "acp.rpc", "inbound frame error, ending dispatch: {e}");
                        break;
                    }
                };
                Self::route(&frame, &pending, &listeners, &handlers, &writer).await;
            }
            // Reader ended: fail every pending request loudly (절대 규칙 1).
            let mut map = pending.lock().await;
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(AcpError::AgentExited { code: None }));
            }
        });
    }

    async fn route(
        frame: &Value,
        pending: &PendingMap,
        listeners: &Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>>,
        handlers: &Arc<dyn ClientSideHandlers>,
        writer: &mpsc::UnboundedSender<Value>,
    ) {
        let has_id = frame.get("id").map(|v| !v.is_null()).unwrap_or(false);
        let has_method = frame.get("method").and_then(|v| v.as_str()).is_some();

        if has_method && has_id {
            // Inbound request (agent → client). Handle on a spawned task.
            Self::dispatch_request(frame.clone(), handlers.clone(), writer.clone());
        } else if has_method {
            // Notification (agent → client), e.g. session/update.
            Self::dispatch_notification(frame, listeners).await;
        } else if has_id {
            // Response to one of our requests.
            Self::dispatch_response(frame, pending).await;
        } else {
            tracing::warn!(target: "acp.rpc", "unroutable frame (no id, no method): {frame}");
        }
    }

    async fn dispatch_response(frame: &Value, pending: &PendingMap) {
        let id = match frame.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => {
                tracing::warn!(target: "acp.rpc", "response with non-integer id ignored: {frame}");
                return;
            }
        };
        let tx = pending.lock().await.remove(&id);
        let Some(tx) = tx else {
            tracing::warn!(target: "acp.rpc", "response for unknown id {id}");
            return;
        };
        let outcome = if let Some(err) = frame.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("<no message>")
                .to_string();
            Err(AcpError::RpcError { code, message })
        } else if let Some(result) = frame.get("result") {
            Ok(result.clone())
        } else {
            Err(AcpError::InvalidRpcResponse(
                "response missing both result and error".into(),
            ))
        };
        let _ = tx.send(outcome);
    }

    async fn dispatch_notification(
        frame: &Value,
        listeners: &Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>>,
    ) {
        let method = frame.get("method").and_then(|m| m.as_str()).unwrap_or("");
        if method != "session/update" {
            tracing::debug!(target: "acp.rpc", "unhandled notification: {method}");
            return;
        }
        let params = frame.get("params").cloned().unwrap_or(Value::Null);
        let session_id = params
            .get("sessionId")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();
        let map = listeners.lock().await;
        if let Some(tx) = map.get(&session_id) {
            let _ = tx.send(params);
        } else {
            tracing::debug!(target: "acp.rpc", "session/update for unregistered session {session_id}");
        }
    }

    fn dispatch_request(
        frame: Value,
        handlers: Arc<dyn ClientSideHandlers>,
        writer: mpsc::UnboundedSender<Value>,
    ) {
        tokio::spawn(async move {
            let id = frame.get("id").cloned().unwrap_or(Value::Null);
            let method = frame
                .get("method")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let params = frame.get("params").cloned().unwrap_or(Value::Null);

            let response = match handlers.handle(&method, params).await {
                Ok(result) => RpcResponse {
                    jsonrpc: JSONRPC_VERSION,
                    id,
                    result: Some(result),
                    error: None,
                },
                Err(e) => RpcResponse {
                    jsonrpc: JSONRPC_VERSION,
                    id,
                    result: None,
                    error: Some(json!({ "code": -32603, "message": e.to_string() })),
                },
            };
            match serde_json::to_value(&response) {
                Ok(v) => {
                    let _ = writer.send(v);
                }
                Err(e) => {
                    tracing::error!(target: "acp.rpc", "failed to serialize handler response: {e}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::DefaultHandlers;

    fn test_peer() -> (
        RpcPeer,
        mpsc::UnboundedReceiver<Value>,
        mpsc::UnboundedSender<Result<Value>>,
    ) {
        let (writer_tx, writer_rx) = mpsc::unbounded_channel::<Value>();
        let (reader_tx, reader_rx) = mpsc::unbounded_channel::<Result<Value>>();
        let peer = RpcPeer::start(writer_tx, reader_rx, Arc::new(DefaultHandlers::default()));
        (peer, writer_rx, reader_tx)
    }

    #[tokio::test]
    async fn request_resolves_on_matching_response() {
        let (peer, mut writer_rx, reader_tx) = test_peer();

        let p = peer.clone();
        let join = tokio::spawn(async move { p.request("initialize", json!({})).await });

        // The outbound frame should have been written with id=1.
        let out = writer_rx.recv().await.expect("outbound");
        assert_eq!(out["id"], 1);
        assert_eq!(out["method"], "initialize");

        // Feed a matching response.
        reader_tx
            .send(Ok(json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}})))
            .expect("inject");

        let res = join.await.expect("join").expect("ok");
        assert_eq!(res["ok"], true);
    }

    #[tokio::test]
    async fn rpc_error_response_maps_to_error() {
        let (peer, mut writer_rx, reader_tx) = test_peer();
        let p = peer.clone();
        let join = tokio::spawn(async move { p.request("session/new", json!({})).await });
        let _ = writer_rx.recv().await.expect("outbound");
        reader_tx
            .send(Ok(
                json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"nope"}}),
            ))
            .expect("inject");
        let err = join.await.expect("join").unwrap_err();
        matches!(err, AcpError::RpcError { code: -32601, .. });
    }

    #[tokio::test]
    async fn notification_routes_to_session_listener() {
        let (peer, _writer_rx, reader_tx) = test_peer();
        let mut updates = peer.register_listener("sess-1".into()).await;
        reader_tx
            .send(Ok(json!({
                "jsonrpc":"2.0",
                "method":"session/update",
                "params":{"sessionId":"sess-1","sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}
            })))
            .expect("inject");
        let got = updates.recv().await.expect("update");
        assert_eq!(got["sessionId"], "sess-1");
    }
}
