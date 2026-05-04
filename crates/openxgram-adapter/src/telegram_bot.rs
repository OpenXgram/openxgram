//! Telegram bot 어댑터 — sendMessage (송신) + getUpdates long-polling (수신).
//!
//! 양방향 통합. send_text 는 `Adapter` trait 구현, poll_updates 는 별도 메서드
//! (모든 어댑터가 받기를 지원하지는 않으므로 trait 강제 안 함).

use serde::{Deserialize, Serialize};

use crate::{check_status, Adapter, AdapterError, Result};

/// long polling timeout (초). Telegram 공식 권장 25 ~ 50.
const DEFAULT_POLL_TIMEOUT_SECS: u32 = 25;

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

    fn endpoint(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            self.api_base.trim_end_matches('/'),
            self.bot_token,
            method
        )
    }

    /// long-polling 으로 신규 update 조회.
    ///
    /// `offset` — Telegram update_id 기준. 받은 마지막 update_id + 1 을 다음 호출에 넘긴다.
    /// 서버는 새 메시지가 올 때까지 최대 `timeout_secs` 초 동안 응답을 보류한다.
    pub async fn poll_updates(
        &self,
        offset: i64,
        timeout_secs: Option<u32>,
    ) -> Result<Vec<TelegramUpdate>> {
        let timeout = timeout_secs.unwrap_or(DEFAULT_POLL_TIMEOUT_SECS);
        // 단순 querystring (reqwest query feature 없이도 동작).
        let url = format!(
            "{}?offset={}&timeout={}",
            self.endpoint("getUpdates"),
            offset,
            timeout,
        );
        // reqwest 의 read timeout 은 polling timeout 보다 약간 길게.
        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs((timeout + 5) as u64))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AdapterError::ServerError {
                status: status.as_u16(),
                body,
            });
        }
        let parsed: GetUpdatesResponse = resp.json().await?;
        if !parsed.ok {
            return Err(AdapterError::ServerError {
                status: 200,
                body: format!(
                    "telegram getUpdates ok=false: {}",
                    parsed
                        .description
                        .unwrap_or_else(|| "(no description)".into())
                ),
            });
        }
        Ok(parsed
            .result
            .unwrap_or_default()
            .into_iter()
            .filter_map(TelegramUpdate::from_raw)
            .collect())
    }
}

#[derive(Serialize)]
struct TelegramSendPayload<'a> {
    chat_id: &'a str,
    text: &'a str,
}

impl Adapter for TelegramBotAdapter {
    async fn send_text(&self, text: &str) -> Result<()> {
        let url = self.endpoint("sendMessage");
        let resp = self
            .client
            .post(&url)
            .json(&TelegramSendPayload {
                chat_id: &self.chat_id,
                text,
            })
            .send()
            .await?;
        check_status(resp).await
    }
}

// ── 수신 모델 ────────────────────────────────────────────────────────────

/// 사용자가 봇에 보낸 메시지 1건. 텍스트 메시지만 추출 (사진·스티커·기타 무시).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub chat_id: i64,
    pub text: String,
    pub sender_username: Option<String>,
}

impl TelegramUpdate {
    fn from_raw(raw: RawUpdate) -> Option<Self> {
        let msg = raw.message?;
        let text = msg.text?;
        Some(Self {
            update_id: raw.update_id,
            chat_id: msg.chat.id,
            text,
            sender_username: msg.from.and_then(|u| u.username),
        })
    }
}

#[derive(Deserialize)]
struct GetUpdatesResponse {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    result: Option<Vec<RawUpdate>>,
}

#[derive(Deserialize)]
struct RawUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    chat: RawChat,
    #[serde(default)]
    from: Option<RawUser>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct RawChat {
    id: i64,
}

#[derive(Deserialize)]
struct RawUser {
    #[serde(default)]
    username: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_update() {
        let json = r#"{
            "ok": true,
            "result": [
                {"update_id": 100, "message": {
                    "chat": {"id": 7777},
                    "from": {"username": "alice"},
                    "text": "hello"
                }},
                {"update_id": 101, "message": {
                    "chat": {"id": 7777},
                    "text": "no sender"
                }},
                {"update_id": 102, "message": {
                    "chat": {"id": 7777},
                    "from": {"username": "bob"}
                }}
            ]
        }"#;
        let parsed: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let updates: Vec<_> = parsed
            .result
            .unwrap()
            .into_iter()
            .filter_map(TelegramUpdate::from_raw)
            .collect();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].update_id, 100);
        assert_eq!(updates[0].chat_id, 7777);
        assert_eq!(updates[0].text, "hello");
        assert_eq!(updates[0].sender_username.as_deref(), Some("alice"));
        assert_eq!(updates[1].sender_username, None);
    }
}
