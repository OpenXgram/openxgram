//! 도메인 — `ActionPattern`, `ActionStep`, ID.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// 행동 패턴 ID — `action:<uuid>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionPatternId(pub String);

impl ActionPatternId {
    /// 신규.
    pub fn new() -> Self {
        Self(format!("action:{}", Uuid::new_v4()))
    }
    /// inner.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ActionPatternId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ActionPatternId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ActionPatternId {
    type Err = crate::PatternsError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("action:") {
            return Err(crate::PatternsError::Invalid(format!(
                "action pattern id must start with 'action:' — got {}",
                s
            )));
        }
        Ok(Self(s.to_string()))
    }
}

/// 시퀀스 한 단계.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionStep {
    /// 단계 설명 (사람이 읽음).
    pub step: String,
    /// 호출할 도구 (선택).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// 도구 인자 (JSON, 선택).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
}

/// 신규 행동 패턴 입력.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewActionPattern {
    /// 기존 L3 patterns(0004) id 참조.
    pub pattern_id: String,
    /// 단계 시퀀스 (1개 이상).
    pub action_sequence: Vec<ActionStep>,
}

impl NewActionPattern {
    /// 검증.
    pub fn validate(&self) -> Result<(), crate::PatternsError> {
        if self.pattern_id.trim().is_empty() {
            return Err(crate::PatternsError::Invalid("pattern_id required".into()));
        }
        if self.action_sequence.is_empty() {
            return Err(crate::PatternsError::Invalid(
                "action_sequence: at least 1 step required".into(),
            ));
        }
        for (i, step) in self.action_sequence.iter().enumerate() {
            if step.step.trim().is_empty() {
                return Err(crate::PatternsError::Invalid(format!(
                    "step[{}]: 'step' description required",
                    i
                )));
            }
        }
        Ok(())
    }
}

/// 영속 객체.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPattern {
    /// `action:<uuid>`.
    pub id: ActionPatternId,
    /// L3 pattern id (FK).
    pub pattern_id: String,
    /// 단계 시퀀스.
    pub action_sequence: Vec<ActionStep>,
    /// 성공 케이스 누적 평균 (ms).
    pub avg_duration_ms: Option<i64>,
    /// 성공 카운트.
    pub success_count: i64,
    /// 실패 카운트.
    pub failure_count: i64,
    /// 마지막 실행 (epoch ms, 성공·실패 무관).
    pub last_executed: Option<i64>,
    /// 임베딩 hash.
    pub embedding_hash: String,
    /// 생성.
    pub created_at: i64,
    /// 갱신.
    pub updated_at: i64,
}

impl ActionPattern {
    /// `NewActionPattern` → 영속.
    pub fn from_new(input: NewActionPattern) -> Result<Self, crate::PatternsError> {
        input.validate()?;
        let now = Utc::now().timestamp_millis();
        let hash = compute_embedding_hash(&input.action_sequence);
        Ok(Self {
            id: ActionPatternId::new(),
            pattern_id: input.pattern_id,
            action_sequence: input.action_sequence,
            avg_duration_ms: None,
            success_count: 0,
            failure_count: 0,
            last_executed: None,
            embedding_hash: hash,
            created_at: now,
            updated_at: now,
        })
    }

    /// 성공률 (0.0 ~ 1.0, 호출 0이면 None).
    pub fn success_rate(&self) -> Option<f64> {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            None
        } else {
            Some(self.success_count as f64 / total as f64)
        }
    }

    /// 시퀀스 요약 (LIKE 검색용 평면화).
    pub fn flatten_sequence(&self) -> String {
        self.action_sequence
            .iter()
            .map(|s| {
                let tool = s.tool.as_deref().unwrap_or("");
                if tool.is_empty() {
                    s.step.clone()
                } else {
                    format!("{} [{}]", s.step, tool)
                }
            })
            .collect::<Vec<_>>()
            .join(" → ")
    }
}

/// SHA-256 of JSON-serialized sequence.
pub fn compute_embedding_hash(seq: &[ActionStep]) -> String {
    let json = serde_json::to_string(seq).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(json.as_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(s: &str, tool: Option<&str>) -> ActionStep {
        ActionStep {
            step: s.into(),
            tool: tool.map(String::from),
            args: None,
        }
    }

    #[test]
    fn id_roundtrip() {
        let id = ActionPatternId::new();
        let parsed: ActionPatternId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn id_must_have_prefix() {
        let res: Result<ActionPatternId, _> = "not-action".parse();
        assert!(res.is_err());
    }

    #[test]
    fn validate_requires_sequence() {
        let bad = NewActionPattern {
            pattern_id: "p:1".into(),
            action_sequence: vec![],
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn from_new_sets_defaults() {
        let ap = ActionPattern::from_new(NewActionPattern {
            pattern_id: "p:1".into(),
            action_sequence: vec![step("git status", None), step("git add", Some("bash"))],
        })
        .unwrap();
        assert_eq!(ap.success_count, 0);
        assert_eq!(ap.failure_count, 0);
        assert!(ap.last_executed.is_none());
        assert!(ap.success_rate().is_none());
    }

    #[test]
    fn flatten_sequence_joins() {
        let ap = ActionPattern::from_new(NewActionPattern {
            pattern_id: "p:1".into(),
            action_sequence: vec![step("a", None), step("b", Some("tool"))],
        })
        .unwrap();
        assert_eq!(ap.flatten_sequence(), "a → b [tool]");
    }
}
