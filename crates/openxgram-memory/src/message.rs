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
    /// inbound + 그에 따른 응답·서브 호출·outbox 회신을 묶는 ID.
    /// 신규 inbound 는 새 ID, 응답/회신은 inbound 의 ID 를 재사용.
    pub conversation_id: String,
}

#[derive(Debug, Clone)]
pub struct RecallHit {
    pub message: Message,
    /// sqlite-vec L2 distance — 작을수록 유사.
    pub distance: f32,
    /// "message" | "memory" — 어떤 테이블에서 왔는지.
    pub source: String,
}

pub struct MessageStore<'a, E: Embedder + ?Sized> {
    db: &'a mut Db,
    embedder: &'a E,
}

impl<'a, E: Embedder + ?Sized> MessageStore<'a, E> {
    pub fn new(db: &'a mut Db, embedder: &'a E) -> Self {
        Self { db, embedder }
    }

    /// 메시지 + 임베딩을 한 트랜잭션으로 저장.
    ///
    /// `conversation_id`:
    /// - `None` → 새 UUID 생성 (신규 inbound 가 시작하는 conversation)
    /// - `Some(id)` → 같은 conversation 에 묶음 (응답·서브 호출·회신)
    pub fn insert(
        &mut self,
        session_id: &str,
        sender: &str,
        body: &str,
        signature: &str,
        conversation_id: Option<&str>,
    ) -> Result<Message> {
        let embedding = self.embedder.embed_passage(body);
        if embedding.len() != self.embedder.dim() {
            return Err(MemoryError::DimMismatch {
                got: embedding.len(),
                expected: self.embedder.dim(),
            });
        }

        let id = Uuid::new_v4().to_string();
        let conv_id = conversation_id
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = kst_now();
        let now_rfc3339 = now.to_rfc3339();
        let embedding_bytes = floats_to_bytes(&embedding);

        let conn = self.db.conn();
        let tx = conn.transaction()?;

        let affected = tx.execute(
            "INSERT INTO messages (id, session_id, sender, body, signature, timestamp, conversation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, session_id, sender, body, signature, now_rfc3339, conv_id],
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
            conversation_id: conv_id,
        })
    }

    /// session 내 모든 메시지 (timestamp 오름차순).
    pub fn list_for_session(&mut self, session_id: &str) -> Result<Vec<Message>> {
        self.query_messages(
            "SELECT id, session_id, sender, body, signature, timestamp, conversation_id
             FROM messages WHERE session_id = ?1 ORDER BY timestamp",
            rusqlite::params![session_id],
        )
    }

    /// 동일 conversation_id 의 모든 메시지 (timestamp 오름차순) — cross-session.
    pub fn list_for_conversation(&mut self, conversation_id: &str) -> Result<Vec<Message>> {
        self.query_messages(
            "SELECT id, session_id, sender, body, signature, timestamp, conversation_id
             FROM messages WHERE conversation_id = ?1 ORDER BY timestamp",
            rusqlite::params![conversation_id],
        )
    }

    /// 최근 N개 메시지 (timestamp 내림차순) — 전체 세션 합쳐서.
    /// GUI Messenger 의 "활동 흐름" 모니터링용.
    pub fn list_recent(&mut self, limit: usize) -> Result<Vec<Message>> {
        self.query_messages(
            "SELECT id, session_id, sender, body, signature, timestamp, conversation_id
             FROM messages ORDER BY timestamp DESC LIMIT ?1",
            rusqlite::params![limit as i64],
        )
    }

    fn query_messages(&mut self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Message>> {
        let mut stmt = self.db.conn().prepare(sql)?;
        let rows = stmt.query_map(params, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, session_id, sender, body, signature, ts, conv) = row?;
            out.push(Message {
                id,
                session_id,
                sender,
                body,
                signature,
                timestamp: parse_ts(&ts)?,
                conversation_id: conv.unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// 쿼리 텍스트와 가장 유사한 K 개 메시지 (sqlite-vec KNN).
    /// L2 memories 도 함께 검색해 통합 랭킹으로 반환.
    /// `source` 필드: "message" | "memory".
    pub fn recall_top_k(&mut self, query_text: &str, k: usize) -> Result<Vec<RecallHit>> {
        // query prefix 적용 — e5 권장 패턴
        let q_bytes = floats_to_bytes(&self.embedder.embed_query(query_text));
        // 통합 결과를 위해 k*2 를 각각 검색 후 병합
        let fetch_k = (k * 2).max(k);
        let conn = self.db.conn();

        // --- messages KNN ---
        let msg_rows: Vec<(f32, RawRow)> = {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.session_id, m.sender, m.body, m.signature, m.timestamp,
                        m.conversation_id, emb.distance
                 FROM message_embeddings emb
                 JOIN message_embedding_map map ON map.embedding_rowid = emb.rowid
                 JOIN messages m ON m.id = map.message_id
                 WHERE emb.embedding MATCH ?1 AND k = ?2
                 ORDER BY emb.distance",
            )?;
            let rows = stmt.query_map(rusqlite::params![q_bytes, fetch_k as i64], |r| {
                let ts: String = r.get(5)?;
                let dist: f32 = r.get(7)?;
                Ok((dist, RawRow {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    sender: r.get(2)?,
                    body: r.get(3)?,
                    signature: r.get(4)?,
                    timestamp_rfc3339: ts,
                    conversation_id: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    distance: dist,
                    source: "message".into(),
                }))
            })?;
            let mut out = Vec::new();
            for row in rows { out.push(row?); }
            out
        };

        // --- memories KNN (memory_embeddings 테이블이 없으면 skip) ---
        let mem_rows: Vec<(f32, RawRow)> = {
            // 테이블 존재 여부 확인
            let tbl_exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='memory_embedding_map')",
                [],
                |r| r.get(0),
            )?;
            if tbl_exists {
                let mut stmt = conn.prepare(
                    "SELECT mem.id, COALESCE(mem.session_id,''), 'memory', mem.content,
                            mem.kind, mem.created_at, mem.kind, emb.distance
                     FROM memory_embeddings emb
                     JOIN memory_embedding_map map ON map.embedding_rowid = emb.rowid
                     JOIN memories mem ON mem.id = map.memory_id
                     WHERE emb.embedding MATCH ?1 AND k = ?2
                     ORDER BY emb.distance",
                )?;
                let rows = stmt.query_map(rusqlite::params![q_bytes, fetch_k as i64], |r| {
                    let ts: String = r.get(5)?;
                    let dist: f32 = r.get(7)?;
                    Ok((dist, RawRow {
                        id: r.get(0)?,
                        session_id: r.get(1)?,
                        sender: r.get(2)?,
                        body: r.get(3)?,
                        signature: r.get(4)?,
                        timestamp_rfc3339: ts,
                        conversation_id: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        distance: dist,
                        source: "memory".into(),
                    }))
                })?;
                let mut out = Vec::new();
                for row in rows { out.push(row?); }
                out
            } else {
                Vec::new()
            }
        };

        // 병합 후 distance 오름차순 정렬 → 상위 k
        let mut combined: Vec<(f32, RawRow)> = msg_rows.into_iter().chain(mem_rows).collect();
        combined.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        combined.truncate(k);

        let mut hits = Vec::new();
        for (_dist, raw) in combined {
            hits.push(RecallHit {
                message: Message {
                    id: raw.id,
                    session_id: raw.session_id,
                    sender: raw.sender,
                    body: raw.body,
                    signature: raw.signature,
                    timestamp: parse_ts(&raw.timestamp_rfc3339)?,
                    conversation_id: raw.conversation_id,
                },
                distance: raw.distance,
                source: raw.source,
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
    conversation_id: String,
    distance: f32,
    source: String,
}

/// message_id + body 를 받아 임베딩 → message_embeddings + message_embedding_map INSERT.
///
/// 이미 map에 있으면 skip (idempotent).
/// 임베딩 실패 시 tracing::warn 으로 드러내고 Ok(false) 반환 — 메시지 저장 자체는 보존.
pub fn embed_and_store<E: Embedder + ?Sized>(
    db: &mut Db,
    message_id: &str,
    body: &str,
    embedder: &E,
) -> Result<bool> {
    // idempotent: 이미 임베딩된 메시지면 skip
    {
        let conn = db.conn();
        let already: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_embedding_map WHERE message_id = ?1)",
            rusqlite::params![message_id],
            |r| r.get(0),
        )?;
        if already {
            return Ok(false);
        }
    }

    let embedding = embedder.embed_passage(body);
    if embedding.len() != embedder.dim() {
        return Err(MemoryError::DimMismatch {
            got: embedding.len(),
            expected: embedder.dim(),
        });
    }
    let embedding_bytes = floats_to_bytes(&embedding);

    let conn = db.conn();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO message_embeddings (embedding) VALUES (?1)",
        rusqlite::params![embedding_bytes],
    )?;
    let embedding_rowid = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO message_embedding_map (message_id, embedding_rowid) VALUES (?1, ?2)",
        rusqlite::params![message_id, embedding_rowid],
    )?;
    tx.commit()?;

    Ok(true)
}

