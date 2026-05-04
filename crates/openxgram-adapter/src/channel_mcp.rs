//! Starian Channel MCP HTTP gateway 클라이언트.
//!
//! Channel MCP 는 다중 에이전트 메시지 라우팅 허브로,
//! discord/telegram/slack/kakaotalk/webhook 5개 어댑터를 통합 인터페이스로
//! 노출한다 (`send_to_platform` 도구). 또한 역할명 기반 피어 라우팅
//! (`send_message`) 과 어댑터 목록 조회 (`list_adapters`) 도구를 제공한다.
//!
//! 본 클라이언트는 channel-mcp 의 **HTTP gateway** 가 표준 MCP JSON-RPC
//! `tools/call` 엔드포인트(`POST /mcp`) 를 노출한다고 가정한다. 만약 호스트가
//! 다른 경로/스키마를 사용하면 [`ChannelMcpClient::with_endpoint`] 로 교체한다.
//!
//! ## fallback 금지
//!
//! base_url / 인증 토큰 누락은 호출자(CLI) 가 명시적으로 raise 한다. 본 모듈은
//! 빈 base_url 자체를 거부하지 않으며, HTTP 호출 단계에서 자연스럽게 실패한다.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{AdapterError, Result};

/// channel-mcp HTTP gateway 의 기본 tool 호출 엔드포인트.
///
/// 표준 MCP HTTP transport 는 `POST /mcp` 에서 JSON-RPC 메시지를 받는다.
/// 호스트가 다른 경로를 쓰면 [`ChannelMcpClient::with_endpoint`] 로 교체.
pub const DEFAULT_TOOL_ENDPOINT: &str = "/mcp";

/// channel-mcp 의 send_to_platform / send_message / list_adapters 결과
/// 공통 표현. 성공 시 `success=true`, 실패 시 `error` 메시지.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSendResult {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// channel-mcp `list_adapters` 도구가 반환하는 어댑터 메타데이터.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub platform: String,
    /// 어댑터가 현재 연결되어 있는가.
    #[serde(default)]
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// HTTP 로 channel-mcp 의 도구를 호출하는 thin client.
pub struct ChannelMcpClient {
    base_url: String,
    endpoint: String,
    auth_token: Option<String>,
    client: reqwest::Client,
}

impl ChannelMcpClient {
    /// `base_url` 예: `http://localhost:7100`. trailing slash 자동 제거.
    pub fn new(base_url: impl Into<String>, auth_token: Option<String>) -> Self {
        let mut base = base_url.into();
        while base.ends_with('/') {
            base.pop();
        }
        Self {
            base_url: base,
            endpoint: DEFAULT_TOOL_ENDPOINT.to_string(),
            auth_token,
            client: reqwest::Client::new(),
        }
    }

    /// 호스트가 표준 `/mcp` 가 아닌 다른 경로를 쓰면 교체.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    fn url(&self) -> String {
        let mut ep = self.endpoint.clone();
        if !ep.starts_with('/') {
            ep.insert(0, '/');
        }
        format!("{}{}", self.base_url, ep)
    }

    /// channel-mcp `tools/call` JSON-RPC 호출. MCP 표준 형식 (2.0).
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
            }
        });

        let mut req = self.client.post(self.url()).json(&body);
        if let Some(tok) = &self.auth_token {
            req = req.bearer_auth(tok);
        }
        let resp = req.send().await?;
        let resp = check_status_keep(resp).await?;
        let raw: Value = resp.json().await?;

        // JSON-RPC 응답: {"result": {...}} 또는 {"error": {"message": "..."}}.
        if let Some(err) = raw.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown channel-mcp error");
            return Err(AdapterError::ServerError {
                status: 200,
                body: msg.to_string(),
            });
        }
        // result 가 없으면 raw 자체를 반환 (REST 스타일 호스트 호환).
        Ok(raw.get("result").cloned().unwrap_or(raw))
    }

    /// `send_to_platform(platform, channel_id, text, reply_to?)` 호출.
    pub async fn send_to_platform(
        &self,
        platform: &str,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<ChannelSendResult> {
        let mut args = json!({
            "platform": platform,
            "channel_id": channel_id,
            "text": text,
        });
        if let Some(r) = reply_to {
            args["reply_to"] = json!(r);
        }
        let v = self.call_tool("send_to_platform", args).await?;
        Ok(parse_send_result(&v))
    }

    /// `send_message(to, summary, type)` — 피어 (역할명) 라우팅.
    pub async fn send_message(
        &self,
        to_role: &str,
        summary: &str,
        msg_type: &str,
    ) -> Result<ChannelSendResult> {
        let args = json!({
            "to": to_role,
            "summary": summary,
            "type": msg_type,
        });
        let v = self.call_tool("send_message", args).await?;
        Ok(parse_send_result(&v))
    }

    /// `list_adapters()` — 등록된 플랫폼 목록 + 연결 상태.
    pub async fn list_adapters(&self) -> Result<Vec<AdapterInfo>> {
        let v = self.call_tool("list_adapters", json!({})).await?;
        Ok(parse_adapters(&v))
    }
}

