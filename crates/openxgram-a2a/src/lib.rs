//! openxgram-a2a — A2A (Agent-to-Agent) 호환 어댑터.
//!
//! 정본: docs/PRD-OpenXgram.md §4.5
//!
//! 외부 A2A 표준 에이전트 (https://a2a-protocol.org/) 와의 상호운용 layer.
//!
//! - AgentCard JSON 가져오기 (`/.well-known/agent-card.json` HTTP GET)
//! - JSON-RPC 2.0 endpoint 호출 (`tasks/send`, `tasks/get`, `tasks/cancel`)
//! - 인증: Bearer / OAuth2 / none
//! - 응답 상태 관리: TaskState (submitted/working/input-required/completed/canceled/failed)
//!
//! 모듈:
//!   - agent_card : AgentCard 도메인 (serde)
//!   - client     : A2aClient (reqwest 래퍼, JSON-RPC 호출)
//!   - task       : Task / TaskState / Message
//!   - mcp        : 4개 MCP 도구 핸들러 (A2aTools)
//!
//! v0.7 범위는 **client only**. server 노출 (`/v1/a2a/...`) 은 stretch.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod agent_card;
pub mod client;
pub mod mcp;
pub mod task;

pub use agent_card::{AgentCard, AgentCapabilities, AgentSkill, Authentication};
pub use client::A2aClient;
pub use mcp::A2aTools;
pub use task::{Message, Part, Task, TaskState};

use thiserror::Error;

/// A2A 어댑터 전반 에러.
///
/// 절대 규칙 1 (fallback 금지): silent fallback 없음. 모든 실패는 명시 enum variant.
#[derive(Debug, Error)]
pub enum A2aError {
    /// HTTP / 네트워크 실패.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// 잘못된 URL.
    #[error("invalid url: {0}")]
    InvalidUrl(String),

    /// AgentCard 가져오기 실패 (404 / 5xx).
    #[error("agent-card fetch failed for {url}: status={status}")]
    AgentCardFetch {
        /// 시도한 URL.
        url: String,
        /// HTTP status.
        status: u16,
    },

    /// JSON-RPC 응답이 error 객체를 담음.
    #[error("a2a rpc error code={code}: {message}")]
    RpcError {
        /// JSON-RPC error code.
        code: i64,
        /// 에러 메시지.
        message: String,
    },

    /// JSON-RPC 응답에 result 도 error 도 없음 (프로토콜 위반).
    #[error("invalid jsonrpc response: missing both result and error")]
    InvalidRpcResponse,

    /// 알 수 없는 / 지원 안 되는 task state 문자열.
    #[error("unknown task state: {0}")]
    UnknownTaskState(String),

    /// JSON 직렬화/역직렬화 실패.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// 기타 (메시지로 명시).
    #[error("{0}")]
    Other(String),
}

/// crate result alias.
pub type Result<T> = std::result::Result<T, A2aError>;

/// AgentCard well-known 경로 (A2A 표준).
pub const WELL_KNOWN_AGENT_CARD: &str = "/.well-known/agent-card.json";

/// 지원하는 JSON-RPC version.
pub const JSONRPC_VERSION: &str = "2.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_path_is_standard() {
        assert_eq!(WELL_KNOWN_AGENT_CARD, "/.well-known/agent-card.json");
    }

    #[test]
    fn jsonrpc_version_is_2_0() {
        assert_eq!(JSONRPC_VERSION, "2.0");
    }
}
