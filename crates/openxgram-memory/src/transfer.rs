//! Memory Transfer — session 통째 export/import (PRD §17, SPEC-MT §4.1).
//!
//! Phase 1 first PR: export 만 (text-package-v1 JSON). import 는 후속 PR
//! (충돌 처리·embedding 재생성·서명 검증 추가).

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};

use crate::episode::EpisodeStore;
use crate::memory::{MemoryKind, MemoryStore};
use crate::message::MessageStore;
use crate::session::SessionStore;
use crate::{embed::DummyEmbedder, MemoryError, Result};

const FORMAT: &str = "text-package-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPackage {
    pub format: String,
    pub exported_at: DateTime<FixedOffset>,
    pub source_machine: String,
    pub session: PkgSession,
    pub messages: Vec<PkgMessage>,
    pub episodes: Vec<PkgEpisode>,
    pub memories: Vec<PkgMemory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgSession {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<FixedOffset>,
    pub last_active: DateTime<FixedOffset>,
    pub home_machine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgMessage {
    pub id: String,
    pub sender: String,
    pub body: String,
    pub signature: String,
    pub timestamp: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgEpisode {
    pub id: String,
    pub started_at: DateTime<FixedOffset>,
    pub ended_at: DateTime<FixedOffset>,
    pub message_count: i64,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgMemory {
    pub id: String,
    pub kind: MemoryKind,
    pub content: String,
    pub pinned: bool,
    pub importance: f64,
    pub created_at: DateTime<FixedOffset>,
}

/// session 의 모든 메시지·episode·memory 를 1 패키지로 묶어 반환.
pub fn export_session(db: &mut Db, session_id: &str, source_machine: &str) -> Result<TextPackage> {
    let session = SessionStore::new(db)
        .get_by_id(session_id)?
        .ok_or_else(|| MemoryError::InvalidKind(format!("session not found: {session_id}")))?;

    let embedder = DummyEmbedder;
    let messages = MessageStore::new(db, &embedder)
        .list_for_session(session_id)?
        .into_iter()
        .map(|m| PkgMessage {
            id: m.id,
            sender: m.sender,
            body: m.body,
            signature: m.signature,
            timestamp: m.timestamp,
        })
        .collect();

    let episodes = EpisodeStore::new(db)
        .list_for_session(session_id)?
        .into_iter()
        .map(|e| PkgEpisode {
            id: e.id,
            started_at: e.started_at,
            ended_at: e.ended_at,
            message_count: e.message_count,
            summary: e.summary,
        })
        .collect();

    let memories = MemoryStore::new(db)
        .list_for_session(session_id)?
        .into_iter()
        .map(|m| PkgMemory {
            id: m.id,
            kind: m.kind,
            content: m.content,
            pinned: m.pinned,
            importance: m.importance,
            created_at: m.created_at,
        })
        .collect();

    Ok(TextPackage {
        format: FORMAT.into(),
        exported_at: kst_now(),
        source_machine: source_machine.into(),
        session: PkgSession {
            id: session.id,
            title: session.title,
            created_at: session.created_at,
            last_active: session.last_active,
            home_machine: session.home_machine,
        },
        messages,
        episodes,
        memories,
    })
}

impl TextPackage {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| MemoryError::InvalidKind(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| MemoryError::InvalidKind(e.to_string()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportSummary {
    pub session_id: String,
    pub messages_inserted: usize,
    pub episodes_inserted: usize,
    pub memories_inserted: usize,
}

/// 새 session 으로 패키지를 흡수. 충돌 회피 위해 항상 새 ID 발급.
/// 메시지는 embedder 로 임베딩 재생성. episode/memory 는 새 ID 로 복사.
pub fn import_session(
    db: &mut Db,
    package: &TextPackage,
    home_machine: &str,
) -> Result<ImportSummary> {
    if package.format != FORMAT {
        return Err(MemoryError::InvalidKind(format!(
            "unsupported package format: {} (expected {})",
            package.format, FORMAT
        )));
    }

    let session = SessionStore::new(db).create(&package.session.title, home_machine)?;
    let new_session_id = session.id;

    let embedder = DummyEmbedder;
    let mut messages_inserted = 0;
    {
        let mut store = MessageStore::new(db, &embedder);
        for msg in &package.messages {
            store.insert(&new_session_id, &msg.sender, &msg.body, &msg.signature)?;
            messages_inserted += 1;
        }
    }

    let now = kst_now().to_rfc3339();
    let mut episodes_inserted = 0;
    for ep in &package.episodes {
        let new_id = uuid::Uuid::new_v4().to_string();
        let affected = db.conn().execute(
            "INSERT INTO episodes
                 (id, session_id, started_at, ended_at, message_count, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                new_id,
                new_session_id,
                ep.started_at.to_rfc3339(),
                ep.ended_at.to_rfc3339(),
                ep.message_count,
                ep.summary,
                now,
            ],
        )?;
        if affected != 1 {
            return Err(MemoryError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }
        episodes_inserted += 1;
    }

    let mut memories_inserted = 0;
    {
        let mut store = MemoryStore::new(db);
        for m in &package.memories {
            let inserted = store.insert(Some(&new_session_id), m.kind, &m.content)?;
            if m.pinned {
                store.set_pinned(&inserted.id, true)?;
            }
            memories_inserted += 1;
        }
    }

    Ok(ImportSummary {
        session_id: new_session_id,
        messages_inserted,
        episodes_inserted,
        memories_inserted,
    })
}
