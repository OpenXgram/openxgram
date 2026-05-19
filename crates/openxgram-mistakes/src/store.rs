//! DB CRUD + LIKE 검색. 벡터 검색은 후속 (sqlite-vec 통합).

use crate::mistake::{Mistake, MistakeId};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::str::FromStr;
use thiserror::Error;

/// 저장소 에러.
#[derive(Debug, Error)]
pub enum MistakeStoreError {
    /// rusqlite.
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),

    /// 도메인.
    #[error("domain: {0}")]
    Domain(String),
}

/// SQLite 위에 얹은 실수 저장소.
pub struct MistakeStore<'a> {
    conn: &'a Connection,
}

impl<'a> MistakeStore<'a> {
    /// 신규.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// INSERT — 신규만. 이미 같은 ID 있으면 에러.
    pub fn insert(&self, m: &Mistake) -> Result<(), MistakeStoreError> {
        self.conn.execute(
            "INSERT INTO mistakes (
                id, session_id, occurred_at,
                intended_action, actual_outcome, failure_reason, lesson,
                severity, resolved, resolution, related_wiki,
                embedding_hash, created_at, updated_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                m.id.as_str(),
                m.session_id,
                m.occurred_at,
                m.intended_action,
                m.actual_outcome,
                m.failure_reason,
                m.lesson,
                m.severity as i64,
                m.resolved as i64,
                m.resolution,
                m.related_wiki,
                m.embedding_hash,
                m.created_at,
                m.updated_at,
            ],
        )?;
        Ok(())
    }

    /// ID로 조회.
    pub fn get(&self, id: &MistakeId) -> Result<Option<Mistake>, MistakeStoreError> {
        let mut stmt = self.conn.prepare(SELECT_ALL_WHERE_ID)?;
        let row = stmt
            .query_row(params![id.as_str()], row_to_mistake)
            .optional()?;
        Ok(row)
    }

    /// 미해결 + 심각도 ≥ threshold (severity DESC, occurred_at DESC) — 최신 limit개.
    pub fn list_unresolved(
        &self,
        min_severity: u8,
        limit: i64,
    ) -> Result<Vec<Mistake>, MistakeStoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, occurred_at, intended_action, actual_outcome,
                    failure_reason, lesson, severity, resolved, resolution,
                    related_wiki, embedding_hash, created_at, updated_at
             FROM mistakes
             WHERE resolved = 0 AND severity >= ?1
             ORDER BY severity DESC, occurred_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![min_severity as i64, limit], row_to_mistake)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 텍스트 LIKE 검색 — intended/outcome/reason/lesson 어디든 매칭.
    ///
    /// 벡터 KNN은 후속 작업 (sqlite-vec 통합 시 search.rs로 분리).
    pub fn search_like(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<Mistake>, MistakeStoreError> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let pattern = format!("%{}%", query.trim());
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, occurred_at, intended_action, actual_outcome,
                    failure_reason, lesson, severity, resolved, resolution,
                    related_wiki, embedding_hash, created_at, updated_at
             FROM mistakes
             WHERE intended_action LIKE ?1
                OR actual_outcome  LIKE ?1
                OR failure_reason  LIKE ?1
                OR lesson          LIKE ?1
             ORDER BY severity DESC, occurred_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], row_to_mistake)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// `resolved=1` + resolution 저장.
    pub fn mark_resolved(
        &self,
        id: &MistakeId,
        resolution: &str,
    ) -> Result<(), MistakeStoreError> {
        if resolution.trim().is_empty() {
            return Err(MistakeStoreError::Domain("resolution required".into()));
        }
        let now = Utc::now().timestamp_millis();
        let updated = self.conn.execute(
            "UPDATE mistakes SET resolved = 1, resolution = ?2, updated_at = ?3
              WHERE id = ?1",
            params![id.as_str(), resolution, now],
        )?;
        if updated == 0 {
            return Err(MistakeStoreError::Domain(format!(
                "mistake not found: {}",
                id
            )));
        }
        Ok(())
    }

    /// 전체 카운트.
    pub fn count(&self) -> Result<i64, MistakeStoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM mistakes", [], |r| r.get(0))?;
        Ok(n)
    }
}

