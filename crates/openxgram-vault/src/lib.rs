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

/// Pending confirmation 만료 시간 (24시간) — 자동 만료는 후속.
pub const PENDING_TTL_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingConfirmation {
    pub id: String,
    pub key: String,
    pub agent: String,
    pub action: AclAction,
    pub status: PendingStatus,
    pub requested_at: DateTime<FixedOffset>,
    pub decided_at: Option<DateTime<FixedOffset>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

impl PendingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "expired" => Self::Expired,
            other => return Err(VaultError::InvalidAcl(format!("status: {other}"))),
        })
    }
}

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
                rusqlite::Error::QueryReturnedNoRows => VaultError::NotFound(key.to_string()),
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
            return Err(VaultError::InvalidAcl("allowed_actions 비어있음".into()));
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
            Ok((id, key_pattern, agent, actions, daily_limit, policy, created)) => {
                Ok(Some(AclEntry {
                    id,
                    key_pattern,
                    agent,
                    allowed_actions: parse_actions(&actions)?,
                    daily_limit,
                    policy: AclPolicy::parse(&policy)?,
                    created_at: parse_ts(&created)?,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// (key, agent, action) 에 대한 ACL 결정. 매칭 우선순위: exact key + exact agent
    /// > exact key + '*' > '*' + exact agent > '*' + '*'. 첫 매칭 사용.
    pub fn check_acl(&mut self, key: &str, agent: &str, action: AclAction) -> Result<AclDecision> {
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
            rusqlite::params![
                id,
                key,
                agent,
                action.as_str(),
                allowed_int,
                decision.reason,
                now_rfc
            ],
        )?;
        Ok(())
    }

    /// agent 식별자가 있는 호출자의 vault.get — ACL 검사 + 감사 로그.
    /// MASTER_AGENT 는 ACL 우회. policy=confirm/mfa 면 ensure_policy 라우팅.
    pub fn get_as(&mut self, key: &str, password: &str, agent: &str) -> Result<Vec<u8>> {
        self.get_as_authed(key, password, agent, None)
    }

    pub fn get_as_authed(
        &mut self,
        key: &str,
        password: &str,
        agent: &str,
        mfa_code: Option<&str>,
    ) -> Result<Vec<u8>> {
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
        self.ensure_policy(key, agent, AclAction::Get, mfa_code, decision.policy)?;
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
        self.set_as_authed(key, plaintext, password, tags, agent, None)
    }

    pub fn set_as_authed(
        &mut self,
        key: &str,
        plaintext: &[u8],
        password: &str,
        tags: &[String],
        agent: &str,
        mfa_code: Option<&str>,
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
        self.ensure_policy(key, agent, AclAction::Set, mfa_code, decision.policy)?;
        self.set(key, plaintext, password, tags)
    }

    pub fn delete_as(&mut self, key: &str, agent: &str) -> Result<()> {
        self.delete_as_authed(key, agent, None)
    }

    pub fn delete_as_authed(
        &mut self,
        key: &str,
        agent: &str,
        mfa_code: Option<&str>,
    ) -> Result<()> {
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
        self.ensure_policy(key, agent, AclAction::Delete, mfa_code, decision.policy)?;
        self.delete(key)
    }

    /// confirm: 마스터 승인 대기 — pending 큐 사용 / mfa: TOTP 코드 검증.
    fn ensure_policy(
        &mut self,
        key: &str,
        agent: &str,
        action: AclAction,
        mfa_code: Option<&str>,
        policy: AclPolicy,
    ) -> Result<()> {
        match policy {
            AclPolicy::Auto => Ok(()),
            AclPolicy::Confirm => {
                if self.consume_approved_confirmation(key, agent, action)? {
                    return Ok(());
                }
                let id = self.insert_pending(key, agent, action)?;
                Err(VaultError::AclDenied(format!(
                    "confirm 정책 — 마스터 승인 대기 중 (id={id}). `xgram vault approve {id}` 후 재시도."
                )))
            }
            AclPolicy::Mfa => {
                let code = mfa_code
                    .ok_or_else(|| VaultError::AclDenied("mfa 정책 — TOTP 코드 필요".into()))?;
                if self.validate_mfa(agent, code)? {
                    Ok(())
                } else {
                    Err(VaultError::AclDenied("mfa 코드 검증 실패".into()))
                }
            }
        }
    }

    // ── Pending Confirmations ────────────────────────────────────────────
    fn insert_pending(&mut self, key: &str, agent: &str, action: AclAction) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now_rfc = kst_now().to_rfc3339();
        self.db.conn().execute(
            "INSERT INTO vault_pending_confirmations (id, key, agent, action, status, requested_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
            rusqlite::params![id, key, agent, action.as_str(), now_rfc],
        )?;
        // 마스터 알림 — DISCORD_WEBHOOK_URL 환경 시 fire-and-forget POST.
        // 실패해도 vault 흐름 차단 안 함 (silent error 패턴, tracing 으로 기록).
        notify_pending_via_discord(&id, key, agent, action);
        Ok(id)
    }

    /// 마스터 승인 → status='approved'.
    pub fn approve_confirmation(&mut self, id: &str) -> Result<()> {
        self.set_pending_status(id, PendingStatus::Approved)
    }

    pub fn deny_confirmation(&mut self, id: &str) -> Result<()> {
        self.set_pending_status(id, PendingStatus::Denied)
    }

    fn set_pending_status(&mut self, id: &str, status: PendingStatus) -> Result<()> {
        let now_rfc = kst_now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE vault_pending_confirmations
             SET status = ?1, decided_at = ?2
             WHERE id = ?3 AND status = 'pending'",
            rusqlite::params![status.as_str(), now_rfc, id],
        )?;
        if affected != 1 {
            return Err(VaultError::NotFound(format!(
                "pending {id} (이미 처리됐거나 미존재)"
            )));
        }
        Ok(())
    }

    /// status='approved' 가장 오래된 row 발견 시 → 'expired' 처리(소비) 후 true.
    /// 없으면 false. (consume = 1회용 승인)
    fn consume_approved_confirmation(
        &mut self,
        key: &str,
        agent: &str,
        action: AclAction,
    ) -> Result<bool> {
        let id_opt: Option<String> = self
            .db
            .conn()
            .query_row(
                "SELECT id FROM vault_pending_confirmations
                 WHERE key = ?1 AND agent = ?2 AND action = ?3 AND status = 'approved'
                 ORDER BY decided_at ASC LIMIT 1",
                rusqlite::params![key, agent, action.as_str()],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok::<Option<String>, VaultError>(None),
                other => Err(other.into()),
            })?;

        let Some(id) = id_opt else { return Ok(false) };
        // 소비 — status='expired'
        let now_rfc = kst_now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE vault_pending_confirmations
             SET status = 'expired', decided_at = ?1
             WHERE id = ?2",
            rusqlite::params![now_rfc, id],
        )?;
        Ok(true)
    }

    pub fn list_pending(&mut self) -> Result<Vec<PendingConfirmation>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, key, agent, action, status, requested_at, decided_at
             FROM vault_pending_confirmations
             WHERE status = 'pending'
             ORDER BY requested_at ASC",
        )?;
        let rows = stmt.query_map([], |r| {
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
            let (id, key, agent, action, status, requested, decided) = row?;
            let action = match action.as_str() {
                "get" => AclAction::Get,
                "set" => AclAction::Set,
                "delete" => AclAction::Delete,
                other => return Err(VaultError::InvalidAcl(format!("action: {other}"))),
            };
            out.push(PendingConfirmation {
                id,
                key,
                agent,
                action,
                status: PendingStatus::parse(&status)?,
                requested_at: parse_ts(&requested)?,
                decided_at: decided.as_deref().map(parse_ts).transpose()?,
            });
        }
        Ok(out)
    }

    // ── MFA (TOTP RFC 6238) ──────────────────────────────────────────────
    /// agent 별 TOTP secret 생성·저장 → base32 secret 반환 (마스터가 authenticator 등록).
    pub fn issue_mfa_secret(&mut self, agent: &str) -> Result<String> {
        let secret = totp_rs::Secret::generate_secret();
        let base32 = secret.to_encoded().to_string();
        let id = Uuid::new_v4().to_string();
        let now_rfc = kst_now().to_rfc3339();
        self.db.conn().execute(
            "INSERT INTO vault_mfa_secrets (id, agent, secret_base32, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent) DO UPDATE SET secret_base32 = ?3, created_at = ?4",
            rusqlite::params![id, agent, base32, now_rfc],
        )?;
        Ok(base32)
    }

    pub fn validate_mfa(&mut self, agent: &str, code: &str) -> Result<bool> {
        let secret_b32: Option<String> = self
            .db
            .conn()
            .query_row(
                "SELECT secret_base32 FROM vault_mfa_secrets WHERE agent = ?1",
                [agent],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok::<Option<String>, VaultError>(None),
                other => Err(other.into()),
            })?;
        let Some(base32) = secret_b32 else {
            return Err(VaultError::AclDenied(format!(
                "mfa secret 미등록 — `xgram vault mfa-issue --agent {agent}` 먼저"
            )));
        };
        let raw = totp_rs::Secret::Encoded(base32)
            .to_bytes()
            .map_err(|e| VaultError::InvalidAcl(format!("base32 decode: {e}")))?;
        let totp = totp_rs::TOTP::new(
            totp_rs::Algorithm::SHA1,
            6,
            1,
            30,
            raw,
            Some("OpenXgram".into()),
            agent.to_string(),
        )
        .map_err(|e| VaultError::InvalidAcl(format!("totp init: {e}")))?;
        totp.check_current(code)
            .map_err(|e| VaultError::InvalidAcl(format!("totp check: {e}")))
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

