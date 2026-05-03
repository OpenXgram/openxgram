//! openxgram-vault — 암호화 자격증명 저장 (PRD §8).
//!
//! 디스크 암호화: keystore::encrypt_blob (ChaCha20-Poly1305 + Argon2id).
//! ACL: agent × key 패턴 매칭, 일일 한도 enforcement, 감사 로그 (vault_audit).
//! MFA 정책은 vault_acl.policy 컬럼에 저장 (Phase 1 enforcement 는 auto 만 즉시).
//! 머신 화이트리스트는 후속 PR.

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

    #[error("acl denied: {0}")]
    AclDenied(String),

    #[error("invalid acl: {0}")]
    InvalidAcl(String),
}

/// 마스터 호출 — ACL 우회. 다른 식별자는 ACL 검사 통과 필요.
pub const MASTER_AGENT: &str = "master";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AclAction {
    Get,
    Set,
    Delete,
}

impl AclAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Set => "set",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AclPolicy {
    Auto,
    Confirm,
    Mfa,
}

impl AclPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Confirm => "confirm",
            Self::Mfa => "mfa",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "auto" => Self::Auto,
            "confirm" => Self::Confirm,
            "mfa" => Self::Mfa,
            other => return Err(VaultError::InvalidAcl(format!("policy: {other}"))),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclEntry {
    pub id: String,
    pub key_pattern: String,
    pub agent: String,
    pub allowed_actions: Vec<AclAction>,
    pub daily_limit: i64,
    pub policy: AclPolicy,
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone)]
pub struct AclDecision {
    pub allowed: bool,
    pub policy: AclPolicy,
    pub reason: Option<String>,
    pub matched_acl_id: Option<String>,
    pub daily_used: i64,
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

// ── ACL ──────────────────────────────────────────────────────────────────

impl<'a> VaultStore<'a> {
    /// ACL 등록/갱신. (key_pattern, agent) 가 unique 키 — 중복 시 갱신.
    pub fn upsert_acl(
        &mut self,
        key_pattern: &str,
        agent: &str,
        allowed_actions: &[AclAction],
        daily_limit: i64,
        policy: AclPolicy,
    ) -> Result<AclEntry> {
        if allowed_actions.is_empty() {
            return Err(VaultError::InvalidAcl(
                "allowed_actions 비어있음".into(),
            ));
        }
        let id = Uuid::new_v4().to_string();
        let now_rfc = kst_now().to_rfc3339();
        let actions_str = encode_actions(allowed_actions);

        self.db.conn().execute(
            "INSERT INTO vault_acl (id, key_pattern, agent, allowed_actions, daily_limit, policy, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(key_pattern, agent) DO UPDATE SET
                 allowed_actions = ?4, daily_limit = ?5, policy = ?6",
            rusqlite::params![id, key_pattern, agent, actions_str, daily_limit, policy.as_str(), now_rfc],
        )?;

        self.get_acl(key_pattern, agent)?
            .ok_or_else(|| VaultError::InvalidAcl(format!("upsert lost: {key_pattern}/{agent}")))
    }

    pub fn delete_acl(&mut self, key_pattern: &str, agent: &str) -> Result<()> {
        let affected = self.db.conn().execute(
            "DELETE FROM vault_acl WHERE key_pattern = ?1 AND agent = ?2",
            [key_pattern, agent],
        )?;
        if affected != 1 {
            return Err(VaultError::NotFound(format!("{key_pattern}/{agent}")));
        }
        Ok(())
    }

    pub fn list_acl(&mut self) -> Result<Vec<AclEntry>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, key_pattern, agent, allowed_actions, daily_limit, policy, created_at
             FROM vault_acl ORDER BY key_pattern, agent",
        )?;
        let rows = stmt.query_map([], |r| {
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
            let (id, key_pattern, agent, actions, daily_limit, policy, created) = row?;
            out.push(AclEntry {
                id,
                key_pattern,
                agent,
                allowed_actions: parse_actions(&actions)?,
                daily_limit,
                policy: AclPolicy::parse(&policy)?,
                created_at: parse_ts(&created)?,
            });
        }
        Ok(out)
    }