/// MCP 표준은 `result.content[0].text` 에 JSON 문자열을 담는 경우가 많다.
/// 다음 우선순위로 ChannelSendResult 를 추출:
/// 1. `{"success":..,"message_id":..}` 형태로 바로 해석 가능
/// 2. `result.content[*].text` 에 JSON 문자열이 있으면 디코드
/// 3. 그 외 — `success=true` 로 해석 (서버가 200 OK 만 반환한 경우)
fn parse_send_result(v: &Value) -> ChannelSendResult {
    if let Ok(r) = serde_json::from_value::<ChannelSendResult>(v.clone()) {
        return r;
    }
    if let Some(content) = v.get("content").and_then(Value::as_array) {
        for item in content {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                if let Ok(r) = serde_json::from_str::<ChannelSendResult>(text) {
                    return r;
                }
            }
        }
    }
    ChannelSendResult {
        success: true,
        message_id: None,
        error: None,
    }
}

fn parse_adapters(v: &Value) -> Vec<AdapterInfo> {
    if let Ok(list) = serde_json::from_value::<Vec<AdapterInfo>>(v.clone()) {
        return list;
    }
    if let Some(arr) = v.get("adapters").and_then(Value::as_array) {
        if let Ok(list) = serde_json::from_value::<Vec<AdapterInfo>>(Value::Array(arr.clone())) {
            return list;
        }
    }
    if let Some(content) = v.get("content").and_then(Value::as_array) {
        for item in content {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                if let Ok(list) = serde_json::from_str::<Vec<AdapterInfo>>(text) {
                    return list;
                }
            }
        }
    }
    Vec::new()
}

/// `check_status` 와 동일하지만 응답을 소비하지 않고 돌려준다 (body 를 JSON 으로
/// 다시 읽어야 하므로).
async fn check_status_keep(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(AdapterError::ServerError {
        status: status.as_u16(),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn send_to_platform_parses_jsonrpc_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "success": true,
                    "message_id": "abc-123"
                }
            })))
            .mount(&server)
            .await;

        let client = ChannelMcpClient::new(server.uri(), None);
        let r = client
            .send_to_platform("discord", "12345", "hello", None)
            .await
            .unwrap();
        assert!(r.success);
        assert_eq!(r.message_id.as_deref(), Some("abc-123"));
    }

    #[tokio::test]
    async fn send_message_handles_content_text_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "content": [
                        {"type": "text", "text": "{\"success\":true,\"message_id\":\"peer-7\"}"}
                    ]
                }
            })))
            .mount(&server)
            .await;

        let client = ChannelMcpClient::new(server.uri(), Some("tok".into()));
        let r = client.send_message("res", "조사 부탁", "request").await.unwrap();
        assert!(r.success);
        assert_eq!(r.message_id.as_deref(), Some("peer-7"));
    }

    #[tokio::test]
    async fn list_adapters_parses_array() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": [
                    {"platform": "discord", "connected": true, "channel_id": "111"},
                    {"platform": "telegram", "connected": false}
                ]
            })))
            .mount(&server)
            .await;

        let client = ChannelMcpClient::new(server.uri(), None);
        let v = client.list_adapters().await.unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].platform, "discord");
        assert!(v[0].connected);
    }

    #[tokio::test]
    async fn jsonrpc_error_is_returned_as_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32603, "message": "tool not found"}
            })))
            .mount(&server)
            .await;

        let client = ChannelMcpClient::new(server.uri(), None);
        let err = client.list_adapters().await.unwrap_err();
        match err {
            AdapterError::ServerError { body, .. } => assert!(body.contains("tool not found")),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
