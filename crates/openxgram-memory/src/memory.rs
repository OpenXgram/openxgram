//! L2 memories — 사실·결정·reference·rule 저장.
//!
//! Phase 1 first PR: insert / list_by_kind / pin / unpin / mark_accessed.
//! sqlite-vec 임베딩 통합·NEW/RECURRING/ROUTINE 분류는 후속 PR.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::parse_ts;
use crate::{MemoryError, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Fact,
    Decision,
    Reference,
    Rule,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Decision => "decision",
            Self::Reference => "reference",
            Self::Rule => "rule",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "fact" => Self::Fact,
            "decision" => Self::Decision,
            "reference" => Self::Reference,
            "rule" => Self::Rule,
            other => return Err(MemoryError::InvalidKind(other.into())),
        })
    }
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: String,
    pub session_id: Option<String>,
    pub kind: MemoryKind,
    pub content: String,
    pub pinned: bool,
    pub importance: f64,
    pub access_count: i64,
    pub created_at: DateTime<FixedOffset>,
    pub last_accessed: DateTime<FixedOffset>,
}

pub struct MemoryStore<'a> {
    db: &'a mut Db,
}

impl<'a> MemoryStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// L2 memory insert. importance 기본 0.5, pinned 기본 false.
    pub fn insert(
        &mut self,
        session_id: Option<&str>,
        kind: MemoryKind,
        content: &str,
    ) -> Result<Memory> {
        let id = Uuid::new_v4().to_string();
        let now = kst_now();
        let now_rfc = now.to_rfc3339();

        let affected = self.db.conn().execute(
            "INSERT INTO memories
                 (id, session_id, kind, content, pinned, importance,
                  access_count, created_at, last_accessed)
             VALUES (?1, ?2, ?3, ?4, 0, 0.5, 0, ?5, ?5)",
            rusqlite::params![id, session_id, kind.as_str(), content, now_rfc],
        )?;
        if affected != 1 {
            return Err(MemoryError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }

        Ok(Memory {
            id,
            session_id: session_id.map(str::to_string),
            kind,
            content: content.into(),
            pinned: false,
            importance: 0.5,
            access_count: 0,
            created_at: now,
            last_accessed: now,
        })
    }

    pub fn list_for_session(&mut self, session_id: &str) -> Result<Vec<Memory>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, session_id, kind, content, pinned, importance,
                    access_count, created_at, last_accessed
             FROM memories WHERE session_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([session_id], row_to_memory)?;
        rows.collect::<rusqlite::Result<Vec<RawMemory>>>()
            .map_err(MemoryError::from)?
            .into_iter()
            .map(raw_to_memory)
            .collect()
    }

    pub fn list_by_kind(&mut self, kind: MemoryKind) -> Result<Vec<Memory>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, session_id, kind, content, pinned, importance,
                    access_count, created_at, last_accessed
             FROM memories WHERE kind = ?1 ORDER BY pinned DESC, last_accessed DESC",
        )?;
        let rows = stmt.query_map([kind.as_str()], row_to_memory)?;
        rows.collect::<rusqlite::Result<Vec<RawMemory>>>()
            .map_err(MemoryError::from)?
            .into_iter()
            .map(raw_to_memory)
            .collect()
    }

    /// pin/unpin. UPDATE 가 0건이면 NotFound 가 아닌 UnexpectedRowCount raise (silent error 방지).
    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<()> {
        let affected = self.db.conn().execute(
            "UPDATE memories SET pinned = ?1 WHERE id = ?2",
            rusqlite::params![pinned as i64, id],
        )?;
        if affected != 1 {
            return Err(MemoryError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }
        Ok(())
    }

    /// 접근 카운트 증가 + last_accessed 갱신.
    pub fn mark_accessed(&mut self, id: &str) -> Result<()> {
        let now_rfc = kst_now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1
             WHERE id = ?2",
            rusqlite::params![now_rfc, id],
        )?;
        if affected != 1 {
            return Err(MemoryError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }
        Ok(())
    }
}

struct RawMemory {
    id: String,
    session_id: Option<String>,
    kind: String,
    content: String,
    pinned: i64,
    importance: f64,
    access_count: i64,
    created_at: String,
    last_accessed: String,
}

fn row_to_memory(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawMemory> {
    Ok(RawMemory {
        id: r.get(0)?,
        session_id: r.get(1)?,
        kind: r.get(2)?,
        content: r.get(3)?,
        pinned: r.get(4)?,
        importance: r.get(5)?,
        access_count: r.get(6)?,
        created_at: r.get(7)?,
        last_accessed: r.get(8)?,
    })
}

fn raw_to_memory(raw: RawMemory) -> Result<Memory> {
    Ok(Memory {
        id: raw.id,
        session_id: raw.session_id,
        kind: MemoryKind::parse(&raw.kind)?,
        content: raw.content,
        pinned: raw.pinned != 0,
        importance: raw.importance,
        access_count: raw.access_count,
        created_at: parse_ts(&raw.created_at)?,
        last_accessed: parse_ts(&raw.last_accessed)?,
    })
}
