//! L3 patterns — 반복 행동/발화 분류 (NEW / RECURRING / ROUTINE).
//!
//! Phase 1: 빈도 기반 분류기 + observe (upsert). 시간 간격 기반 ROUTINE
//! 임계값 조정·embedder 클러스터링은 후속.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::parse_ts;
use crate::Result;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    New,
    Recurring,
    Routine,
}

impl Classification {
    /// PRD §7 임계값: 1=NEW, 2~4=RECURRING, 5+=ROUTINE.
    pub fn from_frequency(freq: i64) -> Self {
        match freq {
            ..=1 => Self::New,
            2..=4 => Self::Recurring,
            _ => Self::Routine,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Recurring => "recurring",
            Self::Routine => "routine",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pattern {
    pub id: String,
    pub pattern_text: String,
    pub frequency: i64,
    pub first_seen: DateTime<FixedOffset>,
    pub last_seen: DateTime<FixedOffset>,
    pub classification: Classification,
}

pub struct PatternStore<'a> {
    db: &'a mut Db,
}

impl<'a> PatternStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// upsert: 존재하면 frequency+1 + last_seen 갱신, 없으면 새로 insert.
    /// 분류는 갱신된 frequency 로 derive 후 반환.
    pub fn observe(&mut self, pattern_text: &str) -> Result<Pattern> {
        let now = kst_now();
        let now_rfc = now.to_rfc3339();
        let id = Uuid::new_v4().to_string();

        let conn = self.db.conn();
        // INSERT ... ON CONFLICT 으로 upsert
        conn.execute(
            "INSERT INTO patterns (id, pattern_text, frequency, first_seen, last_seen)
             VALUES (?1, ?2, 1, ?3, ?3)
             ON CONFLICT(pattern_text) DO UPDATE SET
                 frequency = frequency + 1,
                 last_seen = ?3",
            rusqlite::params![id, pattern_text, now_rfc],
        )?;

        // 갱신된 row 조회
        let (id, pattern_text, frequency, first_seen, last_seen): (
            String,
            String,
            i64,
            String,
            String,
        ) = conn.query_row(
            "SELECT id, pattern_text, frequency, first_seen, last_seen
             FROM patterns WHERE pattern_text = ?1",
            [pattern_text],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;

        Ok(Pattern {
            id,
            pattern_text,
            frequency,
            first_seen: parse_ts(&first_seen)?,
            last_seen: parse_ts(&last_seen)?,
            classification: Classification::from_frequency(frequency),
        })
    }

    pub fn list_by_classification(&mut self, c: Classification) -> Result<Vec<Pattern>> {
        let (lo, hi) = match c {
            Classification::New => (i64::MIN, 1),
            Classification::Recurring => (2, 4),
            Classification::Routine => (5, i64::MAX),
        };
        let mut stmt = self.db.conn().prepare(
            "SELECT id, pattern_text, frequency, first_seen, last_seen
             FROM patterns WHERE frequency BETWEEN ?1 AND ?2
             ORDER BY frequency DESC, last_seen DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![lo, hi], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, pattern_text, frequency, first_seen, last_seen) = row?;
            out.push(Pattern {
                id,
                pattern_text,
                frequency,
                first_seen: parse_ts(&first_seen)?,
                last_seen: parse_ts(&last_seen)?,
                classification: Classification::from_frequency(frequency),
            });
        }
        Ok(out)
    }
}
