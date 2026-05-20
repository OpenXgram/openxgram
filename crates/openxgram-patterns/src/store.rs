//! DB CRUD + LIKE 검색 + 결과 누적 (success/failure + avg_duration).

use crate::pattern::{ActionPattern, ActionPatternId, ActionStep};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::str::FromStr;
use thiserror::Error;

/// 저장소 에러.
#[derive(Debug, Error)]
pub enum ActionPatternStoreError {
    /// rusqlite.
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),

    /// JSON.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// 도메인.
    #[error("domain: {0}")]
    Domain(String),
}

/// FK 위해 patterns(0004) row가 필요. 테스트용으로 inline 생성 헬퍼 제공.
pub struct ActionPatternStore<'a> {
    conn: &'a Connection,
}

impl<'a> ActionPatternStore<'a> {
    /// 신규.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// INSERT — pattern_id가 존재해야 함 (FK).
    pub fn insert(&self, ap: &ActionPattern) -> Result<(), ActionPatternStoreError> {
        let seq_json = serde_json::to_string(&ap.action_sequence)?;
        self.conn.execute(
            "INSERT INTO action_patterns (
                id, pattern_id, action_sequence,
                avg_duration_ms, success_count, failure_count, last_executed,
                embedding_hash, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                ap.id.as_str(),
                ap.pattern_id,
                seq_json,
                ap.avg_duration_ms,
                ap.success_count,
                ap.failure_count,
                ap.last_executed,
                ap.embedding_hash,
                ap.created_at,
                ap.updated_at,
            ],
        )?;
        Ok(())
    }

    /// ID 조회.
    pub fn get(
        &self,
        id: &ActionPatternId,
    ) -> Result<Option<ActionPattern>, ActionPatternStoreError> {
        let mut stmt = self.conn.prepare(SELECT_ALL)?;
        Ok(stmt
            .query_row(params![id.as_str()], row_to_action)
            .optional()?)
    }

    /// LIKE 검색 (action_sequence JSON 또는 flatten).
    pub fn search_like(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ActionPattern>, ActionPatternStoreError> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let pattern = format!("%{}%", query.trim());
        let mut stmt = self.conn.prepare(
            "SELECT id, pattern_id, action_sequence,
                    avg_duration_ms, success_count, failure_count, last_executed,
                    embedding_hash, created_at, updated_at
             FROM action_patterns
             WHERE action_sequence LIKE ?1
             ORDER BY (success_count - failure_count) DESC,
                      last_executed DESC NULLS LAST
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], row_to_action)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 결과 기록 — success/failure 카운트 + duration 누적평균.
    pub fn record_outcome(
        &self,
        id: &ActionPatternId,
        success: bool,
        duration_ms: Option<i64>,
    ) -> Result<(), ActionPatternStoreError> {
        let now = Utc::now().timestamp_millis();
        // 기존 row 가져와서 avg 계산.
        let cur = self
            .get(id)?
            .ok_or_else(|| ActionPatternStoreError::Domain(format!("pattern not found: {}", id)))?;

        let (new_success, new_failure) = if success {
            (cur.success_count + 1, cur.failure_count)
        } else {
            (cur.success_count, cur.failure_count + 1)
        };

        // 누적 평균 (성공 케이스만).
        let new_avg = if success {
            match (cur.avg_duration_ms, duration_ms) {
                (Some(prev), Some(d)) => Some(((prev * cur.success_count) + d) / new_success),
                (None, Some(d)) => Some(d),
                (prev, None) => prev,
            }
        } else {
            cur.avg_duration_ms
        };

        let updated = self.conn.execute(
            "UPDATE action_patterns
                SET success_count = ?2,
                    failure_count = ?3,
                    avg_duration_ms = ?4,
                    last_executed = ?5,
                    updated_at = ?5
              WHERE id = ?1",
            params![id.as_str(), new_success, new_failure, new_avg, now],
        )?;
        if updated == 0 {
            return Err(ActionPatternStoreError::Domain(format!(
                "pattern not found: {}",
                id
            )));
        }
        Ok(())
    }

    /// 전체 카운트.
    pub fn count(&self) -> Result<i64, ActionPatternStoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM action_patterns", [], |r| r.get(0))?;
        Ok(n)
    }

    /// 테스트 헬퍼 — patterns(0004) 테이블에 더미 row 등록.
    pub fn ensure_pattern(
        &self,
        pattern_id: &str,
        text: &str,
    ) -> Result<(), ActionPatternStoreError> {
        let now_iso = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO patterns (id, pattern_text, frequency, first_seen, last_seen, metadata)
             VALUES (?1, ?2, 1, ?3, ?3, '{}')",
            params![pattern_id, text, now_iso],
        )?;
        Ok(())
    }
}

