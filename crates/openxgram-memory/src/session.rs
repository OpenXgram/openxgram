//! Sessions (대화 컨테이너) — message·memory·episode 의 부모.
//!
//! Phase 1: create / list / get_by_id. metadata·participants 편집은 후속.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::parse_ts;
use crate::{MemoryError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<FixedOffset>,
    pub last_active: DateTime<FixedOffset>,
    pub home_machine: String,
}

pub struct SessionStore<'a> {
    db: &'a mut Db,
}

impl<'a> SessionStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    pub fn create(&mut self, title: &str, home_machine: &str) -> Result<Session> {
        let id = Uuid::new_v4().to_string();
        let now = kst_now();
        let now_rfc = now.to_rfc3339();
        let affected = self.db.conn().execute(
            "INSERT INTO sessions (id, title, created_at, last_active, home_machine)
             VALUES (?1, ?2, ?3, ?3, ?4)",
            rusqlite::params![id, title, now_rfc, home_machine],
        )?;
        if affected != 1 {
            return Err(MemoryError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }
        Ok(Session {
            id,
            title: title.into(),
            created_at: now,
            last_active: now,
            home_machine: home_machine.into(),
        })
    }

    /// title 로 session 찾고 없으면 생성. inbound peer 별 inbox session 자동 생성에 활용.
    pub fn ensure_by_title(&mut self, title: &str, home_machine: &str) -> Result<Session> {
        let existing: Option<(String, String, String, String)> = self
            .db
            .conn()
            .query_row(
                "SELECT id, created_at, last_active, home_machine FROM sessions WHERE title = ?1",
                [title],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok::<_, MemoryError>(None),
                other => Err(other.into()),
            })?;
        if let Some((id, created, active, hm)) = existing {
            return Ok(Session {
                id,
                title: title.into(),
                created_at: parse_ts(&created)?,
                last_active: parse_ts(&active)?,
                home_machine: hm,
            });
        }
        self.create(title, home_machine)
    }

    pub fn list(&mut self) -> Result<Vec<Session>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, title, created_at, last_active, home_machine
             FROM sessions ORDER BY last_active DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, title, created, last, home) = row?;
            out.push(Session {
                id,
                title,
                created_at: parse_ts(&created)?,
                last_active: parse_ts(&last)?,
                home_machine: home,
            });
        }
        Ok(out)
    }

    /// session 삭제 — FK CASCADE 로 messages/episodes 동시 삭제. memories 는 SET NULL.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        let affected = self
            .db
            .conn()
            .execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        if affected != 1 {
            return Err(MemoryError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }
        Ok(())
    }

    pub fn get_by_id(&mut self, id: &str) -> Result<Option<Session>> {
        let result = self.db.conn().query_row(
            "SELECT id, title, created_at, last_active, home_machine
             FROM sessions WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        );
        match result {
            Ok((id, title, created, last, home)) => Ok(Some(Session {
                id,
                title,
                created_at: parse_ts(&created)?,
                last_active: parse_ts(&last)?,
                home_machine: home,
            })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