    fn get_acl(&mut self, key_pattern: &str, agent: &str) -> Result<Option<AclEntry>> {
        let result = self.db.conn().query_row(
            "SELECT id, key_pattern, agent, allowed_actions, daily_limit, policy, created_at
             FROM vault_acl WHERE key_pattern = ?1 AND agent = ?2",
            [key_pattern, agent],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        );
        match result {
            Ok((id, key_pattern, agent, actions, daily_limit, policy, created)) => Ok(Some(AclEntry {
                id,
                key_pattern,
                agent,
                allowed_actions: parse_actions(&actions)?,
                daily_limit,
                policy: AclPolicy::parse(&policy)?,
                created_at: parse_ts(&created)?,
            })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// (key, agent, action) 에 대한 ACL 결정. 매칭 우선순위: exact key + exact agent
    /// > exact key + '*' > '*' + exact agent > '*' + '*'. 첫 매칭 사용.
    pub fn check_acl(
        &mut self,
        key: &str,
        agent: &str,
        action: AclAction,
    ) -> Result<AclDecision> {
        let acl = self.find_matching_acl(key, agent)?;
        let Some(acl) = acl else {
            return Ok(AclDecision {
                allowed: false,
                policy: AclPolicy::Auto,
                reason: Some("no acl matches".into()),
                matched_acl_id: None,
                daily_used: 0,
            });
        };

        if !acl.allowed_actions.contains(&action) {
            return Ok(AclDecision {
                allowed: false,
                policy: acl.policy,
                reason: Some(format!("action {} not allowed", action.as_str())),
                matched_acl_id: Some(acl.id.clone()),
                daily_used: 0,
            });
        }

        let used = self.count_today(key, agent, action)?;
        if acl.daily_limit > 0 && used >= acl.daily_limit {
            return Ok(AclDecision {
                allowed: false,
                policy: acl.policy,
                reason: Some(format!(
                    "daily limit exceeded ({} >= {})",
                    used, acl.daily_limit
                )),
                matched_acl_id: Some(acl.id),
                daily_used: used,
            });
        }

        Ok(AclDecision {
            allowed: true,
            policy: acl.policy,
            reason: None,
            matched_acl_id: Some(acl.id),
            daily_used: used,
        })
    }

    fn find_matching_acl(&mut self, key: &str, agent: &str) -> Result<Option<AclEntry>> {
        // 우선순위 4 단계 — 가장 구체적 → 가장 일반적
        for (kp, ag) in [(key, agent), (key, "*"), ("*", agent), ("*", "*")] {
            if let Some(acl) = self.get_acl(kp, ag)? {
                return Ok(Some(acl));
            }
        }
        Ok(None)
    }

    /// 오늘(KST) 자정 이후 (key, agent, action) 의 allowed=1 호출 횟수.
    pub fn count_today(&mut self, key: &str, agent: &str, action: AclAction) -> Result<i64> {
        let day_start = kst_now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("KST midnight is valid")
            .and_local_timezone(*kst_now().offset())
            .single()
            .expect("KST midnight unique")
            .to_rfc3339();
        let count: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM vault_audit
             WHERE key = ?1 AND agent = ?2 AND action = ?3 AND allowed = 1 AND timestamp >= ?4",
            rusqlite::params![key, agent, action.as_str(), day_start],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    pub fn log_audit(
        &mut self,
        key: &str,
        agent: &str,
        action: AclAction,
        decision: &AclDecision,
    ) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let now_rfc = kst_now().to_rfc3339();
        let allowed_int: i64 = if decision.allowed { 1 } else { 0 };
        self.db.conn().execute(
            "INSERT INTO vault_audit (id, key, agent, action, allowed, reason, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, key, agent, action.as_str(), allowed_int, decision.reason, now_rfc],
        )?;
        Ok(())
    }

    /// agent 식별자가 있는 호출자의 vault.get — ACL 검사 + 감사 로그.
    /// MASTER_AGENT 는 ACL 우회.
    pub fn get_as(&mut self, key: &str, password: &str, agent: &str) -> Result<Vec<u8>> {
        if agent == MASTER_AGENT {
            return self.get(key, password);
        }
        let decision = self.check_acl(key, agent, AclAction::Get)?;
        self.log_audit(key, agent, AclAction::Get, &decision)?;
        if !decision.allowed {
            return Err(VaultError::AclDenied(
                decision.reason.unwrap_or_else(|| "denied".into()),
            ));
        }
        self.get(key, password)
    }

    pub fn set_as(
        &mut self,
        key: &str,
        plaintext: &[u8],
        password: &str,
        tags: &[String],
        agent: &str,
    ) -> Result<VaultEntry> {
        if agent == MASTER_AGENT {
            return self.set(key, plaintext, password, tags);
        }
        let decision = self.check_acl(key, agent, AclAction::Set)?;
        self.log_audit(key, agent, AclAction::Set, &decision)?;
        if !decision.allowed {
            return Err(VaultError::AclDenied(
                decision.reason.unwrap_or_else(|| "denied".into()),
            ));
        }
        self.set(key, plaintext, password, tags)
    }

    pub fn delete_as(&mut self, key: &str, agent: &str) -> Result<()> {
        if agent == MASTER_AGENT {
            return self.delete(key);
        }
        let decision = self.check_acl(key, agent, AclAction::Delete)?;
        self.log_audit(key, agent, AclAction::Delete, &decision)?;
        if !decision.allowed {
            return Err(VaultError::AclDenied(
                decision.reason.unwrap_or_else(|| "denied".into()),
            ));
        }
        self.delete(key)
    }
}

fn encode_actions(actions: &[AclAction]) -> String {
    let parts: Vec<&str> = actions.iter().map(|a| a.as_str()).collect();
    parts.join(",")
}

fn parse_actions(s: &str) -> Result<Vec<AclAction>> {
    let mut out = Vec::new();
    for token in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        out.push(match token {
            "get" => AclAction::Get,
            "set" => AclAction::Set,
            "delete" => AclAction::Delete,
            other => return Err(VaultError::InvalidAcl(format!("action: {other}"))),
        });
    }
    Ok(out)
}

fn parse_ts(s: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).map_err(|e| VaultError::InvalidTimestamp(e.to_string()))
}