const SELECT_ALL: &str = "
    SELECT id, pattern_id, action_sequence,
           avg_duration_ms, success_count, failure_count, last_executed,
           embedding_hash, created_at, updated_at
      FROM action_patterns WHERE id = ?1
";

fn row_to_action(r: &rusqlite::Row<'_>) -> rusqlite::Result<ActionPattern> {
    fn fail(msg: String) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, msg)),
        )
    }
    let id_s: String = r.get("id")?;
    let id = ActionPatternId::from_str(&id_s).map_err(|e| fail(e.to_string()))?;
    let seq_json: String = r.get("action_sequence")?;
    let action_sequence: Vec<ActionStep> =
        serde_json::from_str(&seq_json).map_err(|e| fail(e.to_string()))?;
    Ok(ActionPattern {
        id,
        pattern_id: r.get("pattern_id")?,
        action_sequence,
        avg_duration_ms: r.get("avg_duration_ms")?,
        success_count: r.get("success_count")?,
        failure_count: r.get("failure_count")?,
        last_executed: r.get("last_executed")?,
        embedding_hash: r.get("embedding_hash")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{ActionStep, NewActionPattern};

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        // 0004 + 0020 동시 적용.
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

    fn sample(pattern_id: &str) -> ActionPattern {
        ActionPattern::from_new(NewActionPattern {
            pattern_id: pattern_id.into(),
            action_sequence: vec![
                ActionStep {
                    step: "git status".into(),
                    tool: None,
                    args: None,
                },
                ActionStep {
                    step: "commit".into(),
                    tool: Some("bash".into()),
                    args: None,
                },
            ],
        })
        .unwrap()
    }

    #[test]
    fn insert_and_get() {
        let conn = fresh();
        let store = ActionPatternStore::new(&conn);
        store.ensure_pattern("p:git", "git workflow").unwrap();
        let ap = sample("p:git");
        store.insert(&ap).unwrap();
        let got = store.get(&ap.id).unwrap().unwrap();
        assert_eq!(got.action_sequence.len(), 2);
        assert_eq!(got.success_count, 0);
    }

    #[test]
    fn record_outcome_updates_counts_and_avg() {
        let conn = fresh();
        let store = ActionPatternStore::new(&conn);
        store.ensure_pattern("p:git", "git workflow").unwrap();
        let ap = sample("p:git");
        store.insert(&ap).unwrap();

        store.record_outcome(&ap.id, true, Some(1000)).unwrap();
        store.record_outcome(&ap.id, true, Some(2000)).unwrap();
        store.record_outcome(&ap.id, false, None).unwrap();

        let got = store.get(&ap.id).unwrap().unwrap();
        assert_eq!(got.success_count, 2);
        assert_eq!(got.failure_count, 1);
        assert_eq!(got.avg_duration_ms, Some(1500)); // (1000+2000)/2
        assert!(got.last_executed.is_some());
    }

    #[test]
    fn search_like_matches_sequence_json() {
        let conn = fresh();
        let store = ActionPatternStore::new(&conn);
        store.ensure_pattern("p:git", "git").unwrap();
        store.insert(&sample("p:git")).unwrap();
        let hits = store.search_like("git status", 5).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn record_outcome_unknown_id_errors() {
        let conn = fresh();
        let store = ActionPatternStore::new(&conn);
        store.ensure_pattern("p:x", "x").unwrap();
        let unknown = ActionPatternId::new();
        let res = store.record_outcome(&unknown, true, None);
        assert!(res.is_err());
    }
}
