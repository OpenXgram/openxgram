//! HD derivation index + KEK 회전 (PRD-ROT-01, 02, 03).
//!
//! 흐름:
//!   1. current_index() — 현재 활성 KEK 의 derivation index
//!   2. next() — 다음 N 산출 + 회전 기록 row INSERT (rotated_at_kst, retired_at_kst NULL)
//!   3. retire(N, grace_days=7) — old N 의 retired_at 설정 (read-only 유예 시작)
//!   4. zeroize_expired() — retired_at + 7일 경과 시 audit insert + secret 메모리 zeroize
//!
//! KEK 자체는 master m/44'/0'/0'/0/N 에서 derive — keystore::derive_keypair 위임.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::{kst_now, kst_offset};
use openxgram_db::Db;
use rusqlite::params;

use crate::audit_chain::{AuditEntry, chain_hash, next_seq_and_prev};

#[derive(Debug, thiserror::Error)]
pub enum RotationError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("audit chain: {0}")]
    AuditChain(#[from] crate::audit_chain::AuditChainError),
    #[error("not found: derivation_index={0}")]
    NotFound(u32),
}

pub type Result<T> = std::result::Result<T, RotationError>;

pub const GRACE_DAYS: i64 = 7;

#[derive(Debug, Clone)]
pub struct KekRotation {
    pub id: i64,
    pub derivation_index: u32,
    pub rotated_at: DateTime<FixedOffset>,
    pub retired_at: Option<DateTime<FixedOffset>>,
    pub audit_row_id: Option<String>,
}

fn ts_to_dt(ts: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp(ts, 0)
        .expect("valid ts")
        .with_timezone(&kst_offset())
}

/// 현재 활성 KEK 의 derivation index — retired_at 가 NULL 인 가장 최근 row.
/// 없으면 None (= 첫 회전 전).
pub fn current_index(db: &mut Db) -> Result<Option<u32>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT derivation_index FROM vault_kek_rotations
         WHERE retired_at_kst IS NULL ORDER BY rotated_at_kst DESC LIMIT 1",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| Some(n as u32))
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(RotationError::Sqlite(e)),
    })
}

/// 다음 N 산출 — 전체 row 의 MAX(derivation_index) + 1 (없으면 0). retired 와 무관 — N 은 단조 증가.
/// audit_row_id 는 PRD-ROT-03 의 KEK_ROTATE_START 이벤트 id (호출자가 audit insert 후 전달).
pub fn next(db: &mut Db, audit_row_id: Option<&str>) -> Result<u32> {
    let conn = db.conn();
    let max_n: Option<i64> = conn
        .query_row(
            "SELECT MAX(derivation_index) FROM vault_kek_rotations",
            [],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let next_n = max_n.map(|n| n as u32 + 1).unwrap_or(0);
    let conn = db.conn();
    let now_kst = kst_now().timestamp();
    conn.execute(
        "INSERT INTO vault_kek_rotations (derivation_index, rotated_at_kst, audit_row_id)
         VALUES (?1, ?2, ?3)",
        params![next_n as i64, now_kst, audit_row_id],
    )?;
    Ok(next_n)
}

/// 특정 N 의 retired_at 설정 — 7일 read-only 유예 시작.
pub fn retire(db: &mut Db, derivation_index: u32) -> Result<()> {
    let conn = db.conn();
    let now_kst = kst_now().timestamp();
    let n = conn.execute(
        "UPDATE vault_kek_rotations SET retired_at_kst = ?1 WHERE derivation_index = ?2",
        params![now_kst, derivation_index as i64],
    )?;
    if n == 0 {
        return Err(RotationError::NotFound(derivation_index));
    }
    Ok(())
}

/// retired_at + GRACE_DAYS 초과한 row 들을 반환 — zeroize 대상.
/// 호출자가 secret 을 zeroize 한 후 audit row 추가 (PRD-ROT-03 KEK_ROTATE_ZEROIZE).
pub fn list_expired(db: &mut Db) -> Result<Vec<KekRotation>> {
    let conn = db.conn();
    let cutoff = kst_now().timestamp() - GRACE_DAYS * 24 * 3600;
    let mut stmt = conn.prepare(
        "SELECT id, derivation_index, rotated_at_kst, retired_at_kst, audit_row_id
         FROM vault_kek_rotations
         WHERE retired_at_kst IS NOT NULL AND retired_at_kst <= ?1",
    )?;
    let rows = stmt.query_map(params![cutoff], |r| {
        Ok(KekRotation {
            id: r.get(0)?,
            derivation_index: r.get::<_, i64>(1)? as u32,
            rotated_at: ts_to_dt(r.get(2)?),
            retired_at: r.get::<_, Option<i64>>(3)?.map(ts_to_dt),
            audit_row_id: r.get(4)?,
        })
    })?;
    Ok(rows.filter_map(|x| x.ok()).collect())
}

/// audit chain 에 회전 이벤트 row 자동 추가 (PRD-ROT-03).
/// reason 은 "KEK_ROTATE_START" / "KEK_ROTATE_COMMIT" / "KEK_ROTATE_ZEROIZE" 중 하나.
pub fn record_rotation_event(
    db: &mut Db,
    event: KekRotateEvent,
    derivation_index: u32,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let ts = kst_now().to_rfc3339();
    let key = format!("kek/index/{derivation_index}");
    let agent = "master";
    let action = "rotate";
    let reason = event.as_str();

    let entry = AuditEntry {
        id: id.clone(),
        key: key.clone(),
        agent: agent.to_string(),
        action: action.to_string(),
        allowed: true,
        reason: Some(reason.to_string()),
        timestamp: ts.clone(),
    };
    let (seq, prev) = next_seq_and_prev(db)?;
    let h = chain_hash(&prev, &entry);

    let conn = db.conn();
    conn.execute(
        "INSERT INTO vault_audit (id, key, agent, action, allowed, reason, timestamp, prev_hash, entry_hash, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            key,
            agent,
            action,
            1i64,
            Some(reason),
            ts,
            &prev[..],
            &h[..],
            seq,
        ],
    )?;
    Ok(id)
}

