//! Discord Gateway 어댑터 — 봇 토큰으로 WebSocket 양방향 연결.
//!
//! webhook 은 송신 전용. 다중 에이전트가 같은 디스코드 채널에서 대화·결정·합의
//! 하려면 메시지를 받아야 하고, 받으려면 봇 + Gateway WebSocket 이 필수다.
//!
//! 책임 분리: 송신 (`DiscordWebhookAdapter`) 은 별도 모듈. 이 모듈은 수신 + 봇
//! identity 만 담당.
//!
//! 사용 (요약):
//! ```ignore
//! use futures_util::StreamExt;
//! use openxgram_adapter::discord_gateway::DiscordGatewayClient;
//!
//! let client = DiscordGatewayClient::new(token);
//! let mut stream = Box::pin(client.connect().await?);
//! while let Some(msg) = stream.next().await {
//!     println!("[#{}] {}: {}", msg.channel_id, msg.author_name, msg.content);
//! }
//! ```

use std::sync::Once;

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use futures_util::stream::{self, Stream, StreamExt};
use thiserror::Error;
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_model::gateway::payload::incoming::MessageCreate;

/// KST UTC offset (+09:00).
const KST_OFFSET_SECS: i32 = 9 * 3600;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("gateway 연결 실패: {0}")]
    Connect(String),
    #[error("rustls provider 설치 실패: {0}")]
    Crypto(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;

/// Discord 첨부 파일 1건 메타.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordAttachment {
    pub url: String,
    pub filename: String,
    pub size: u64,
    pub content_type: Option<String>,
}

/// Discord 채널에서 받은 메시지 1건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordIncomingMessage {
    pub message_id: u64,
    pub channel_id: u64,
    pub guild_id: Option<u64>,
    pub author_id: u64,
    pub author_name: String,
    pub content: String,
    pub timestamp_kst: DateTime<FixedOffset>,
    /// rc.91 — 첨부 파일 (옵션, content 비어있어도 attachments 만 있으면 전달).
    pub attachments: Vec<DiscordAttachment>,
}

impl DiscordIncomingMessage {
    pub fn from_event(ev: &MessageCreate) -> Option<Self> {
        if ev.author.bot {
            return None;
        }
        let attachments: Vec<DiscordAttachment> = ev.attachments.iter().map(|a| DiscordAttachment {
            url: a.url.clone(),
            filename: a.filename.clone(),
            size: a.size as u64,
            content_type: a.content_type.clone(),
        }).collect();
        // 빈 content + 첨부 없음 → skip. 첨부만 있어도 통과 (rc.91).
        if ev.content.is_empty() && attachments.is_empty() {
            return None;
        }
        let utc_micros = ev.timestamp.as_micros();
        let utc: DateTime<Utc> = Utc
            .timestamp_micros(utc_micros)
            .single()
            .unwrap_or_else(Utc::now);
        let offset = FixedOffset::east_opt(KST_OFFSET_SECS).expect("KST offset valid");
        Some(Self {
            message_id: ev.id.get(),
            channel_id: ev.channel_id.get(),
            guild_id: ev.guild_id.map(|g| g.get()),
            author_id: ev.author.id.get(),
            author_name: ev.author.name.clone(),
            content: ev.content.clone(),
            timestamp_kst: utc.with_timezone(&offset),
            attachments,
        })
    }
}

/// Discord Gateway 봇 클라이언트.
pub struct DiscordGatewayClient {
    token: String,
    intents: Intents,
}

impl DiscordGatewayClient {
    /// 기본 intents 로 클라이언트 생성.
    ///
    /// `MESSAGE_CONTENT` 는 Discord 가 강제로 별도 활성을 요구하는 privileged
    /// intent. 봇 설정 페이지에서 사전 활성 필요.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            intents: Intents::GUILD_MESSAGES | Intents::DIRECT_MESSAGES | Intents::MESSAGE_CONTENT,
        }
    }

    /// intents 커스터마이즈 (테스트·고급 사용자).
    pub fn with_intents(mut self, intents: Intents) -> Self {
        self.intents = intents;
        self
    }

    /// Gateway 연결 + MESSAGE_CREATE 이벤트 stream 반환.
    pub async fn connect(&self) -> Result<impl Stream<Item = DiscordIncomingMessage> + Send> {
        install_crypto_provider()?;
        let shard = Shard::new(ShardId::ONE, self.token.clone(), self.intents);
        Ok(message_stream(shard, None))
    }

    /// 특정 channel_id 만 필터링하는 stream.
    pub async fn listen_channel(
        &self,
        channel_id: u64,
    ) -> Result<impl Stream<Item = DiscordIncomingMessage> + Send> {
        install_crypto_provider()?;
        let shard = Shard::new(ShardId::ONE, self.token.clone(), self.intents);
        Ok(message_stream(shard, Some(channel_id)))
    }
}

/// rustls ring provider 는 process-wide 1회만 설치. 이미 설치돼 있으면 무시.
fn install_crypto_provider() -> Result<()> {
    static INSTALL: Once = Once::new();
    let mut err: Option<String> = None;
    INSTALL.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_some() {
            return;
        }
        if let Err(e) = rustls::crypto::ring::default_provider().install_default() {
            err = Some(format!("{e:?}"));
        }
    });
    if let Some(e) = err {
        return Err(GatewayError::Crypto(e));
    }
    Ok(())
}

/// shard.next_event() loop → DiscordIncomingMessage stream.
///
/// 치명 오류 (invalid token, intent 미허용) 는 stream 종료, 그 외는 warn 로 skip.
fn message_stream(
    shard: Shard,
    filter_channel: Option<u64>,
) -> impl Stream<Item = DiscordIncomingMessage> + Send {
    let event_flags = EventTypeFlags::MESSAGE_CREATE;
    stream::unfold(
        (shard, filter_channel),
        move |(mut shard, filter)| async move {
            loop {
                let item = shard.next_event(event_flags).await?;
                let event = match item {
                    Ok(ev) => ev,
                    Err(source) => {
                        tracing::warn!(?source, "discord gateway event error");
                        continue;
                    }
                };
                tracing::info!("[DEBUG] discord event: {:?}", event.kind());
                if let Event::MessageCreate(msg_create) = event {
                    tracing::info!(
                        "[DEBUG] MessageCreate ch={} author_bot={} content_len={} content_preview={:?}",
                        msg_create.channel_id, msg_create.author.bot, msg_create.content.len(),
                        msg_create.content.chars().take(40).collect::<String>()
                    );
                    let Some(msg) = DiscordIncomingMessage::from_event(&msg_create) else {
                        continue;
                    };
                    if let Some(want) = filter {
                        if msg.channel_id != want {
                            continue;
                        }
                    }
                    return Some((msg, (shard, filter)));
                }
            }
        },
    )
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intents_include_message_content() {
        let client = DiscordGatewayClient::new("dummy");
        assert!(client.intents.contains(Intents::MESSAGE_CONTENT));
        assert!(client.intents.contains(Intents::GUILD_MESSAGES));
        assert!(client.intents.contains(Intents::DIRECT_MESSAGES));
    }

    #[test]
    fn install_crypto_provider_idempotent() {
        // 중복 호출이 panic 하지 않아야 한다 (Once 가드).
        install_crypto_provider().unwrap();
        install_crypto_provider().unwrap();
    }
}