/// 임베딩이 없는 기존 메시지를 일괄 임베딩한다.
///
/// `message_embedding_map` 에 없는 메시지를 순서대로 순회하며
/// embedder 로 임베딩 → `message_embeddings` + `message_embedding_map` INSERT.
/// 이미 맵에 있는 메시지는 건너뛴다 (idempotent).
///
/// 반환값: (처리된 건수, 전체 미임베딩 건수)
pub fn backfill_message_embeddings<E: Embedder + ?Sized>(
    db: &mut Db,
    embedder: &E,
) -> Result<(usize, usize)> {
    // 미임베딩 메시지 전체를 id + body 로 가져온다.
    let unembedded: Vec<(String, String)> = {
        let conn = db.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.body FROM messages m
             WHERE NOT EXISTS (
                 SELECT 1 FROM message_embedding_map map WHERE map.message_id = m.id
             )
             ORDER BY m.timestamp",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    let total = unembedded.len();
    let mut done = 0usize;

    for (id, body) in &unembedded {
        let embedding = embedder.embed_passage(body);
        if embedding.len() != embedder.dim() {
            return Err(MemoryError::DimMismatch {
                got: embedding.len(),
                expected: embedder.dim(),
            });
        }
        let embedding_bytes = floats_to_bytes(&embedding);

        let conn = db.conn();
        let tx = conn.transaction()?;
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

        done += 1;
        if done % 100 == 0 || done == total {
            eprintln!("[backfill] {done}/{total} 완료");
        }
    }

    Ok((done, total))
}
