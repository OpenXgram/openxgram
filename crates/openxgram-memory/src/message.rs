//! L0 messages 저장 + 임베딩 + sqlite-vec KNN 회상.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::embed::Embedder;
use crate::util::{floats_to_bytes, parse_ts};
use crate::{MemoryError, Result};

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
            hits.push(RecallHit {
                message: Message {
                    id: raw.id,
                    session_id: raw.session_id,
                    sender: raw.sender,
                    body: raw.body,
                    signature: raw.signature,
                    timestamp: parse_ts(&raw.timestamp_rfc3339)?,
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
