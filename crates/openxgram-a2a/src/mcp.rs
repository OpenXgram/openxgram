//! MCP 도구 핸들러 — 4개 신규 도구 (PRD-OpenXgram §4.5).
//!
//! - `a2a_discover(url)`                                → AgentCard JSON
//! - `a2a_send_task(agent_url, skill, params, ...)`     → Task
//! - `a2a_get_task(agent_url, task_id)`                 → Task
//! - `a2a_cancel_task(agent_url, task_id)`              → Task
//!
//! 본 모듈은 도메인 핸들러만 제공. JSON-RPC 어댑터는 openxgram-mcp가 래핑.
//!
//! 캐시: agent-card 호출은 in-memory LRU 없이 매번 신선 조회 (Phase v0.7).
//! 추후 §4.5 캐시 정책 결정 후 추가.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_card::AgentCard;
use crate::client::A2aClient;
use crate::task::Task;
use crate::A2aError;

/// A2A MCP 도구 묶음.
///
/// stateless — 매 호출마다 새 `A2aClient` 를 만든다.
/// 인증 토큰은 호출자가 별도 메커니즘 (예: vault) 으로 주입해야 한다.
/// 토큰 없는 호출은 unauthenticated 로 그대로 전송.
#[derive(Debug, Default, Clone)]
pub struct A2aTools {
    /// 선택: 모든 호출에 같은 bearer 사용.
    /// 호출별 다른 인증이 필요하면 `with_bearer_per_call` 빌더 패턴 추가.
    default_bearer: Option<String>,
}

impl A2aTools {
    /// 신규 stateless 핸들러.
    pub fn new() -> Self {
        Self::default()
    }

    /// 모든 호출에 적용할 기본 bearer 토큰.
    pub fn with_default_bearer(mut self, token: impl Into<String>) -> Self {
        self.default_bearer = Some(token.into());
        self
    }

    fn client(&self, base_url: &str) -> Result<A2aClient, A2aError> {
        let mut c = A2aClient::new(base_url)?;
        if let Some(tok) = &self.default_bearer {
            c = c.with_bearer(tok)?;
        }
        Ok(c)
    }

    /// `a2a_discover` — AgentCard 가져오기.
    pub async fn discover(&self, url: &str) -> Result<AgentCard, A2aError> {
        let client = self.client(url)?;
        client.discover().await
    }

    /// `a2a_send_task` — 작업 전송.
    pub async fn send_task(&self, args: SendTaskArgs) -> Result<Task, A2aError> {
        let client = self.client(&args.agent_url)?;
        client
            .send_task(&args.skill, args.params, args.session_id)
            .await
    }

    /// `a2a_get_task` — 작업 조회.
    pub async fn get_task(&self, agent_url: &str, task_id: &str) -> Result<Task, A2aError> {
        let client = self.client(agent_url)?;
        client.get_task(task_id).await
    }

    /// `a2a_cancel_task` — 작업 취소.
    pub async fn cancel_task(&self, agent_url: &str, task_id: &str) -> Result<Task, A2aError> {
        let client = self.client(agent_url)?;
        client.cancel_task(task_id).await
    }
}

/// `a2a_send_task` 인자.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendTaskArgs {
    /// A2A 에이전트 base URL.
    pub agent_url: String,
    /// AgentCard 의 skill id.
    pub skill: String,
    /// skill 별 파라미터 (자유 JSON).
    #[serde(default = "default_params")]
    pub params: Value,
    /// 세션/컨텍스트 id (선택).
    #[serde(default)]
    pub session_id: Option<String>,
}

fn default_params() -> Value {
    Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_constructs_default() {
        let t = A2aTools::new();
        assert!(t.default_bearer.is_none());
    }

    #[test]
    fn tools_with_bearer() {
        let t = A2aTools::new().with_default_bearer("tok");
        assert_eq!(t.default_bearer.as_deref(), Some("tok"));
    }

    #[test]
    fn send_args_deserializes_minimal() {
        let json = r#"{"agent_url":"https://x.example","skill":"translate"}"#;
        let args: SendTaskArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.agent_url, "https://x.example");
        assert_eq!(args.skill, "translate");
        assert!(args.session_id.is_none());
    }

    #[test]
    fn send_args_invalid_url_propagates() {
        let t = A2aTools::new();
        let r = t.client("not a url");
        assert!(matches!(r, Err(A2aError::InvalidUrl(_))));
    }
}
