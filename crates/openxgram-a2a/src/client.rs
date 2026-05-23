//! A2aClient — 외부 A2A 에이전트 HTTP/JSON-RPC 클라이언트.
//!
//! 절대 규칙 1 (fallback 금지): 모든 HTTP/RPC 실패는 명시 enum.
//!
//! 사용:
//! ```no_run
//! # use openxgram_a2a::A2aClient;
//! # async fn demo() -> anyhow::Result<()> {
//! let client = A2aClient::new("https://translate.example.com")?;
//! let card = client.discover().await?;
//! println!("{}", card.name);
//! # Ok(()) }
//! ```

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;
use uuid::Uuid;

use crate::agent_card::AgentCard;
use crate::task::Task;
use crate::{A2aError, Result, JSONRPC_VERSION, WELL_KNOWN_AGENT_CARD};

/// 기본 요청 타임아웃 (초).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// A2A 에이전트 클라이언트.
///
/// `base_url` 은 에이전트의 origin (예: `https://translate.example.com`).
/// AgentCard 의 `url` 필드와 같거나 그 origin.
#[derive(Debug, Clone)]
pub struct A2aClient {
    base_url: Url,
    http: reqwest::Client,
    auth_header: Option<HeaderValue>,
}

impl A2aClient {
    /// 기본 클라이언트 생성 (인증 없음).
    pub fn new(base_url: impl AsRef<str>) -> Result<Self> {
        let url = Url::parse(base_url.as_ref())
            .map_err(|e| A2aError::InvalidUrl(format!("{}: {e}", base_url.as_ref())))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .user_agent(concat!("openxgram-a2a/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            base_url: url,
            http,
            auth_header: None,
        })
    }

    /// Bearer 토큰 부착.
    pub fn with_bearer(mut self, token: impl AsRef<str>) -> Result<Self> {
        let v = HeaderValue::from_str(&format!("Bearer {}", token.as_ref()))
            .map_err(|e| A2aError::Other(format!("invalid bearer header: {e}")))?;
        self.auth_header = Some(v);
        Ok(self)
    }

    /// 직접 헤더 지정 (OAuth2 access token 등 동일 형식이면 그대로).
    pub fn with_auth_header(mut self, value: HeaderValue) -> Self {
        self.auth_header = Some(value);
        self
    }

    /// base_url 조회.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// AgentCard 가져오기 — `/.well-known/agent-card.json` GET.
    pub async fn discover(&self) -> Result<AgentCard> {
        let url = self
            .base_url
            .join(WELL_KNOWN_AGENT_CARD)
            .map_err(|e| A2aError::InvalidUrl(format!("agent-card join: {e}")))?;

        let mut req = self.http.get(url.clone());
        if let Some(h) = &self.auth_header {
            req = req.header(AUTHORIZATION, h.clone());
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(A2aError::AgentCardFetch {
                url: url.to_string(),
                status: status.as_u16(),
            });
        }
        let card: AgentCard = resp.json().await?;
        Ok(card)
    }

    /// JSON-RPC 호출 — base_url 에 POST.
    ///
    /// 반환: `result` 필드를 `T` 로 역직렬화.
    pub async fn rpc<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let id = Uuid::new_v4().to_string();
        let body = json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(h) = &self.auth_header {
            headers.insert(AUTHORIZATION, h.clone());
        }

        tracing::debug!(method = %method, "a2a rpc dispatch");
        let resp = self
            .http
            .post(self.base_url.clone())
            .headers(headers)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let v: Value = resp.json().await?;
        if let Some(err) = v.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("(no message)")
                .to_string();
            return Err(A2aError::RpcError { code, message });
        }
        let result = v.get("result").ok_or(A2aError::InvalidRpcResponse)?;
        let parsed: T = serde_json::from_value(result.clone())?;
        Ok(parsed)
    }

    /// `tasks/send` — 새 작업 전송.
    ///
    /// `skill` 은 AgentCard skill id, `params` 는 skill 별 입력.
    /// 표준 모양: `{ id, message, sessionId? }`. 여기선 user_text 메시지로 감쌈.
    pub async fn send_task(
        &self,
        skill: &str,
        params: Value,
        session_id: Option<String>,
    ) -> Result<Task> {
        let task_id = Uuid::new_v4().to_string();
        let mut send_params = json!({
            "id": task_id,
            "skill": skill,
            "message": {
                "role": "user",
                "parts": [{ "type": "data", "data": params }],
            }
        });
        if let Some(sid) = session_id {
            send_params["sessionId"] = json!(sid);
        }
        self.rpc("tasks/send", send_params).await
    }

    /// `tasks/get` — 작업 상태/결과 조회.
    pub async fn get_task(&self, task_id: &str) -> Result<Task> {
        self.rpc("tasks/get", json!({ "id": task_id })).await
    }

    /// `tasks/cancel` — 작업 취소 요청.
    pub async fn cancel_task(&self, task_id: &str) -> Result<Task> {
        self.rpc("tasks/cancel", json!({ "id": task_id })).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_url() {
        let r = A2aClient::new("not a url");
        assert!(matches!(r, Err(A2aError::InvalidUrl(_))));
    }

    #[test]
    fn accepts_https_url() {
        let c = A2aClient::new("https://x.example/agent").unwrap();
        assert_eq!(c.base_url().scheme(), "https");
    }

    #[test]
    fn bearer_sets_header() {
        let c = A2aClient::new("https://x.example")
            .unwrap()
            .with_bearer("tok-123")
            .unwrap();
        assert!(c.auth_header.is_some());
        let v = c.auth_header.unwrap();
        assert_eq!(v.to_str().unwrap(), "Bearer tok-123");
    }
}
