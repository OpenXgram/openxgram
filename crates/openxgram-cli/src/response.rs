//! 메인 에이전트 응답 백엔드 — Echo / Anthropic / (예정) OpenAgentX 의 단일 진입점.
//!
//! 1.7.1 ResponseGenerator: enum dispatcher 로 단일 SOT.
//! 1.7.3.4 context: 최근 N개 inbox 메시지를 LLM 호출 messages 에 동봉.
//! 1.7.3.5 토큰 / 비용 로깅: Anthropic `usage` 필드 → tracing::info.

use anyhow::{Context, Result};
use openxgram_memory::Message;
use serde::{Deserialize, Serialize};

/// 단일 응답 호출의 결과 — 본문 + 서명(signature) 컬럼에 들어갈 식별자.
#[derive(Debug, Clone)]
pub struct GeneratorOutput {
    pub body: String,
    pub signature: &'static str,
}

/// 응답 백엔드 선택. 새 백엔드는 enum variant + match arm 로 추가.
#[derive(Debug, Clone)]
pub enum Generator {
    /// 무 LLM fallback — 입력 첫 줄을 echo.
    Echo,
    /// Anthropic Claude. `api_key` 는 호출자가 env 에서 주입.
    Anthropic { api_key: String },
}

impl Generator {
    /// env 기반 자동 선택 — `XGRAM_ANTHROPIC_API_KEY` 가 있으면 Anthropic, 없으면 Echo.
    pub fn from_env() -> Self {
        match std::env::var("XGRAM_ANTHROPIC_API_KEY") {
            Ok(key) if !key.trim().is_empty() => Self::Anthropic { api_key: key },
            _ => Self::Echo,
        }
    }

    /// 명시적으로 Anthropic 키를 옵션으로 받는 빌더 — agent.rs 의 기존 opts 와 호환.
    pub fn from_anthropic_key(key: Option<&str>) -> Self {
        match key {
            Some(k) if !k.trim().is_empty() => Self::Anthropic {
                api_key: k.to_string(),
            },
            _ => Self::Echo,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Echo => "echo-v0",
            Self::Anthropic { .. } => "anthropic-haiku-4.5",
        }
    }

    /// 메인 응답 생성. `history` 는 같은 conversation 의 이전 inbox/outbox 메시지 (시간순).
    /// 마지막 항목이 응답 대상이라고 가정 (호출자가 보장).
    pub async fn generate(
        &self,
        http: &reqwest::Client,
        alias: &str,
        input: &str,
        history: &[Message],
    ) -> Result<GeneratorOutput> {
        match self {
            Self::Echo => Ok(GeneratorOutput {
                body: echo_body(input),
                signature: "echo-v0",
            }),
            Self::Anthropic { api_key } => match anthropic_call(http, api_key, alias, input, history).await {
                Ok(body) => Ok(GeneratorOutput {
                    body,
                    signature: "anthropic-haiku-4.5",
                }),
                Err(e) => {
                    tracing::warn!(error = %e, "Anthropic 호출 실패 — echo fallback");
                    Ok(GeneratorOutput {
                        body: echo_body(input),
                        signature: "echo-v0-fallback",
                    })
                }
            },
        }
    }
}

/// 외부 노출 — 단위 테스트 (1.7.2.2) 가 직접 검증.
pub fn echo_body(input: &str) -> String {
    let trimmed = input.lines().next().unwrap_or(input).trim();
    if trimmed.is_empty() {
        "받았습니다.".to_string()
    } else {
        format!("받았습니다: {trimmed}")
    }
}

#[derive(Serialize)]
struct AnthropicMessageReq<'a> {
    model: &'a str,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResp {
    content: Vec<AnthropicContent>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
/// claude-haiku-4-5 단가 (USD per 1M tokens). 출처: Anthropic 공식 가격표 기준.
const HAIKU_INPUT_PER_M: f64 = 1.0;
const HAIKU_OUTPUT_PER_M: f64 = 5.0;
/// LLM 컨텍스트로 동봉할 최근 메시지 수 (시간순). 너무 많으면 토큰 폭발.
pub const HISTORY_WINDOW: usize = 8;

