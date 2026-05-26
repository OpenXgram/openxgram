//! UI-MEMORY-SPEC v1.1 §K7 / §1.2 — 메시지 L0 저장의 단일 진입점.
//!
//! audit 결과 (W의 지적, 2026-05-22): messages 테이블에 INSERT 하는 path 가 8개 —
//! peer_send / gui_memory_l0 / claude_ingest / webhook / migration_import /
//! bundle_import / discord_listener / telegram_listener — 각각 SQL 컬럼·signature·
//! session 보장 로직이 미세하게 달라서 일관성 깨질 위험.
//!
//! 본 모듈이 단일 canonical write path. 모든 caller 가 `save_l0_message()` 만 호출.

use openxgram_db::Db;
use openxgram_memory::embed::Embedder;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct L0SaveInput<'a> {
    pub id: Option<String>, // None → uuid 생성
    pub session_id: &'a str,
    pub session_title: Option<&'a str>,
    pub sender: &'a str,
    pub body: &'a str,
    pub signature: &'a str, // "external" / "import" / "claude-ingest" / hex signature
    pub timestamp: Option<&'a str>, // None → now
    pub parent_message_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>, // None → session_id 사용
    pub source: &'a str, // "messenger" / "claude_ingest" / "webhook" / "discord" / "telegram" / "import"
    pub extra_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct L0SaveResult {
    pub id: String,
    pub conversation_id: String,
    pub timestamp: String,
    pub inserted: bool, // false = duplicate (INSERT OR IGNORE 효과)
}

/// 메시지를 L0 (messages 테이블) 에 저장. session 자동 보장.
/// embedder 가 Some 이면 저장 직후 임베딩 → message_embeddings + message_embedding_map INSERT.
/// 임베딩 실패는 tracing::warn 으로 드러내고 메시지 저장은 보존.
///
/// 안티패턴 10: 직접 INSERT 금지. 모든 caller 가 본 함수 호출.
pub fn save_l0_message(
    db: &mut Db,
    input: L0SaveInput,
    embedder: Option<&(dyn Embedder + Send + Sync)>,
) -> Result<L0SaveResult, rusqlite::Error> {
    let id = input.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = chrono::Utc::now().to_rfc3339();
    let timestamp = input.timestamp.unwrap_or(&now).to_string();
    let conv_id = input.conversation_id.map(String::from).unwrap_or_else(|| input.session_id.to_string());

    // metadata = {source, ...extra}
    let mut meta = serde_json::Map::new();
    meta.insert("source".into(), serde_json::Value::String(input.source.into()));
    if let Some(extra) = input.extra_metadata {
        if let serde_json::Value::Object(m) = extra {
            for (k, v) in m { meta.insert(k, v); }
        } else {
            meta.insert("extra".into(), extra);
        }
    }
    let metadata_str = serde_json::Value::Object(meta).to_string();

    let conn = db.conn();
    // 1) session 보장 (이미 있으면 last_active 만 갱신)
    let title = input.session_title.unwrap_or(input.session_id);
    let participants = match input.source {
        "claude_ingest" => "[\"W\",\"Claude\"]",
        "discord" | "telegram" => "[\"W\",\"channel-user\"]",
        "import" | "webhook" => "[\"W\",\"imported\"]",
        _ => "[]",
    };
    let _ = conn.execute(
        "INSERT OR IGNORE INTO sessions (id, title, participants, created_at, last_active, home_machine) \
         VALUES (?1, ?2, ?3, ?4, ?4, 'server-seoul')",
        params![input.session_id, title, participants, now],
    );
    let _ = conn.execute(
        "UPDATE sessions SET last_active = ?1 WHERE id = ?2",
        params![now, input.session_id],
    );

    // 2) messages INSERT OR IGNORE (id 중복 시 no-op)
    let affected = conn.execute(
        "INSERT OR IGNORE INTO messages \
         (id, session_id, sender, body, signature, timestamp, parent_message_id, metadata, conversation_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            input.session_id,
            input.sender,
            input.body,
            input.signature,
            timestamp,
            input.parent_message_id,
            metadata_str,
            conv_id
        ],
    )?;

    let inserted = affected > 0;

    // 저장 직후 실시간 임베딩 (embedder 주입된 경우)
    if inserted {
        if let Some(emb) = embedder {
            match openxgram_memory::message::embed_and_store(db, &id, input.body, emb) {
                Ok(true) => tracing::debug!(message_id = %id, "save_l0: 임베딩 완료"),
                Ok(false) => tracing::debug!(message_id = %id, "save_l0: 임베딩 이미 존재 (skip)"),
                Err(e) => tracing::warn!(message_id = %id, error = %e, "save_l0: 임베딩 실패 — 메시지는 보존됨"),
            }
        }
    }

    Ok(L0SaveResult {
        id,
        conversation_id: conv_id,
        timestamp,
        inserted,
    })
}
