//! 도메인 객체 — `Mistake` + ID 타입.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// 실수 ID — `mistake:<uuid>` 형태.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MistakeId(pub String);

impl MistakeId {
    /// 신규 UUIDv4 기반 ID.
    pub fn new() -> Self {
        Self(format!("mistake:{}", Uuid::new_v4()))
    }

    /// 내부 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MistakeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for MistakeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for MistakeId {
    type Err = crate::MistakesError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("mistake:") {
            return Err(crate::MistakesError::Invalid(format!(
                "mistake id must start with 'mistake:' — got {}",
                s
            )));
        }
        Ok(MistakeId(s.to_string()))
    }
}

/// 실수 등록 시 입력.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMistake {
    /// 세션 ID — `session:<uuid>` 또는 호출자 식별.
    pub session_id: String,
    /// 하려던 것.
    pub intended_action: String,
    /// 실제로 일어난 일.
    pub actual_outcome: String,
    /// 왜 실패했는가.
    pub failure_reason: String,
    /// 다음에 어떻게 다르게 할지.
    pub lesson: String,
    /// 1~10 (default 5).
    pub severity: Option<u8>,
    /// 관련 위키 페이지 (선택).
    pub related_wiki: Option<String>,
}

impl NewMistake {
    /// 입력 검증 — 필수 필드 + severity 범위.
    pub fn validate(&self) -> Result<(), crate::MistakesError> {
        if self.session_id.trim().is_empty() {
            return Err(crate::MistakesError::Invalid("session_id required".into()));
        }
        if self.intended_action.trim().is_empty() {
            return Err(crate::MistakesError::Invalid(
                "intended_action required".into(),
            ));
        }
        if self.actual_outcome.trim().is_empty() {
            return Err(crate::MistakesError::Invalid(
                "actual_outcome required".into(),
            ));
        }
        if self.failure_reason.trim().is_empty() {
            return Err(crate::MistakesError::Invalid(
                "failure_reason required".into(),
            ));
        }
        if self.lesson.trim().is_empty() {
            return Err(crate::MistakesError::Invalid("lesson required".into()));
        }
        if let Some(s) = self.severity {
            if !(1..=10).contains(&s) {
                return Err(crate::MistakesError::Invalid(format!(
                    "severity must be 1..=10 — got {}",
                    s
                )));
            }
        }
        Ok(())
    }
}

/// 영속 실수 객체.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mistake {
    /// `mistake:<uuid>`.
    pub id: MistakeId,
    /// 세션.
    pub session_id: String,
    /// 발생 시각 (epoch ms).
    pub occurred_at: i64,
    /// 의도.
    pub intended_action: String,
    /// 결과.
    pub actual_outcome: String,
    /// 원인.
    pub failure_reason: String,
    /// 교훈.
    pub lesson: String,
    /// 1~10.
    pub severity: u8,
    /// 해결됨?
    pub resolved: bool,
    /// 해결 내용 (resolved=true 시).
    pub resolution: Option<String>,
    /// 관련 위키.
    pub related_wiki: Option<String>,
    /// 임베딩 해시 (텍스트 SHA-256).
    pub embedding_hash: String,
    /// 생성 시각 (epoch ms).
    pub created_at: i64,
    /// 갱신 시각 (epoch ms).
    pub updated_at: i64,
}

impl Mistake {
    /// `NewMistake` → 영속 `Mistake` (id, occurred_at, hash, timestamps 채움).
    pub fn from_new(input: NewMistake) -> Result<Self, crate::MistakesError> {
        input.validate()?;
        let now = Utc::now().timestamp_millis();
        let hash = compute_embedding_hash(&input.intended_action, &input.failure_reason);
        Ok(Self {
            id: MistakeId::new(),
            session_id: input.session_id,
            occurred_at: now,
            intended_action: input.intended_action,
            actual_outcome: input.actual_outcome,
            failure_reason: input.failure_reason,
            lesson: input.lesson,
            severity: input.severity.unwrap_or(5),
            resolved: false,
            resolution: None,
            related_wiki: input.related_wiki,
            embedding_hash: hash,
            created_at: now,
            updated_at: now,
        })
    }

    /// 임베딩용 결합 텍스트.
    pub fn embedding_input(&self) -> String {
        format!("{}\n\n{}", self.intended_action, self.failure_reason)
    }
}

/// SHA-256 hex — 임베딩 재생성 트리거.
pub fn compute_embedding_hash(intended: &str, reason: &str) -> String {
    let mut h = Sha256::new();
    h.update(intended.as_bytes());
    h.update(b"\n");
    h.update(reason.as_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mistake_id_roundtrip() {
        let id = MistakeId::new();
        let s = id.to_string();
        assert!(s.starts_with("mistake:"));
        let parsed: MistakeId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn invalid_id_prefix_rejected() {
        let res: Result<MistakeId, _> = "not-a-mistake".parse();
        assert!(res.is_err());
    }

    #[test]
    fn from_new_validates_required_fields() {
        let bad = NewMistake {
            session_id: "".into(),
            intended_action: "x".into(),
            actual_outcome: "x".into(),
            failure_reason: "x".into(),
            lesson: "x".into(),
            severity: None,
            related_wiki: None,
        };
        assert!(Mistake::from_new(bad).is_err());
    }

    #[test]
    fn severity_default_5() {
        let m = Mistake::from_new(NewMistake {
            session_id: "session:abc".into(),
            intended_action: "deploy".into(),
            actual_outcome: "rollback".into(),
            failure_reason: ".env missing".into(),
            lesson: "lint before deploy".into(),
            severity: None,
            related_wiki: None,
        })
        .unwrap();
        assert_eq!(m.severity, 5);
        assert!(!m.resolved);
    }

    #[test]
    fn severity_out_of_range_rejected() {
        let res = Mistake::from_new(NewMistake {
            session_id: "session:abc".into(),
            intended_action: "x".into(),
            actual_outcome: "x".into(),
            failure_reason: "x".into(),
            lesson: "x".into(),
            severity: Some(11),
            related_wiki: None,
        });
        assert!(res.is_err());
    }

    #[test]
    fn embedding_hash_deterministic() {
        let h1 = compute_embedding_hash("a", "b");
        let h2 = compute_embedding_hash("a", "b");
        assert_eq!(h1, h2);
        assert_ne!(h1, compute_embedding_hash("a", "c"));
    }
}
