//! openxgram-memory — L0 messages 저장 + 임베딩 + 회상 (Phase 1 baseline).
//!
//! Phase 1 first PR 범위:
//!   - Embedder trait + DummyEmbedder (SHA256 결정성 384d 정규화)
//!   - MessageStore::insert (트랜잭션: messages + vec0 + map)
//!   - MessageStore::recall_top_k (sqlite-vec KNN)
//!
//! 후속 PR:
//!   - fastembed multilingual-e5-small 통합
//!   - L1 episodes / L2 memories store
//!   - 회상 복합 점수 (α 의미 + β 시간 + γ pin + δ 접근빈도, PRD §5)
//!   - 야간 reflection (L0 → L1 → L2 → L3 → L4)

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const EMBED_DIM: usize = 384;

/// 임베딩 추상화. fastembed·dummy 등 어떤 구현체도 같은 차원·동일 dtype.
pub trait Embedder: Send + Sync {
    fn dim(&self) -> usize;
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// SHA256 해시 기반 결정성 384d 임베딩. 통합 테스트·CI 용.
/// 같은 텍스트 → 같은 벡터. 의미 유사도는 보장하지 않으나, 동일 텍스트는
/// distance 0 으로 검색되어 회상 알고리즘 검증에 충분.
pub struct DummyEmbedder;

impl Embedder for DummyEmbedder {
    fn dim(&self) -> usize {
        EMBED_DIM
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let hash = hasher.finalize();

        let mut out = Vec::with_capacity(EMBED_DIM);
        for i in 0..EMBED_DIM {
            // 32B hash 를 384 floats 로 확장. index 별 mixing 으로 다양성 확보.
            let byte = hash[i % 32];
            let mixed = (byte as f32 - 128.0) / 128.0
                + (i as f32 / EMBED_DIM as f32 - 0.5) * 0.001;
            out.push(mixed);
        }
        // L2 정규화 — distance 비교 일관성
        let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut out {
                *v /= norm;
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub sender: String,
    pub body: String,
    pub signature: String,
    pub timestamp: DateTime<FixedOffset>,
}

#[derive(Debug, Clone)]
pub struct RecallHit {
    pub message: Message,
    /// sqlite-vec L2 distance — 작을수록 유사.
    pub distance: f32,
}

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("db error: {0}")]
    Db(#[from] openxgram_db::DbError),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("embedding dimension mismatch: got {got}, expected {expected}")]
    DimMismatch { got: usize, expected: usize },

    #[error("unexpected affected rows: expected {expected}, got {actual}")]
    UnexpectedRowCount { expected: u64, actual: u64 },

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),
}

pub type Result<T> = std::result::Result<T, MemoryError>;

pub struct MessageStore<'a, E: Embedder> {
    db: &'a mut Db,
    embedder: &'a E,
}

impl<'a, E: Embedder> MessageStore<'a, E> {
    pub fn new(db: &'a mut Db, embedder: &'a E) -> Self {
        Self { db, embedder }
    }

    /// 메시지 + 임베딩을 한 트랜잭션으로 저장.
    pub fn insert(
        &mut self,
        session_id: &str,
        sender: &str,
        body: &str,
        signature: &str,
    ) -> Result<Message> {
        let embedding = self.embedder.embed(body);
        if embedding.len() != self.embedder.dim() {
            return Err(MemoryError::DimMismatch {
                got: embedding.len(),
                expected: self.embedder.dim(),
            });
        }

        let id = Uuid::new_v4().to_string();
        let now = kst_now();
        let now_rfc3339 = now.to_rfc3339();
        let embedding_bytes = floats_to_bytes(&embedding);

        let conn = self.db.conn();
        let tx = conn.transaction()?;

        let affected = tx.execute(
            "INSERT INTO messages (id, session_id, sender, body, signature, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, session_id, sender, body, signature, now_rfc3339],
        )?;
        if affected != 1 {
            return Err(MemoryError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }

        tx.execute(
            "INSERT INTO message_embeddings (embedding) VALUES (?1)",
            rusqlite::params![embedding_bytes],
        )?;
        let embedding_rowid = tx.last_insert_rowid();

        tx.execute(
            "INSERT INTO message_embedding_map (message_id, embedding_rowid) VALUES (?1, ?2)",
            rusqlite::params![id, embedding_rowid],
        )?;

        tx.commit()?;

        Ok(Message {
            id,
            session_id: session_id.into(),
            sender: sender.into(),
            body: body.into(),
            signature: signature.into(),
            timestamp: now,
        })
    }

    /// 쿼리 텍스트와 가장 유사한 K 개 메시지 (sqlite-vec KNN).
    pub fn recall_top_k(&mut self, query_text: &str, k: usize) -> Result<Vec<RecallHit>> {
        let q_bytes = floats_to_bytes(&self.embedder.embed(query_text));
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.session_id, m.sender, m.body, m.signature, m.timestamp,
                    emb.distance
             FROM message_embeddings emb
             JOIN message_embedding_map map ON map.embedding_rowid = emb.rowid
             JOIN messages m ON m.id = map.message_id
             WHERE emb.embedding MATCH ?1 AND k = ?2
             ORDER BY emb.distance",
        )?;

        let rows = stmt.query_map(rusqlite::params![q_bytes, k as i64], |r| {
            let ts: String = r.get(5)?;
            Ok(RawRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                sender: r.get(2)?,
                body: r.get(3)?,
                signature: r.get(4)?,
                timestamp_rfc3339: ts,
                distance: r.get(6)?,
            })
        })?;

        let mut hits = Vec::new();
        for row in rows {
            let raw = row?;
            let ts = DateTime::parse_from_rfc3339(&raw.timestamp_rfc3339)
                .map_err(|e| MemoryError::InvalidTimestamp(e.to_string()))?;
            hits.push(RecallHit {
                message: Message {
                    id: raw.id,
                    session_id: raw.session_id,
                    sender: raw.sender,
                    body: raw.body,
                    signature: raw.signature,
                    timestamp: ts,
                },
                distance: raw.distance,
            });
        }
        Ok(hits)
    }
}

struct RawRow {
    id: String,
    session_id: String,
    sender: String,
    body: String,
    signature: String,
    timestamp_rfc3339: String,
    distance: f32,
}

// ── L1 episodes — session reflection 결과 ─────────────────────────────────

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

/// L0 → L1 — session 의 모든 messages 를 모아 1개 episode 로 집계.
/// Phase 1: 단순 카운트·시간 범위·sender 수. 의미 요약은 fastembed/LLM 통합 이후.
/// 같은 session 에 여러 번 호출하면 episode 가 누적 — Phase 1.5 에서 boundaries 처리.
struct SessionStats {
    count: i64,
    min_ts: Option<String>,
    max_ts: Option<String>,
    senders: Option<String>,
}

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
    let sender_count = senders_str
        .split(',')
        .filter(|s| !s.is_empty())
        .count();

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

fn parse_ts(s: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).map_err(|e| MemoryError::InvalidTimestamp(e.to_string()))
}

fn floats_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