/// `DISCORD_WEBHOOK_URL` 환경 시 새 pending 을 알림. fire-and-forget — 실패는 tracing
/// 으로만 기록하고 vault 흐름 차단 안 함. timeout 5초.
///
/// silent error 패턴: 마스터 알림 실패가 vault 동작 자체를 차단하지 않도록.
///
/// 테스트 회피: `XGRAM_VAULT_NOTIFY=off` 환경 시 skip (CI/integration test 친화).
fn notify_pending_via_discord(id: &str, key: &str, agent: &str, action: AclAction) {
    if std::env::var("XGRAM_VAULT_NOTIFY").as_deref() == Ok("off") {
        return;
    }
    let Ok(url) = std::env::var("DISCORD_WEBHOOK_URL") else {
        return;
    };
    let body = serde_json::json!({
        "content": format!(
            "🔐 OpenXgram vault confirm 요청\n• action: {}\n• key: {}\n• agent: {}\n• id: `{}`\n• 승인: `xgram vault approve {id}`  / 거부: `xgram vault deny {id}`",
            action.as_str(),
            key,
            agent,
            id,
        ),
    });
    // fire-and-forget — 같은 스레드에서 sync POST (blocking client). 실패해도 라이즈 안 함.
    match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(client) => {
            if let Err(e) = client.post(&url).json(&body).send() {
                tracing::warn!(error = %e, id = %id, "vault pending discord notify 실패");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "discord client 생성 실패");
        }
    }
}

