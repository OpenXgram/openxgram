//! openxgram-adapter — outbound + inbound 메시지 어댑터.
//!
//! - Discord webhook        : 송신 전용 (webhook 은 receive 미지원).
//! - Discord Gateway (봇)   : 수신 (WebSocket) — 다중 에이전트 채팅방 허브.
//! - Telegram bot           : 양방향 (sendMessage + getUpdates long-polling).
//!
//! Async fn in trait (Rust 1.75+ stable) 사용. dyn-compatibility 필요 시 호출자
//! 측에서 BoxFuture 래핑.

pub mod channel_mcp;
pub mod discord_gateway;
pub mod telegram_bot;

use serde::Serialize;
use thiserror::Error;

pub use channel_mcp::{AdapterInfo, ChannelMcpClient, ChannelSendResult};
pub use discord_gateway::{DiscordGatewayClient, DiscordIncomingMessage, GatewayError};
pub use telegram_bot::{TelegramBotAdapter, TelegramUpdate};

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("server error (status {status}): {body}")]
    ServerError { status: u16, body: String },
}

pub type Result<T> = std::result::Result<T, AdapterError>;

pub trait Adapter: Send + Sync {
    fn send_text(&self, text: &str) -> impl std::future::Future<Output = Result<()>> + Send;
}

// ── Discord webhook ──────────────────────────────────────────────────────

pub struct DiscordWebhookAdapter {
    pub webhook_url: String,
    client: reqwest::Client,
}

impl DiscordWebhookAdapter {
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self {
            webhook_url: webhook_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct DiscordPayload<'a> {
    content: &'a str,
}

impl Adapter for DiscordWebhookAdapter {
    async fn send_text(&self, text: &str) -> Result<()> {
        let resp = self
            .client
            .post(&self.webhook_url)
            .json(&DiscordPayload { content: text })
            .send()
            .await?;
        check_status(resp).await
    }
}

// ── 공용 ────────────────────────────────────────────────────────────────

pub(crate) async fn check_status(resp: reqwest::Response) -> Result<()> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AdapterError::ServerError {
            status: status.as_u16(),
            body,
        });
    }
    Ok(())
}
