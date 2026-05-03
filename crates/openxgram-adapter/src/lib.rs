//! openxgram-adapter — outbound 메시지 어댑터 (Discord webhook, Telegram bot).
//!
//! Phase 1 first PR: 텍스트 send_text 만. 첨부·서명 envelope·rate-limit 은 후속.
//!
//! Async fn in trait (Rust 1.75+ stable) 사용. dyn-compatibility 필요 시 호출자
//! 측에서 BoxFuture 래핑.

use serde::Serialize;
use thiserror::Error;

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

// ── Telegram bot ─────────────────────────────────────────────────────────

pub struct TelegramBotAdapter {
    pub api_base: String,
    pub bot_token: String,
    pub chat_id: String,
    client: reqwest::Client,
}

impl TelegramBotAdapter {
    pub fn new(bot_token: impl Into<String>, chat_id: impl Into<String>) -> Self {
        Self {
            api_base: "https://api.telegram.org".into(),
            bot_token: bot_token.into(),
            chat_id: chat_id.into(),
            client: reqwest::Client::new(),
        }
    }

    /// 테스트·self-host 환경에서 API 베이스를 교체.
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }
}

#[derive(Serialize)]
struct TelegramPayload<'a> {
    chat_id: &'a str,
    text: &'a str,
}

impl Adapter for TelegramBotAdapter {
    async fn send_text(&self, text: &str) -> Result<()> {
        let url = format!(
            "{}/bot{}/sendMessage",
            self.api_base.trim_end_matches('/'),
            self.bot_token
        );
        let resp = self
            .client
            .post(&url)
            .json(&TelegramPayload {
                chat_id: &self.chat_id,
                text,
            })
            .send()
            .await?;
        check_status(resp).await
    }
}

// ── 공용 ────────────────────────────────────────────────────────────────

async fn check_status(resp: reqwest::Response) -> Result<()> {
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
