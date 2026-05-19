//! 4 MCP 도구 핸들러 — `check_for_mistakes` / `log_mistake` / `find_similar_failures` / `resolve_mistake`.
//!
//! 도메인 핸들러만 제공. JSON-RPC 어댑터는 openxgram-mcp 또는 openxgram-cli의 mcp_serve가 래핑.
//!
//! 벡터 KNN은 후속. 현재는 `MistakeStore::search_like` (LIKE 검색) 기반.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::mistake::{Mistake, MistakeId, NewMistake};
use crate::store::MistakeStore;
use crate::MistakesError;

/// 4 도구 핸들러.
pub struct MistakeTools<'a> {
    conn: &'a Connection,
}

impl<'a> MistakeTools<'a> {
    /// 신규.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// `check_for_mistakes` — planned_action으로 유사 과거 실수 top-K 조회 + 경고문 생성.
    pub fn check(&self, planned_action: &str, k: usize) -> Result<CheckResult, MistakesError> {
        let store = MistakeStore::new(self.conn);
        let hits = store.search_like(planned_action, k as i64)?;
        let warnings = hits.iter().map(make_warning).collect();
        Ok(CheckResult {
            planned_action: planned_action.to_string(),
            similar_count: hits.len(),
            hits: hits.into_iter().map(MistakeSummary::from).collect(),
            warnings,
        })
    }

    /// `log_mistake` — 등록.
    pub fn log(&self, input: NewMistake) -> Result<LogResult, MistakesError> {
        let m = Mistake::from_new(input)?;
        MistakeStore::new(self.conn).insert(&m)?;
        Ok(LogResult {
            id: m.id.to_string(),
            severity: m.severity,
            occurred_at: m.occurred_at,
        })
    }

    /// `find_similar_failures` — situation으로 유사 실수 검색.
    pub fn find_similar(
        &self,
        situation: &str,
        k: usize,
    ) -> Result<Vec<MistakeSummary>, MistakesError> {
        let store = MistakeStore::new(self.conn);
        let hits = store.search_like(situation, k as i64)?;
        Ok(hits.into_iter().map(MistakeSummary::from).collect())
    }

    /// `resolve_mistake` — 해결됨 표시.
    pub fn resolve(&self, mistake_id: &str, resolution: &str) -> Result<(), MistakesError> {
        let id = MistakeId::from_str(mistake_id)?;
        MistakeStore::new(self.conn).mark_resolved(&id, resolution)?;
        Ok(())
    }
}

/// `check_for_mistakes` 응답.
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckResult {
    /// 입력.
    pub planned_action: String,
    /// 매칭 수.
    pub similar_count: usize,
    /// 매칭된 실수들.
    pub hits: Vec<MistakeSummary>,
    /// LLM에 제시할 경고문 (각 hit당 한 줄).
    pub warnings: Vec<String>,
}

/// `log_mistake` 응답.
#[derive(Debug, Serialize, Deserialize)]
pub struct LogResult {
    /// 신규 mistake id.
    pub id: String,
    /// 부여된 severity.
    pub severity: u8,
    /// epoch ms.
    pub occurred_at: i64,
}

/// 요약 표현 — MCP 응답용.
#[derive(Debug, Serialize, Deserialize)]
pub struct MistakeSummary {
    /// id.
    pub id: String,
    /// 의도.
    pub intended_action: String,
    /// 원인.
    pub failure_reason: String,
    /// 교훈.
    pub lesson: String,
    /// 1~10.
    pub severity: u8,
    /// resolved 여부.
    pub resolved: bool,
    /// 발생 시각 (epoch ms).
    pub occurred_at: i64,
}

impl From<Mistake> for MistakeSummary {
    fn from(m: Mistake) -> Self {
        Self {
            id: m.id.to_string(),
            intended_action: m.intended_action,
            failure_reason: m.failure_reason,
            lesson: m.lesson,
            severity: m.severity,
            resolved: m.resolved,
            occurred_at: m.occurred_at,
        }
    }
}

fn make_warning(m: &Mistake) -> String {
    format!(
        "⚠ 비슷한 과거 실수 [{}] (severity {}{}): 의도='{}' → 실패='{}'. 교훈: {}",
        m.id,
        m.severity,
        if m.resolved { ", resolved" } else { "" },
        truncate(&m.intended_action, 60),
        truncate(&m.failure_reason, 60),
        truncate(&m.lesson, 80),
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../openxgram-db/migrations/0019_mistakes.sql"
        ))
        .unwrap();
        conn
    }

    #[test]
    fn log_then_check_finds_match() {
        let conn = fresh();
        let tools = MistakeTools::new(&conn);
        tools
            .log(NewMistake {
                session_id: "session:s1".into(),
                intended_action: "deploy without lint".into(),
                actual_outcome: "type error in prod".into(),
                failure_reason: ".env DATABASE_URL missing".into(),
                lesson: "lint + env check before deploy".into(),
                severity: Some(8),
                related_wiki: None,
            })
            .unwrap();

        let res = tools.check("deploy", 5).unwrap();
        assert_eq!(res.similar_count, 1);
        assert_eq!(res.hits[0].severity, 8);
        assert!(res.warnings[0].contains("deploy without lint"));
    }

    #[test]
    fn resolve_marks_done() {
        let conn = fresh();
        let tools = MistakeTools::new(&conn);
        let logged = tools
            .log(NewMistake {
                session_id: "session:s1".into(),
                intended_action: "x".into(),
                actual_outcome: "y".into(),
                failure_reason: "z".into(),
                lesson: "w".into(),
                severity: None,
                related_wiki: None,
            })
            .unwrap();

        tools.resolve(&logged.id, "fixed").unwrap();
        let hits = tools.find_similar("x", 5).unwrap();
        assert!(hits[0].resolved);
    }

    #[test]
    fn truncate_keeps_short_strings() {
        assert_eq!(truncate("short", 60), "short");
        assert!(truncate("a".repeat(100).as_str(), 60).ends_with('…'));
    }
}