#[derive(Debug, Clone, Copy)]
pub enum KekRotateEvent {
    Start,
    Commit,
    Zeroize,
}

impl KekRotateEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "KEK_ROTATE_START",
            Self::Commit => "KEK_ROTATE_COMMIT",
            Self::Zeroize => "KEK_ROTATE_ZEROIZE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit_chain::verify_chain;
    use openxgram_db::DbConfig;
    use tempfile::tempdir;

    fn open_db() -> Db {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("rot.db");
        let mut db = Db::open(DbConfig {
            path,
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        std::mem::forget(tmp);
        db
    }

    #[test]
    fn first_rotation_returns_index_zero() {
        let mut db = open_db();
        assert_eq!(current_index(&mut db).unwrap(), None);
        let n = next(&mut db, None).unwrap();
        assert_eq!(n, 0);
        assert_eq!(current_index(&mut db).unwrap(), Some(0));
    }

    #[test]
    fn second_rotation_increments_index() {
        let mut db = open_db();
        let n0 = next(&mut db, None).unwrap();
        retire(&mut db, n0).unwrap();
        let n1 = next(&mut db, None).unwrap();
        assert_eq!(n0, 0);
        assert_eq!(n1, 1);
        assert_eq!(current_index(&mut db).unwrap(), Some(1));
    }

    #[test]
    fn retire_unknown_index_errors() {
        let mut db = open_db();
        let err = retire(&mut db, 999);
        assert!(matches!(err, Err(RotationError::NotFound(999))));
    }

    #[test]
    fn rotation_event_appends_to_chain_and_verifies() {
        let mut db = open_db();
        let _ = record_rotation_event(&mut db, KekRotateEvent::Start, 0).unwrap();
        let _ = record_rotation_event(&mut db, KekRotateEvent::Commit, 0).unwrap();
        verify_chain(&mut db).unwrap();
    }

    #[test]
    fn list_expired_empty_before_grace() {
        let mut db = open_db();
        let n = next(&mut db, None).unwrap();
        retire(&mut db, n).unwrap();
        // grace 7일 미경과 → empty
        let xs = list_expired(&mut db).unwrap();
        assert!(xs.is_empty());
    }
}
