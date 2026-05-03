//! openxgram-vault — 암호화 자격증명 저장 (PRD §8).
//!
//! 디스크 암호화: keystore::encrypt_blob (ChaCha20-Poly1305 + Argon2id).
//! ACL · daily 한도 · MFA · 머신 화이트리스트는 후속 PR.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbError};
use openxgram_keystore::{decrypt_blob, encrypt_blob, KeystoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("db error: {0}")]
    Db(#[from] DbError),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("keystore error: {0}")]
    Keystore(#[from] KeystoreError),

    #[error("entry not found: {0}")]
    NotFound(String),

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),

    #[error("unexpected affected rows: expected {expected}, got {actual}")]
    UnexpectedRowCount { expected: u64, actual: u64 },
}

pub type Result<T> = std::result::Result<T, VaultError>;

/// metadata 만 노출 — 평문 값은 별도 get(key, password) 호출.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultEntry {
    pub id: String,
    pub key: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<FixedOffset>,
    pub last_accessed: DateTime<FixedOffset>,
}

pub struct VaultStore<'a> {
    db: &'a mut Db,
}

impl<'a> VaultStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// upsert: 같은 key 있으면 encrypted_value/tags/last_accessed 갱신.
    pub fn set(
        &mut self,
        key: &str,
        plaintext: &[u8],
        password: &str,
        tags: &[String],
    ) -> Result<VaultEntry> {
        let encrypted = encrypt_blob(password, plaintext)?;
        let now = kst_now();
        let now_rfc = now.to_rfc3339();
        let id = Uuid::new_v4().to_string();
        let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".into());

        self.db.conn().execute(
            "INSERT INTO vault_entries (id, key, encrypted_value, tags, created_at, last_accessed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(key) DO UPDATE SET
                 encrypted_value = ?3, tags = ?4, last_accessed = ?5",
            rusqlite::params![id, key, encrypted, tags_json, now_rfc],
        )?;

        self.get_entry(key)?
            .ok_or_else(|| VaultError::NotFound(format!("upsert lost: {key}")))
    }

    /// 평문 값 복호화. 잘못된 패스워드 → KeystoreError::InvalidPassword raise.
    /// 호출 시 last_accessed 갱신.
    pub fn get(&mut self, key: &str, password: &str) -> Result<Vec<u8>> {
        let conn = self.db.conn();
        let encrypted: Vec<u8> = conn
            .query_row(
                "SELECT encrypted_value FROM vault_entries WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    VaultError::NotFound(key.to_string())
                }
                other => VaultError::Sqlite(other),
            })?;
        let plaintext = decrypt_blob(password, &encrypted)?;

        let now_rfc = kst_now().to_rfc3339();
        let affected = conn.execute(
            "UPDATE vault_entries SET last_accessed = ?1 WHERE key = ?2",
            rusqlite::params![now_rfc, key],
        )?;
        if affected != 1 {
            return Err(VaultError::UnexpectedRowCount {
                expected: 1,
                actual: affected as u64,
            });
        }
        Ok(plaintext)
    }

    pub fn list(&mut self) -> Result<Vec<VaultEntry>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, key, tags, created_at, last_accessed
             FROM vault_entries ORDER BY last_accessed DESC",
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
            let (id, key, tags_json, created, last) = row?;
            out.push(VaultEntry {
                id,
                key,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                created_at: parse_ts(&created)?,
                last_accessed: parse_ts(&last)?,
            });
        }
        Ok(out)
    }

    pub fn delete(&mut self, key: &str) -> Result<()> {
        let affected = self
            .db
            .conn()
            .execute("DELETE FROM vault_entries WHERE key = ?1", [key])?;
        if affected != 1 {
            return Err(VaultError::NotFound(key.to_string()));
        }
        Ok(())
    }

    fn get_entry(&mut self, key: &str) -> Result<Option<VaultEntry>> {
        let result = self.db.conn().query_row(
            "SELECT id, key, tags, created_at, last_accessed
             FROM vault_entries WHERE key = ?1",
            [key],
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
            Ok((id, key, tags_json, created, last)) => Ok(Some(VaultEntry {
                id,
                key,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                created_at: parse_ts(&created)?,
                last_accessed: parse_ts(&last)?,
            })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn parse_ts(s: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).map_err(|e| VaultError::InvalidTimestamp(e.to_string()))
}
