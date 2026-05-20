//! 4 MCP 도구 — match / suggest / confirm / record.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::pattern::{ActionPattern, ActionPatternId, ActionStep, NewActionPattern};
use crate::store::ActionPatternStore;
use crate::PatternsError;

/// 핸들러.
pub struct PatternTools<'a> {
    conn: &'a Connection,
}

impl<'a> PatternTools<'a> {
    /// 신규.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// `match_action_pattern` — 유사 패턴 top-K (현재는 LIKE; 임베딩 KNN은 후속).
    pub fn match_pattern(
        &self,
        new_action: &str,
        k: usize,
        _min_similarity: f64,
    ) -> Result<MatchResult, PatternsError> {
        let store = ActionPatternStore::new(self.conn);
        let hits = store.search_like(new_action, k as i64)?;
        Ok(MatchResult {
            input: new_action.to_string(),
            count: hits.len(),
            patterns: hits.into_iter().map(PatternSummary::from).collect(),
        })
    }

    /// `suggest_next_steps` — current_state로 매칭 → 다음 단계 추천.
    pub fn suggest_next(&self, current_state: &str) -> Result<Vec<SuggestedStep>, PatternsError> {
        let store = ActionPatternStore::new(self.conn);
        let hits = store.search_like(current_state, 3)?;
        let mut out = Vec::new();
        for ap in hits {
            // current_state와 매칭되는 step 다음 단계들 추천.
            for (idx, step) in ap.action_sequence.iter().enumerate() {
                if step.step.contains(current_state) {
                    // 다음 단계가 있으면.
                    if let Some(next) = ap.action_sequence.get(idx + 1) {
                        out.push(SuggestedStep {
                            pattern_id: ap.id.to_string(),
                            success_rate: ap.success_rate(),
                            step: next.step.clone(),
                            tool: next.tool.clone(),
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    /// `confirm_pattern_execution` — modifications 적용 후 실행 plan 반환 (실행 자체는 외부).
    pub fn confirm(
        &self,
        pattern_id: &str,
        modifications: Option<Vec<ActionStep>>,
    ) -> Result<ConfirmResult, PatternsError> {
        let id = ActionPatternId::from_str(pattern_id)?;
        let store = ActionPatternStore::new(self.conn);
        let ap = store
            .get(&id)?
            .ok_or_else(|| PatternsError::NotFound(pattern_id.to_string()))?;
        let plan = modifications.unwrap_or_else(|| ap.action_sequence.clone());
        Ok(ConfirmResult {
            pattern_id: ap.id.to_string(),
            plan,
            success_rate: ap.success_rate(),
            avg_duration_ms: ap.avg_duration_ms,
        })
    }

    /// `record_pattern_outcome` — 결과 기록.
    pub fn record(
        &self,
        pattern_id: &str,
        success: bool,
        duration_ms: Option<i64>,
    ) -> Result<(), PatternsError> {
        let id = ActionPatternId::from_str(pattern_id)?;
        ActionPatternStore::new(self.conn).record_outcome(&id, success, duration_ms)?;
        Ok(())
    }

    /// 신규 패턴 등록 (MCP 도구는 아니지만 헬퍼).
    pub fn create(&self, input: NewActionPattern) -> Result<ActionPattern, PatternsError> {
        let ap = ActionPattern::from_new(input)?;
        ActionPatternStore::new(self.conn).insert(&ap)?;
        Ok(ap)
    }
}

/// `match_action_pattern` 응답.
#[derive(Debug, Serialize, Deserialize)]
pub struct MatchResult {
    /// 입력.
    pub input: String,
    /// 매칭 수.
    pub count: usize,
    /// 패턴들 (success_rate DESC, last_executed DESC).
    pub patterns: Vec<PatternSummary>,
}

/// 패턴 요약.
#[derive(Debug, Serialize, Deserialize)]
pub struct PatternSummary {
    /// `action:<uuid>`.
    pub id: String,
    /// L3 patterns(0004) id 참조.
    pub pattern_id: String,
    /// 시퀀스 flatten ("step1 → step2 [tool] → ...").
    pub sequence: String,
    /// 0.0~1.0 또는 None (호출 0).
    pub success_rate: Option<f64>,
    /// 평균 ms.
    pub avg_duration_ms: Option<i64>,
}

impl From<ActionPattern> for PatternSummary {
    fn from(ap: ActionPattern) -> Self {
        let success_rate = ap.success_rate();
        let avg_duration_ms = ap.avg_duration_ms;
        let sequence = ap.flatten_sequence();
        Self {
            id: ap.id.to_string(),
            pattern_id: ap.pattern_id,
            sequence,
            success_rate,
            avg_duration_ms,
        }
    }
}

/// `suggest_next_steps` 응답 항목.
#[derive(Debug, Serialize, Deserialize)]
pub struct SuggestedStep {
    /// 출처 패턴.
    pub pattern_id: String,
    /// 0.0~1.0.
    pub success_rate: Option<f64>,
    /// 다음 단계 설명.
    pub step: String,
    /// 도구 (선택).
    pub tool: Option<String>,
}

/// `confirm_pattern_execution` 응답.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfirmResult {
    /// id.
    pub pattern_id: String,
    /// 적용된 plan (modifications가 없으면 원본 시퀀스).
    pub plan: Vec<ActionStep>,
    /// 성공률.
    pub success_rate: Option<f64>,
    /// 평균 ms.
    pub avg_duration_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{ActionStep, NewActionPattern};

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(include_str!(
            "../../openxgram-db/migrations/0004_patterns.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!(
            "../../openxgram-db/migrations/0020_action_patterns.sql"
        ))
        .unwrap();
        conn
    }

    fn seed_pattern(conn: &Connection) {
        let store = ActionPatternStore::new(conn);
        store.ensure_pattern("p:deploy", "deploy workflow").unwrap();
    }

    fn step(s: &str, t: Option<&str>) -> ActionStep {
        ActionStep {
            step: s.into(),
            tool: t.map(String::from),
            args: None,
        }
    }

    #[test]
    fn create_then_match() {
        let conn = fresh();
        seed_pattern(&conn);
        let tools = PatternTools::new(&conn);

        tools
            .create(NewActionPattern {
                pattern_id: "p:deploy".into(),
                action_sequence: vec![
                    step("git tag", Some("bash")),
                    step("push tag", Some("bash")),
                    step("deploy", None),
                ],
            })
            .unwrap();

        let r = tools.match_pattern("git tag", 5, 0.0).unwrap();
        assert_eq!(r.count, 1);
        assert!(r.patterns[0].sequence.contains("git tag"));
    }

    #[test]
    fn suggest_next_returns_following_step() {
        let conn = fresh();
        seed_pattern(&conn);
        let tools = PatternTools::new(&conn);
        tools
            .create(NewActionPattern {
                pattern_id: "p:deploy".into(),
                action_sequence: vec![
                    step("lint", None),
                    step("build", None),
                    step("deploy", None),
                ],
            })
            .unwrap();

        let suggestions = tools.suggest_next("lint").unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].step, "build");
    }

    #[test]
    fn record_outcome_updates() {
        let conn = fresh();
        seed_pattern(&conn);
        let tools = PatternTools::new(&conn);
        let ap = tools
            .create(NewActionPattern {
                pattern_id: "p:deploy".into(),
                action_sequence: vec![step("x", None)],
            })
            .unwrap();

        tools.record(&ap.id.to_string(), true, Some(500)).unwrap();
        tools.record(&ap.id.to_string(), false, None).unwrap();

        let r = tools.match_pattern("x", 5, 0.0).unwrap();
        let p = &r.patterns[0];
        assert!((p.success_rate.unwrap() - 0.5).abs() < 0.001);
        assert_eq!(p.avg_duration_ms, Some(500));
    }

    #[test]
    fn confirm_returns_plan() {
        let conn = fresh();
        seed_pattern(&conn);
        let tools = PatternTools::new(&conn);
        let ap = tools
            .create(NewActionPattern {
                pattern_id: "p:deploy".into(),
                action_sequence: vec![step("original", None)],
            })
            .unwrap();

        let modified = vec![step("modified", Some("tool"))];
        let res = tools
            .confirm(&ap.id.to_string(), Some(modified.clone()))
            .unwrap();
        assert_eq!(res.plan, modified);
    }
}
