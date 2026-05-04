//! Channel adapter registry.
//!
//! 통합되는 어댑터:
//! - Discord webhook  (openxgram-adapter::DiscordWebhookAdapter 재사용)
//! - Telegram bot     (openxgram-adapter::TelegramBotAdapter 재사용)
//! - Slack webhook    (신규 — webhook payload 만 다름)
//! - KakaoTalk        (placeholder — 실 API 통합은 후속)
//!
//! 모든 어댑터는 사용자 자기 webhook/bot 토큰만 사용. 외부로 데이터 노출 0.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use openxgram_adapter::{Adapter, DiscordWebhookAdapter, TelegramBotAdapter};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{ChannelError, Result};

/// 어댑터 종류 — Starian channel-mcp `list_adapters()` 응답과 일치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AdapterKind {
    Discord,
    Telegram,
    Slack,
    Kakaotalk,
    Webhook,
}

impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::Slack => "slack",
            Self::Kakaotalk => "kakaotalk",
            Self::Webhook => "webhook",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "discord" => Self::Discord,
            "telegram" => Self::Telegram,
            "slack" => Self::Slack,
            "kakaotalk" | "kakao" => Self::Kakaotalk,
            "webhook" => Self::Webhook,
            other => return Err(ChannelError::Invalid(format!("unknown adapter: {other}"))),
        })
    }
}

/// 어댑터 메타데이터 + 실제 sender.
#[derive(Clone)]
pub struct AdapterEntry {
    pub kind: AdapterKind,
    pub label: String,
    inner: Arc<dyn ChannelAdapter>,
}

impl std::fmt::Debug for AdapterEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterEntry")
            .field("kind", &self.kind)
            .field("label", &self.label)
            .finish()
    }
}

impl AdapterEntry {
    pub fn new(
        kind: AdapterKind,
        label: impl Into<String>,
        inner: Arc<dyn ChannelAdapter>,
    ) -> Self {
        Self {
            kind,
            label: label.into(),
            inner,
        }
    }

    pub async fn send(&self, text: &str) -> Result<()> {
        self.inner.send_text(text).await
    }
}

/// dyn-compatible adapter trait — Starian channel-mcp 의 send_to_platform 단일 진입점.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    async fn send_text(&self, text: &str) -> Result<()>;
}

#[async_trait]
impl ChannelAdapter for DiscordWebhookAdapter {
    async fn send_text(&self, text: &str) -> Result<()> {
        Adapter::send_text(self, text).await?;
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramBotAdapter {
    async fn send_text(&self, text: &str) -> Result<()> {
        Adapter::send_text(self, text).await?;
        Ok(())
    }
}

// ── Slack webhook ────────────────────────────────────────────────────────

/// Slack incoming webhook 어댑터. 사용자 webhook URL 만 사용. bot 통합은 후속.
pub struct SlackWebhookAdapter {
    pub webhook_url: String,
    client: reqwest::Client,
}

impl SlackWebhookAdapter {
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self {
            webhook_url: webhook_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct SlackPayload<'a> {
    text: &'a str,
}

#[async_trait]
impl ChannelAdapter for SlackWebhookAdapter {
    async fn send_text(&self, text: &str) -> Result<()> {
        let resp = self
            .client
            .post(&self.webhook_url)
            .json(&SlackPayload { text })
            .send()
            .await
            .map_err(openxgram_adapter::AdapterError::Http)?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Adapter(
                openxgram_adapter::AdapterError::ServerError {
                    status: status.as_u16(),
                    body,
                },
            ));
        }
        Ok(())
    }
}

// ── KakaoTalk placeholder ────────────────────────────────────────────────

/// KakaoTalk 어댑터 placeholder — Kakao Open Builder / Bizmessage API 통합은 후속.
/// 현재는 호출 시 명시적으로 unimplemented 반환 (조용히 무시 금지 — fallback 절대 규칙).
pub struct KakaoTalkPlaceholderAdapter {
    pub channel_id: String,
}

impl KakaoTalkPlaceholderAdapter {
    pub fn new(channel_id: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
        }
    }
}

#[async_trait]
impl ChannelAdapter for KakaoTalkPlaceholderAdapter {
    async fn send_text(&self, _text: &str) -> Result<()> {
        Err(ChannelError::Invalid(
            "kakaotalk adapter is a placeholder — real API integration pending".into(),
        ))
    }
}

// ── Registry ─────────────────────────────────────────────────────────────

/// 다수 adapter 를 (kind, label) 키로 등록·조회.
#[derive(Default, Clone)]
pub struct AdapterRegistry {
    inner: Arc<RwLock<HashMap<String, AdapterEntry>>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key(kind: AdapterKind, label: &str) -> String {
        format!("{}:{}", kind.as_str(), label)
    }

    pub async fn register(&self, entry: AdapterEntry) {
        let key = Self::key(entry.kind, &entry.label);
        self.inner.write().await.insert(key, entry);
    }

    pub async fn get(&self, kind: AdapterKind, label: &str) -> Option<AdapterEntry> {
        self.inner
            .read()
            .await
            .get(&Self::key(kind, label))
            .cloned()
    }

    /// 같은 kind 중 첫 번째 — channel_id 가 명시되지 않으면 fallback 으로 사용.
    pub async fn first_of(&self, kind: AdapterKind) -> Option<AdapterEntry> {
        self.inner
            .read()
            .await
            .values()
            .find(|e| e.kind == kind)
            .cloned()
    }

    pub async fn list(&self) -> Vec<AdapterDescriptor> {
        self.inner
            .read()
            .await
            .values()
            .map(|e| AdapterDescriptor {
                kind: e.kind,
                label: e.label.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterDescriptor {
    pub kind: AdapterKind,
    pub label: String,
}
