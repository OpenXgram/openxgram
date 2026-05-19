//! reqwest 기반 OpenAgentX 마켓플레이스 HTTP 클라이언트.
//!
//! base_url 기본값: `https://openagentx.org`. config로 override 가능 (테스트에 mockito 사용).
//!
//! 인증: Bearer API key (선택). API key가 없으면 검색 같은 공개 엔드포인트만 동작.

use crate::agent::{Agent, AgentId, Job, JobId, NewJobRequest};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// 기본 base URL.
pub const DEFAULT_BASE_URL: &str = "https://openagentx.org";

/// HTTP 요청 timeout (초).
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// User-Agent 헤더.
pub const USER_AGENT: &str = concat!("openxgram-marketplace/", env!("CARGO_PKG_VERSION"));

/// HTTP 클라이언트 에러.
#[derive(Debug, Error)]
pub enum MarketplaceClientError {
    /// reqwest.
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    /// HTTP 상태 코드가 2xx가 아님.
    #[error("status {status}: {body}")]
    Status {
        /// HTTP status.
        status: u16,
        /// 응답 본문 (truncated).
        body: String,
    },

    /// URL 빌드 실패.
    #[error("url: {0}")]
    Url(String),

    /// 직렬화·역직렬화.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}

/// OpenAgentX 마켓플레이스 HTTP 클라이언트.
#[derive(Debug, Clone)]
pub struct MarketplaceClient {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl MarketplaceClient {
    /// 기본 클라이언트 (`https://openagentx.org`, API key 없음).
    pub fn new() -> Result<Self, MarketplaceClientError> {
        Self::builder().build()
    }

    /// 빌더.
    pub fn builder() -> MarketplaceClientBuilder {
        MarketplaceClientBuilder::default()
    }

    /// 현재 base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// API key 등록 여부.
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }

    /// `GET /api/agents?q=<query>&limit=<n>` — 검색.
    pub async fn search_agents(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Agent>, MarketplaceClientError> {
        let mut url = format!("{}/api/agents?q={}", self.base_url, urlencode(query));
        if let Some(n) = limit {
            url.push_str(&format!("&limit={n}"));
        }
        let req = self.attach_auth(self.http.get(url));
        let resp = req.send().await?;
        let resp = ensure_ok(resp).await?;
        let envelope: SearchEnvelope = resp.json().await?;
        Ok(envelope.into_agents())
    }

    /// `GET /api/agents/[id]` — 단일 에이전트 + 서비스 목록.
    pub async fn get_agent(
        &self,
        agent_id: &AgentId,
    ) -> Result<Agent, MarketplaceClientError> {
        let req = self
            .http
            .get(format!(
                "{}/api/agents/{}",
                self.base_url,
                urlencode(agent_id.as_str())
            ));
        let req = self.attach_auth(req);
        let resp = req.send().await?;
        let resp = ensure_ok(resp).await?;
        let envelope: SingleEnvelope<Agent> = resp.json().await?;
        Ok(envelope.into_inner())
    }

    /// `POST /api/jobs` — 작업 발주. 결제 토큰·서명은 별도 헤더로 첨부 가능.
    pub async fn create_job(
        &self,
        request: &NewJobRequest,
        payment_token: Option<&str>,
    ) -> Result<Job, MarketplaceClientError> {
        let body = JobCreateBody {
            agent_id: request.agent_id.as_str().to_string(),
            service_id: request.service_id.as_str().to_string(),
            input: request.input.clone(),
            max_price_usdc_micro: request.max_price_usdc_micro,
        };
        let mut req = self
            .http
            .post(format!("{}/api/jobs", self.base_url))
            .json(&body);
        if let Some(tok) = payment_token {
            req = req.header("X-Payment-Tx", tok);
        }
        req = self.attach_auth(req);
        let resp = req.send().await?;
        let resp = ensure_ok(resp).await?;
        let envelope: SingleEnvelope<Job> = resp.json().await?;
        Ok(envelope.into_inner())
    }

    /// `GET /api/jobs/[id]` — 작업 상태.
    pub async fn get_job(&self, job_id: &JobId) -> Result<Job, MarketplaceClientError> {
        let req = self.http.get(format!(
            "{}/api/jobs/{}",
            self.base_url,
            urlencode(job_id.as_str())
        ));
        let req = self.attach_auth(req);
        let resp = req.send().await?;
        let resp = ensure_ok(resp).await?;
        let envelope: SingleEnvelope<Job> = resp.json().await?;
        Ok(envelope.into_inner())
    }

    fn attach_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(k) = &self.api_key {
            req.bearer_auth(k)
        } else {
            req
        }
    }
}

/// 빌더.
#[derive(Debug, Default)]
pub struct MarketplaceClientBuilder {
    base_url: Option<String>,
    api_key: Option<String>,
    timeout: Option<Duration>,
}

impl MarketplaceClientBuilder {
    /// base_url 지정 (테스트에서 mockito URL 주입).
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// API key (Bearer 인증).
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// 타임아웃.
    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = Some(t);
        self
    }

