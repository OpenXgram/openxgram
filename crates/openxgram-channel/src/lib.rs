//! openxgram-channel — Embedded Channel MCP server.
//!
//! OpenXgram 자체가 channel-mcp 의 정식 구현체. 외부 channel-mcp 운영 불필요.
//!
//! 구성:
//! - `AdapterRegistry`  : discord/telegram/slack/kakaotalk webhook 어댑터 통합.
//! - `PeerRegistry`     : alias↔role↔platform 매핑 (in-memory; SQLite peers 와 별개의 routing layer).
//! - `RouteEngine`      : send_to_platform / send_message 라우팅.
//! - `ChannelServer`    : axum HTTP 서버. POST /tools/{name} JSON 요청.
//!
//! Starian Channel MCP 호환 도구:
//! - send_to_platform(platform, channel_id, text, reply_to?)
//! - send_message(to, summary, type, details?)
//! - list_adapters()
//! - list_peers()
//! - set_status(status, summary?)
//! - track_task(task_id, title, status?)
//!
//! 외부 0.0.0.0 바인딩 절대 금지 — 기본 127.0.0.1:7250.

pub mod adapter;
pub mod peer;
pub mod route;
pub mod server;

pub use adapter::{
    AdapterDescriptor, AdapterEntry, AdapterKind, AdapterRegistry, ChannelAdapter,
    KakaoTalkPlaceholderAdapter, SlackWebhookAdapter,
};
pub use peer::{ChannelPeer, PeerRegistry};
pub use route::{RouteEngine, RouteResult};
pub use server::{serve, ChannelServer, ServerConfig, ServerHandle};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("adapter not found: {0}")]
    AdapterNotFound(String),

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("auth failed: {0}")]
    Auth(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("adapter error: {0}")]
    Adapter(#[from] openxgram_adapter::AdapterError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ChannelError>;