#[cfg(test)]
mod notify_tests {
    use super::*;

    /// 환경변수 미설정 → 즉시 return, 패닉 없음.
    #[test]
    fn notify_skips_when_env_unset() {
        // 다른 테스트가 set 했을 가능성 차단
        unsafe { std::env::remove_var("DISCORD_WEBHOOK_URL") };
        notify_pending_via_discord("id1", "k", "0xA", AclAction::Get);
    }

    /// XGRAM_VAULT_NOTIFY=off 면 url 있어도 skip.
    #[test]
    fn notify_off_overrides_url() {
        unsafe {
            std::env::set_var("DISCORD_WEBHOOK_URL", "http://127.0.0.1:1");
            std::env::set_var("XGRAM_VAULT_NOTIFY", "off");
        }
        notify_pending_via_discord("id2", "k", "0xA", AclAction::Get);
        unsafe {
            std::env::remove_var("DISCORD_WEBHOOK_URL");
            std::env::remove_var("XGRAM_VAULT_NOTIFY");
        }
    }

    /// 잘못된 url 도 panic 없이 silent error.
    #[test]
    fn notify_swallows_bad_url() {
        unsafe { std::env::set_var("DISCORD_WEBHOOK_URL", "http://127.0.0.1:1") };
        notify_pending_via_discord("id3", "k", "0xA", AclAction::Get);
        unsafe { std::env::remove_var("DISCORD_WEBHOOK_URL") };
    }
}
