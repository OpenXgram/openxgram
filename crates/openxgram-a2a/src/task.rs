//! Task / Message / TaskState — A2A 작업 단위 도메인.
//!
//! 정본: https://a2a-protocol.org/latest/specification/#task

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::A2aError;

/// A2A 작업 상태.
///
/// 표준 6개 + 호환을 위한 raw 변환.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskState {
    /// 막 submit 된 상태.
    Submitted,
    /// 처리 중.
    Working,
    /// 사용자/호출자 추가 입력 대기.
    InputRequired,
    /// 완료.
    Completed,
    /// 취소됨.
    Canceled,
    /// 실패.
    Failed,
}

impl TaskState {
    /// 최종(terminal) 상태인지.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Failed)
    }

    /// kebab-case 문자열.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Working => "working",
            Self::InputRequired => "input-required",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskState {
    type Err = A2aError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "submitted" => Ok(Self::Submitted),
            "working" => Ok(Self::Working),
            "input-required" | "input_required" => Ok(Self::InputRequired),
            "completed" => Ok(Self::Completed),
            "canceled" | "cancelled" => Ok(Self::Canceled),
            "failed" => Ok(Self::Failed),
            other => Err(A2aError::UnknownTaskState(other.to_string())),
        }
    }
}

/// 메시지 (사용자 또는 에이전트의 한 turn).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// "user" 또는 "agent".
    pub role: String,
    /// 메시지 parts.
    #[serde(default)]
    pub parts: Vec<Part>,
}

impl Message {
    /// 단일 text part 로 user 메시지 생성.
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            parts: vec![Part::text(text)],
        }
    }
}

/// 메시지 part — text / data / file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Part {
    /// 텍스트 part.
    Text {
        /// 본문.
        text: String,
    },
    /// 구조화된 데이터 part.
    Data {
        /// JSON 페이로드.
        data: serde_json::Value,
    },
    /// 파일 part (raw bytes 또는 URI).
    File {
        /// 파일 객체 (MIME / URI / bytes).
        file: serde_json::Value,
    },
}

impl Part {
    /// 텍스트 part 생성자.
    pub fn text(t: impl Into<String>) -> Self {
        Self::Text { text: t.into() }
    }
}

/// A2A 작업 결과 객체.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// task id.
    pub id: String,
    /// session/context id (선택).
    #[serde(default, rename = "sessionId", alias = "session_id", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 현재 상태.
    pub status: TaskStatus,
    /// 누적된 메시지 history (선택).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
    /// 결과 artifacts (선택).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<serde_json::Value>,
    /// 표준 외 필드.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Task status 래퍼 (state + 선택 timestamp/메시지).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatus {
    /// 상태.
    pub state: TaskState,
    /// 마지막 메시지 (선택).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    /// 마지막 갱신 ISO-8601 (선택).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip() {
        for s in [
            TaskState::Submitted,
            TaskState::Working,
            TaskState::InputRequired,
            TaskState::Completed,
            TaskState::Canceled,
            TaskState::Failed,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: TaskState = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
            let parsed: TaskState = s.as_str().parse().unwrap();
            assert_eq!(s, parsed);
        }
    }

    #[test]
    fn unknown_state_errors() {
        let r: Result<TaskState, _> = "bogus".parse();
        assert!(matches!(r, Err(A2aError::UnknownTaskState(_))));
    }

    #[test]
    fn terminal_predicate() {
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Canceled.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(!TaskState::Working.is_terminal());
        assert!(!TaskState::Submitted.is_terminal());
        assert!(!TaskState::InputRequired.is_terminal());
    }

    #[test]
    fn message_user_text_helper() {
        let m = Message::user_text("hi");
        assert_eq!(m.role, "user");
        assert_eq!(m.parts.len(), 1);
        match &m.parts[0] {
            Part::Text { text } => assert_eq!(text, "hi"),
            _ => panic!("expected text part"),
        }
    }

    #[test]
    fn task_parses_minimal() {
        let json = r#"{
            "id": "t-1",
            "status": { "state": "working" }
        }"#;
        let t: Task = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "t-1");
        assert_eq!(t.status.state, TaskState::Working);
        assert!(t.history.is_empty());
    }

    #[test]
    fn part_serializes_with_type_tag() {
        let p = Part::text("hi");
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains(r#""type":"text""#));
        assert!(s.contains(r#""text":"hi""#));
    }
}
