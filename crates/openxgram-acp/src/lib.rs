//! openxgram-acp — ACP (Agent Client Protocol, Zed) client adapter.
//!
//! 정본 설계: `docs/research/acp-core-integration.md` (§1 spec, §2 crate design).
//!
//! OpenXgram acts as an **ACP Client**: it spawns an ACP agent as a child
//! subprocess and drives it over newline-delimited JSON-RPC 2.0 on the child's
//! stdin/stdout. The agent's stderr is for logs only and is never parsed as
//! protocol (§6 stdio pollution). ACP is full-duplex — the agent calls back
//! into the client (`fs/*`, `session/request_permission`, `terminal/*`), so
//! this crate runs a full JSON-RPC *peer*, not merely a request sender.
//!
//! 모듈:
//!   - [`transport`] — LDJSON JSON-RPC framing over a child's stdin/stdout (tokio).
//!   - [`rpc`]       — JSON-RPC 2.0 peer: id alloc, pending-request map, inbound dispatch.
//!   - [`types`]     — serde wire types (initialize / session / ContentBlock / SessionUpdate).
//!   - [`handlers`]  — [`handlers::ClientSideHandlers`] trait (default-deny / minimal).
//!   - [`client`]    — [`client::AcpClient`]: owns the spawned process + peer.
//!   - [`session`]   — [`session::AcpSession`]: per-session state.
//!   - [`registry`]  — known ACP agent adapters (name → command/args/env).
//!   - [`mcp`]       — [`mcp::AcpTools`]: acp_spawn / acp_prompt / acp_cancel / ...
//!
//! Phase B-1 scope: the crate itself, building standalone with a mock-agent
//! integration test. Daemon + GUI wiring is B-2 / B-3.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod client;
pub mod handlers;
pub mod mcp;
pub mod registry;
pub mod rpc;
pub mod session;
pub mod transport;
pub mod types;

pub use client::AcpClient;
pub use handlers::{ClientSideHandlers, DefaultHandlers, PermissionDecision};
pub use mcp::{AcpTools, SpawnOpts};
pub use registry::{AgentSpec, AgentSpecBuilder};
pub use rpc::{RpcPeer, RpcRequest, RpcResponse};
pub use session::AcpSession;
pub use types::{
    AgentCapabilities, AgentInfo, ClientCapabilities, ClientInfo, ContentBlock, FsCapabilities,
    InitializeRequest, InitializeResponse, PromptCapabilities, SessionNewRequest,
    SessionNewResponse, SessionNotification, SessionUpdate, StopReason, ToolCall, ToolCallStatus,
};

use thiserror::Error;

/// ACP protocol version OpenXgram speaks. A single MAJOR integer per spec (§1).
pub const PROTOCOL_VERSION: u32 = 1;

/// JSON-RPC version literal used on every frame.
pub const JSONRPC_VERSION: &str = "2.0";

/// ACP adapter 전반 에러.
///
/// 절대 규칙 1 (fallback 금지): silent fallback 없음. 모든 실패는 명시 enum variant.
#[derive(Debug, Error)]
pub enum AcpError {
    /// Failed to spawn the agent subprocess.
    #[error("failed to spawn agent process: {0}")]
    Spawn(std::io::Error),

    /// `initialize` negotiated an incompatible protocol version.
    #[error("initialize failed: agent protocolVersion={got}, want={want}")]
    InitFailed {
        /// Version the agent reported.
        got: u32,
        /// Version we require.
        want: u32,
    },

    /// The agent requires authentication (non-empty `authMethods`) which is
    /// unsupported in this phase.
    #[error("agent requires authentication; authMethods not supported in this phase")]
    AuthRequired,

    /// JSON-RPC response carried an `error` object.
    #[error("acp rpc error code={code}: {message}")]
    RpcError {
        /// JSON-RPC error code.
        code: i64,
        /// Error message.
        message: String,
    },

    /// A JSON-RPC response had neither `result` nor `error`, or was otherwise malformed.
    #[error("invalid jsonrpc response: {0}")]
    InvalidRpcResponse(String),

    /// Generic protocol violation (unexpected method, missing field, ...).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The referenced session is closed or unknown.
    #[error("session closed or unknown")]
    SessionClosed,

    /// The agent process exited unexpectedly.
    #[error("agent process exited (code={code:?})")]
    AgentExited {
        /// Exit code if available.
        code: Option<i32>,
    },

    /// An operation exceeded its deadline.
    #[error("operation timed out")]
    Timeout,

    /// An unknown ACP agent name was requested from the registry.
    #[error("unknown acp agent: {0}")]
    UnknownAgent(String),

    /// A request/response/notification could not be (de)serialized.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Underlying I/O failure on a pipe.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// crate result alias.
pub type Result<T> = std::result::Result<T, AcpError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn jsonrpc_version_is_2_0() {
        assert_eq!(JSONRPC_VERSION, "2.0");
    }
}
