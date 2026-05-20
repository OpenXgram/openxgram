//! # openxgram-anp
//!
//! ANP (Agent Network Protocol) 호환 어댑터.
//!
//! - 분산 에이전트 발견: did:wba (openxgram-did::wba) 기반
//! - 통신: HTTP + JSON + DID 서명 (ECDSA secp256k1)
//! - MCP 도구 4개: anp_resolve_did / anp_send_message / anp_verify_signature / anp_announce_self
//!
//! ## 참고
//! - ANP: <https://github.com/agent-network-protocol/AgentNetworkProtocol>
//! - W3C DID Core: <https://www.w3.org/TR/did-core/>
//!
//! ## 모듈 구성
//! - [`discovery`] — did:wba resolve + service endpoint 추출
//! - [`client`]    — ANP HTTP 클라이언트 (envelope 서명 헤더)
//! - [`message`]   — ANP envelope 직렬화·canonical hashing
//! - [`mcp`]       — 4개 MCP 도구 핸들러 (`AnpTools`)
//!
//! 어떤 모듈도 영구 데이터를 만들지 않는다 → migration 불필요.

use thiserror::Error;

pub mod client;
pub mod discovery;
pub mod mcp;
pub mod message;

pub use client::AnpClient;
pub use discovery::{discover_endpoint, resolve_did_document, DiscoveryResult};
pub use mcp::{AnpToolError, AnpTools};
pub use message::{AnpEnvelope, AnpHeader, EnvelopeError};

#[derive(Debug, Error)]
pub enum AnpError {
    #[error("did resolution failed: {0}")]
    Resolution(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("did error: {0}")]
    Did(String),

    #[error("envelope error: {0}")]
    Envelope(#[from] EnvelopeError),

    #[error("signature verification failed")]
    Signature,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid did:wba document: {0}")]
    InvalidDocument(String),

    #[error("no service endpoint found for type {0}")]
    NoServiceEndpoint(String),
}

impl From<openxgram_did::DidError> for AnpError {
    fn from(e: openxgram_did::DidError) -> Self {
        Self::Did(e.to_string())
    }
}

impl From<reqwest::Error> for AnpError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e.to_string())
    }
}

/// crate 결과 타입.
pub type Result<T> = std::result::Result<T, AnpError>;

/// ANP "AnpAgent" service type — DID document service.type 매칭에 사용.
pub const ANP_AGENT_SERVICE_TYPE: &str = "AnpAgent";

/// ANP 통신 envelope content-type.
pub const ANP_CONTENT_TYPE: &str = "application/anp-envelope+json";