const SELECT_ALL_WHERE_ID: &str = "
    SELECT id, session_id, occurred_at, intended_action, actual_outcome,
           failure_reason, lesson, severity, resolved, resolution,
           related_wiki, embedding_hash, created_at, updated_at
      FROM mistakes WHERE id = ?1
";

fn row_to_mistake(r: &rusqlite::Row<'_>) -> rusqlite::Result<Mistake> {
    let id_s: String = r.get("id")?;
    let id = MistakeId::from_str(&id_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())),
        )
    })?;
    Ok(Mistake {
        id,
        session_id: r.get("session_id")?,
        occurred_at: r.get("occurred_at")?,
        intended_action: r.get("intended_action")?,
        actual_outcome: r.get("actual_outcome")?,
        failure_reason: r.get("failure_reason")?,
        lesson: r.get("lesson")?,
        severity: {
            let n: i64 = r.get("severity")?;
            n as u8
        },
        resolved: {
            let n: i64 = r.get("resolved")?;
            n != 0
        },
        resolution: r.get("resolution")?,
        related_wiki: r.get("related_wiki")?,
        embedding_hash: r.get("embedding_hash")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mistake::{Mistake, NewMistake};

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../openxgram-db/migrations/0019_mistakes.sql"
        ))
        .unwrap();
        conn
    }

    fn sample(intended: &str, reason: &str, severity: Option<u8>) -> Mistake {
        Mistake::from_new(NewMistake {
            session_id: "session:test".into(),
            intended_action: intended.into(),
            actual_outcome: "rollback".into(),
            failure_reason: reason.into(),
            lesson: "다음엔 lint 먼저".into(),
            severity,
            related_wiki: None,
        })
        .unwrap()
    }

    #[test]
    fn insert_and_get() {
        let conn = fresh();
        let store = MistakeStore::new(&conn);
        let m = sample("deploy production", "missing .env", Some(7));
        store.insert(&m).unwrap();

        let got = store.get(&m.id).unwrap().expect("found");
        assert_eq!(got.intended_action, "deploy production");
        assert_eq!(got.severity, 7);
        assert!(!got.resolved);
    }

    #[test]
    fn list_unresolved_filters_severity() {
        let conn = fresh();
        let store = MistakeStore::new(&conn);
        store
            .insert(&sample("low", "minor typo", Some(2)))
            .unwrap();
        store
            .insert(&sample("high", "data loss", Some(9)))
            .unwrap();

        let list = store.list_unresolved(5, 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].intended_action, "high");
    }

    #[test]
    fn search_like_matches_all_fields() {
        let conn = fresh();
        let store = MistakeStore::new(&conn);
        store
            .insert(&sample("git push origin main", "no review", Some(5)))
            .unwrap();
        let hits = store.search_like("git push", 5).unwrap();
        assert_eq!(hits.len(), 1);
        let hits2 = store.search_like("no review", 5).unwrap();
        assert_eq!(hits2.len(), 1);
        let none = store.search_like("nothing matches", 5).unwrap();
        assert_eq!(none.len(), 0);
    }

    #[test]
    fn mark_resolved_updates_row() {
        let conn = fresh();
        let store = MistakeStore::new(&conn);
        let m = sample("x", "y", None);
        store.insert(&m).unwrap();

        store.mark_resolved(&m.id, "추가 lint 도입").unwrap();
        let got = store.get(&m.id).unwrap().unwrap();
        assert!(got.resolved);
        assert_eq!(got.resolution.as_deref(), Some("추가 lint 도입"));
    }

    #[test]
    fn mark_resolved_unknown_id_errors() {
        let conn = fresh();
        let store = MistakeStore::new(&conn);
        let res = store.mark_resolved(&MistakeId::new(), "x");
        assert!(res.is_err());
    }

    #[test]
    fn empty_resolution_rejected() {
        let conn = fresh();
        let store = MistakeStore::new(&conn);
        let m = sample("x", "y", None);
        store.insert(&m).unwrap();
        assert!(store.mark_resolved(&m.id, "").is_err());
    }
}