async fn anthropic_call(
    http: &reqwest::Client,
    api_key: &str,
    alias: &str,
    input: &str,
    history: &[Message],
) -> Result<String> {
    let system = std::env::var("XGRAM_AGENT_SYSTEM_PROMPT").unwrap_or_else(|_| {
        format!(
            "You are {alias}, an autonomous AI agent in the OpenXgram network. \
            Reply concisely in the user's language. Keep responses under 300 words.\n\n\
            Subagents available: @eno (engineering/coding), @qua (QA/verification), \
            @res (research), @pip (PRD/planning), @edu (learning), @law (legal), \
            @ai (SNS posting), @akashic (memory).\n\n\
            When the user asks you to delegate to a subagent, simulate the dialogue:\n\
            1. Acknowledge: \"@<role> 에게 위임합니다: <task>\"\n\
            2. Sub response: \"[<role>]: <what they would say>\"\n\
            3. Wrap-up: \"[{alias}]: <synthesis>\"\n\
            Otherwise, answer directly as {alias}."
        )
    });

    // 1.7.3.4 — 최근 N개 history 를 messages 에 동봉. 마지막 user turn 은 input 그대로.
    let mut messages: Vec<AnthropicMessage> = history
        .iter()
        .rev()
        .take(HISTORY_WINDOW)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|m| AnthropicMessage {
            role: history_role(m, alias).to_string(),
            content: m.body.clone(),
        })
        .collect();
    // 같은 conversation 안에서 마지막 메시지가 input 인 경우 중복 방지 — 끝 항목이 같은 body 면 제거.
    if matches!(messages.last(), Some(last) if last.role == "user" && last.content == input) {
        messages.pop();
    }
    // Anthropic API 는 대화가 'user' 로 시작해야 함 — 'assistant' 로 시작하는 prefix trim.
    while matches!(messages.first(), Some(first) if first.role == "assistant") {
        messages.remove(0);
    }
    messages.push(AnthropicMessage {
        role: "user".into(),
        content: input.to_string(),
    });

    let req = AnthropicMessageReq {
        model: HAIKU_MODEL,
        max_tokens: 1024,
        system,
        messages,
    };
    let resp = http
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&req)
        .send()
        .await
        .context("Anthropic POST")?;
    if !resp.status().is_success() {
        let st = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic HTTP {st}: {body}");
    }
    let parsed: AnthropicResp = resp.json().await.context("Anthropic JSON parse")?;

    // 1.7.3.5 — 토큰 / 비용 로깅
    if let Some(u) = parsed.usage.as_ref() {
        let cost_usd = (u.input_tokens as f64 / 1_000_000.0) * HAIKU_INPUT_PER_M
            + (u.output_tokens as f64 / 1_000_000.0) * HAIKU_OUTPUT_PER_M;
        tracing::info!(
            model = HAIKU_MODEL,
            input_tokens = u.input_tokens,
            output_tokens = u.output_tokens,
            cache_creation = u.cache_creation_input_tokens,
            cache_read = u.cache_read_input_tokens,
            cost_usd = format!("{cost_usd:.6}"),
            "anthropic usage"
        );
        eprintln!(
            "[agent][anthropic] in={} out={} (cache={}+{}) cost=${cost_usd:.6}",
            u.input_tokens,
            u.output_tokens,
            u.cache_creation_input_tokens,
            u.cache_read_input_tokens
        );
    }

    let text = parsed
        .content
        .into_iter()
        .filter(|c| c.kind == "text")
        .find_map(|c| c.text)
        .unwrap_or_else(|| "(no text content)".into());
    Ok(text)
}

/// agent_alias 가 발신자면 'assistant' role, 그 외는 'user' role 로 매핑.
fn history_role(m: &Message, alias: &str) -> &'static str {
    if m.sender == alias {
        "assistant"
    } else {
        "user"
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_returns_first_line_with_prefix() {
        assert_eq!(echo_body("hello"), "받았습니다: hello");
        assert_eq!(echo_body("first\nsecond"), "받았습니다: first");
    }

    #[test]
    fn echo_handles_empty_input() {
        assert_eq!(echo_body(""), "받았습니다.");
        assert_eq!(echo_body("   \n   "), "받았습니다.");
    }

    #[test]
    fn echo_trims_whitespace() {
        assert_eq!(echo_body("  hi  "), "받았습니다: hi");
    }

    #[test]
    fn from_anthropic_key_empty_string_falls_back_to_echo() {
        assert!(matches!(Generator::from_anthropic_key(None), Generator::Echo));
        assert!(matches!(
            Generator::from_anthropic_key(Some("   ")),
            Generator::Echo
        ));
        assert!(matches!(
            Generator::from_anthropic_key(Some("sk-test")),
            Generator::Anthropic { .. }
        ));
    }

    #[tokio::test]
    async fn echo_generator_returns_echo_signature() {
        let http = reqwest::Client::new();
        let g = Generator::Echo;
        let out = g.generate(&http, "Starian", "안녕", &[]).await.unwrap();
        assert_eq!(out.signature, "echo-v0");
        assert_eq!(out.body, "받았습니다: 안녕");
    }
}
