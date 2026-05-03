//! L4 traits — 정체성·성향. PRD §6 L4.
//!
//! source: derived (야간 reflection 자동 도출) 또는 manual (마스터 편집).
//! Phase 1: insert_or_update / list / get_by_name. derived 자동 도출 알고리즘은
//! 후속 (PatternStore + 야간 reflection 통합).

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::parse_ts;
use crate::{MemoryError, Result};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TraitSource {
    Derived,
    Manual,
}

impl TraitSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Derived => "derived",
            Self::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "derived" => Self::Derived,
            "manual" => Self::Manual,
            other => return Err(MemoryError::InvalidKind(format!("trait source: {other}"))),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentTrait {
    pub id: String,
    pub name: String,
    pub value: String,
    pub source: TraitSource,
    pub source_refs: Vec<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

pub struct TraitStore<'a> {
    db: &'a mut Db,
}

impl<'a> TraitStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// 같은 name 이 있으면 value/source/source_refs/updated_at 갱신, 없으면 insert.
    pub fn insert_or_update(
        &mut self,
        name: &str,
        value: &str,
        source: TraitSource,
        source_refs: &[String],
    ) -> Result<AgentTrait> {
        let now = kst_now();
        let now_rfc = now.to_rfc3339();
        let id = Uuid::new_v4().to_string();
        let refs_json = serde_json::to_string(source_refs)
            .map_err(|e| MemoryError::InvalidKind(format!("source_refs serialize: {e}")))?;

        let conn = self.db.conn();
        conn.execute(
            "INSERT INTO traits (id, name, value, source, source_refs, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(name) DO UPDATE SET
                 value = ?3, source = ?4, source_refs = ?5, updated_at = ?6",
            rusqlite::params![id, name, value, source.as_str(), refs_json, now_rfc],
        )?;

        self.get_by_name(name)?
            .ok_or_else(|| MemoryError::InvalidKind(format!("upsert lost: {name}")))
    }

    pub fn get_by_name(&mut self, name: &str) -> Result<Option<AgentTrait>> {
        let result = self.db.conn().query_row(
            "SELECT id, name, value, source, source_refs, created_at, updated_at
             FROM traits WHERE name = ?1",
            [name],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        );
        match result {
            Ok((id, name, value, source, refs_json, created, updated)) => Ok(Some(AgentTrait {
                id,
                name,
                value,
                source: TraitSource::parse(&source)?,
                source_refs: serde_json::from_str(&refs_json).unwrap_or_default(),
                created_at: parse_ts(&created)?,
                updated_at: parse_ts(&updated)?,
            })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list(&mut self) -> Result<Vec<AgentTrait>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, name, value, source, source_refs, created_at, updated_at
             FROM traits ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, value, source, refs_json, created, updated) = row?;
            out.push(AgentTrait {
                id,
                name,
                value,
                source: TraitSource::parse(&source)?,
                source_refs: serde_json::from_str(&refs_json).unwrap_or_default(),
                created_at: parse_ts(&created)?,
                updated_at: parse_ts(&updated)?,
            });
        }
        Ok(out)
    }
}
