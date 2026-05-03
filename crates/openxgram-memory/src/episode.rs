//! L1 episodes — session reflection 결과.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::parse_ts;
use crate::{MemoryError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Episode {
    pub id: String,
    pub session_id: String,
    pub started_at: DateTime<FixedOffset>,
    pub ended_at: DateTime<FixedOffset>,
    pub message_count: i64,
    pub summary: String,
    pub created_at: DateTime<FixedOffset>,
}

pub struct EpisodeStore<'a> {
    db: &'a mut Db,
}

impl<'a> EpisodeStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    pub fn list_for_session(&mut self, session_id: &str) -> Result<Vec<Episode>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, session_id, started_at, ended_at, message_count, summary, created_at
             FROM episodes WHERE session_id = ?1 ORDER BY started_at",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, session_id, started, ended, count, summary, created) = row?;
            out.push(Episode {
                id,
                session_id,
                started_at: parse_ts(&started)?,
                ended_at: parse_ts(&ended)?,
                message_count: count,
                summary,
                created_at: parse_ts(&created)?,
            });
        }
        Ok(out)
    }
}

struct SessionStats {
    count: i64,
    min_ts: Option<String>,
    max_ts: Option<String>,
    senders: Option<String>,
}

/// L0 → L1 — session 의 모든 messages 를 모아 1개 episode 로 집계.
/// Phase 1: 단순 카운트·시간 범위·sender 수. 의미 요약은 fastembed/LLM 통합 이후.
pub fn reflect_session(db: &mut Db, session_id: &str) -> Result<Option<Episode>> {
    let conn = db.conn();

    let stats: Option<SessionStats> = conn
        .query_row(
            "SELECT COUNT(*), MIN(timestamp), MAX(timestamp), GROUP_CONCAT(DISTINCT sender)
             FROM messages WHERE session_id = ?1",
            [session_id],
            |r| {
                Ok(SessionStats {
                    count: r.get(0)?,
                    min_ts: r.get(1)?,
                    max_ts: r.get(2)?,
                    senders: r.get(3)?,
                })
            },
        )
        .ok();
    let stats = match stats {
        Some(s) if s.count > 0 => s,
        _ => return Ok(None),
    };

    let count = stats.count;
    let started = stats.min_ts.expect("count>0 implies MIN(timestamp) Some");
    let ended = stats.max_ts.expect("count>0 implies MAX(timestamp) Some");
    let senders_str = stats.senders.unwrap_or_default();
    let sender_count = senders_str.split(',').filter(|s| !s.is_empty()).count();

    let summary = format!(
        "{count} message{} from {sender_count} sender{}",
        if count == 1 { "" } else { "s" },
        if sender_count == 1 { "" } else { "s" },
    );

    let id = Uuid::new_v4().to_string();
    let now = kst_now();
    let now_rfc = now.to_rfc3339();

    let affected = conn.execute(
        "INSERT INTO episodes (id, session_id, started_at, ended_at, message_count, summary, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, session_id, started, ended, count, summary, now_rfc],
    )?;
    if affected != 1 {
        return Err(MemoryError::UnexpectedRowCount {
            expected: 1,
            actual: affected as u64,
        });
    }

    Ok(Some(Episode {
        id,
        session_id: session_id.into(),
        started_at: parse_ts(&started)?,
        ended_at: parse_ts(&ended)?,
        message_count: count,
        summary,
        created_at: now,
    }))
}