    /// 빌드.
    pub fn build(self) -> Result<MarketplaceClient, MarketplaceClientError> {
        let base_url = self
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        if base_url.is_empty() {
            return Err(MarketplaceClientError::Url("base_url empty".into()));
        }
        let timeout = self.timeout.unwrap_or(Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .build()?;
        Ok(MarketplaceClient {
            base_url,
            api_key: self.api_key,
            http,
        })
    }
}

// ---- private envelope types -------------------------------------------------

/// 검색 응답 — 마켓이 `{"agents": [...]}` 또는 그냥 `[...]` 둘 다 받아주도록 untagged.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SearchEnvelope {
    Wrapped { agents: Vec<Agent> },
    Bare(Vec<Agent>),
}

impl SearchEnvelope {
    fn into_agents(self) -> Vec<Agent> {
        match self {
            SearchEnvelope::Wrapped { agents } => agents,
            SearchEnvelope::Bare(v) => v,
        }
    }
}

/// 단일 객체 응답 — `{"agent": {...}}` / `{"job": {...}}` / 또는 그냥 객체.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SingleEnvelope<T> {
    Wrapped {
        #[serde(alias = "agent", alias = "job", alias = "data")]
        inner: T,
    },
    Bare(T),
}

impl<T> SingleEnvelope<T> {
    fn into_inner(self) -> T {
        match self {
            SingleEnvelope::Wrapped { inner } => inner,
            SingleEnvelope::Bare(t) => t,
        }
    }
}

#[derive(Debug, Serialize)]
struct JobCreateBody {
    agent_id: String,
    service_id: String,
    input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_price_usdc_micro: Option<i64>,
}

async fn ensure_ok(resp: reqwest::Response) -> Result<reqwest::Response, MarketplaceClientError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let truncated: String = body.chars().take(512).collect();
    Err(MarketplaceClientError::Status {
        status: status.as_u16(),
        body: truncated,
    })
}

/// 매우 단순한 URL path 이스케이프 — `/`, `?`, `#`, 공백만 처리.
/// (서비스 ID에 특수문자 거의 없음을 가정.)
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(b as char);
            }
            other => {
                out.push_str(&format!("%{other:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_defaults() {
        let c = MarketplaceClient::new().unwrap();
        assert_eq!(c.base_url(), DEFAULT_BASE_URL);
        assert!(!c.has_api_key());
    }

    #[test]
    fn builder_overrides() {
        let c = MarketplaceClient::builder()
            .base_url("http://localhost:3000/")
            .api_key("k123")
            .build()
            .unwrap();
        assert_eq!(c.base_url(), "http://localhost:3000");
        assert!(c.has_api_key());
    }

    #[test]
    fn urlencode_safe_chars() {
        assert_eq!(urlencode("agent:abc-1"), "agent:abc-1");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("x y"), "x%20y");
    }

    #[test]
    fn empty_base_url_rejected() {
        let res = MarketplaceClient::builder().base_url("").build();
        assert!(res.is_err());
    }
}
