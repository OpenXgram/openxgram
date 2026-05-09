//! 1.8.1.2 SubInvoker enum dispatcher — 메인 에이전트가 서브에이전트(@eno @qua @res …) 위임 시 호출.
//!
//! 현재 variant:
//! - `Stub` — 외부 호출 없이 dialogue 시뮬 (`[<role>]: ack <task>` 한 줄)
//! - `OpenAgentX { base_url, token }` — OpenAgentX 호환 HTTP API (POST `/agents/{role}/invoke`)
//! - `ChannelHttp { base_url, token }` — Starian Channel HTTP bridge (POST `/send`)
//!
//! 모든 variant 는 같은 trait-shaped `invoke(role, task) -> Result<String>` 인터페이스.
//! 호출자는 enum 으로 dispatch — Generator 가 LLM 출력을 파싱해서 위임 패턴
//! `@<role> {task}` 발견 시 호출하는 구조 (다음 PR 통합).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum SubInvoker {
    /// 무 외부 호출 — 단순 시뮬 응답 (단위 테스트 / single-LLM dialogue 모드).
    Stub,
    /// OpenAgentX HTTP 라우팅 — `POST {base_url}/agents/{role}/invoke`.
    OpenAgentX { base_url: String, token: String },
    /// Starian Channel HTTP bridge — `POST {base_url}/send` (to: role, body: task).
    ChannelHttp { base_url: String, token: String },
}

#[derive(Serialize)]
struct InvokeReq<'a> {
    role: &'a str,
    task: &'a str,
}

#[derive(Deserialize)]
struct InvokeResp {
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

impl SubInvoker {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Stub => "stub",
            Self::OpenAgentX { .. } => "openagentx-http",
            Self::ChannelHttp { .. } => "channel-http",
        }
    }

    /// env 기반 자동 선택:
    /// - `XGRAM_OPENAGENTX_URL` + `XGRAM_OPENAGENTX_TOKEN` → OpenAgentX
    /// - `XGRAM_CHANNEL_HTTP_URL` + `XGRAM_CHANNEL_HTTP_TOKEN` → ChannelHttp
    /// - 둘 다 없으면 Stub
    pub fn from_env() -> Self {
        if let (Ok(u), Ok(t)) = (
            std::env::var("XGRAM_OPENAGENTX_URL"),
            std::env::var("XGRAM_OPENAGENTX_TOKEN"),
        ) {
            if !u.trim().is_empty() && !t.trim().is_empty() {
                return Self::OpenAgentX {
                    base_url: u,
                    token: t,
                };
            }
        }
        if let (Ok(u), Ok(t)) = (
            std::env::var("XGRAM_CHANNEL_HTTP_URL"),
            std::env::var("XGRAM_CHANNEL_HTTP_TOKEN"),
        ) {
            if !u.trim().is_empty() && !t.trim().is_empty() {
                return Self::ChannelHttp {
                    base_url: u,
                    token: t,
                };
            }
        }
        Self::Stub
    }

    /// 서브에이전트 한 번 호출. role 은 alias (예: "eno"), task 는 평문 지시.
    pub async fn invoke(
        &self,
        http: &reqwest::Client,
        role: &str,
        task: &str,
    ) -> Result<String> {
        match self {
            Self::Stub => Ok(format!("[{role}]: ack `{task}` (stub)")),
            Self::OpenAgentX { base_url, token } => {
                let url = format!(
                    "{}/agents/{}/invoke",
                    base_url.trim_end_matches('/'),
                    role
                );
                let resp = http
                    .post(&url)
                    .bearer_auth(token)
                    .json(&InvokeReq { role, task })
                    .send()
                    .await
                    .context("openagentx POST")?;
                if !resp.status().is_success() {
                    anyhow::bail!("openagentx HTTP {}", resp.status());
                }
                let parsed: InvokeResp = resp.json().await.context("openagentx JSON")?;
                Ok(parsed
                    .response
                    .or(parsed.text)
                    .unwrap_or_else(|| format!("[{role}]: (no response field)")))
            }
            Self::ChannelHttp { base_url, token } => {
                let url = format!("{}/send", base_url.trim_end_matches('/'));
                let resp = http
                    .post(&url)
                    .bearer_auth(token)
                    .json(&InvokeReq { role, task })
                    .send()
                    .await
                    .context("channel http POST")?;
                if !resp.status().is_success() {
                    anyhow::bail!("channel http HTTP {}", resp.status());
                }
                let parsed: InvokeResp = resp.json().await.context("channel JSON")?;
                Ok(parsed
                    .response
                    .or(parsed.text)
                    .unwrap_or_else(|| format!("[{role}]: (no response field)")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_invoker_returns_ack_line() {
        let http = reqwest::Client::new();
        let inv = SubInvoker::Stub;
        let out = inv.invoke(&http, "eno", "리뷰 부탁").await.unwrap();
        assert_eq!(out, "[eno]: ack `리뷰 부탁` (stub)");
        assert_eq!(inv.label(), "stub");
    }

    #[test]
    fn from_env_returns_stub_when_no_env() {
        unsafe {
            std::env::remove_var("XGRAM_OPENAGENTX_URL");
            std::env::remove_var("XGRAM_OPENAGENTX_TOKEN");
            std::env::remove_var("XGRAM_CHANNEL_HTTP_URL");
            std::env::remove_var("XGRAM_CHANNEL_HTTP_TOKEN");
        }
        assert!(matches!(SubInvoker::from_env(), SubInvoker::Stub));
    }

    #[tokio::test]
    async fn openagentx_invoker_calls_correct_url() {
        // Use wiremock-style: spawn axum receiver
        use axum::routing::post;
        use axum::{Json, Router};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let bind: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

        async fn handler(Json(b): Json<serde_json::Value>) -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "response": format!("got role={} task={}", b["role"], b["task"])
            }))
        }
        let app = Router::new().route("/agents/{role}/invoke", post(handler));
        let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let inv = SubInvoker::OpenAgentX {
            base_url: format!("http://127.0.0.1:{port}"),
            token: "test-tok".into(),
        };
        let http = reqwest::Client::new();
        let out = inv.invoke(&http, "eno", "fix bug").await.unwrap();
        assert!(out.contains("eno"));
        assert!(out.contains("fix bug"));
    }
}
