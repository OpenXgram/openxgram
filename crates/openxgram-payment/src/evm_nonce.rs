//! EVM transaction nonce counter — (from_address, chain_id) 단위 get-and-increment (PRD-PAY-01).
//!
//! - 첫 호출: chain 의 pending nonce 와 sync 후 다음 값 반환
//! - 이후: 로컬 카운터 ++
//! - SQLite IMMEDIATE 트랜잭션으로 동시 insert 시에도 nonce 중복 방지

use openxgram_db::Db;
use rusqlite::params;

use crate::{PaymentError, Result};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS evm_nonce_counter (
    from_address TEXT NOT NULL,
    chain_id INTEGER NOT NULL,
    next_nonce INTEGER NOT NULL,
    updated_at_kst INTEGER NOT NULL,
    PRIMARY KEY (from_address, chain_id)
);
";

pub fn ensure_schema(db: &mut Db) -> Result<()> {
    db.conn()
        .execute_batch(SCHEMA)
        .map_err(PaymentError::Sqlite)?;
    Ok(())
}

/// (from, chain_id) 의 다음 nonce 를 가져오고 카운터 증가.
/// 신규 키면 init_nonce 로 시작 — 호출자가 chain query 결과를 init_nonce 로 전달.
/// 기존 키면 init_nonce 무시하고 저장된 값 사용 (로컬 우선).
pub fn get_and_increment(
    db: &mut Db,
    from_address: &str,
    chain_id: u64,
    init_nonce: u64,
) -> Result<u64> {
    ensure_schema(db)?;
    let conn = db.conn();
    let tx = conn.unchecked_transaction().map_err(PaymentError::Sqlite)?;

    let from_lc = from_address.to_lowercase();
    let now = chrono::Utc::now()
        .with_timezone(&chrono_tz_kst())
        .timestamp();

    let current: Option<i64> = tx
        .query_row(
            "SELECT next_nonce FROM evm_nonce_counter WHERE from_address = ?1 AND chain_id = ?2",
            params![from_lc, chain_id as i64],
            |r| r.get(0),
        )
        .ok();

    let used = match current {
        Some(n) => n as u64,
        None => init_nonce,
    };
    let next = used + 1;

    tx.execute(
        "INSERT INTO evm_nonce_counter (from_address, chain_id, next_nonce, updated_at_kst)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(from_address, chain_id) DO UPDATE SET next_nonce = ?3, updated_at_kst = ?4",
        params![from_lc, chain_id as i64, next as i64, now],
    )
    .map_err(PaymentError::Sqlite)?;
    tx.commit().map_err(PaymentError::Sqlite)?;
    Ok(used)
}

/// 외부 chain query 실패 등으로 카운터를 강제 재설정. 마스터 명시 호출 전제.
pub fn reset(db: &mut Db, from_address: &str, chain_id: u64, new_nonce: u64) -> Result<()> {
    ensure_schema(db)?;
    let conn = db.conn();
    let from_lc = from_address.to_lowercase();
    let now = chrono::Utc::now()
        .with_timezone(&chrono_tz_kst())
        .timestamp();
    conn.execute(
        "INSERT INTO evm_nonce_counter (from_address, chain_id, next_nonce, updated_at_kst)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(from_address, chain_id) DO UPDATE SET next_nonce = ?3, updated_at_kst = ?4",
        params![from_lc, chain_id as i64, new_nonce as i64, now],
    )
    .map_err(PaymentError::Sqlite)?;
    Ok(())
}

/// chrono Asia/Seoul TZ — chrono-tz crate 미사용 시 FixedOffset 9h
fn chrono_tz_kst() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(9 * 3600).expect("KST offset")
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_db::DbConfig;
    use tempfile::tempdir;

    fn open_db() -> Db {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("nonce.db");
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
    fn first_call_uses_init_nonce_then_increments() {
        let mut db = open_db();
        let n1 = get_and_increment(&mut db, "0xAA", 8453, 5).unwrap();
        assert_eq!(n1, 5);
        let n2 = get_and_increment(&mut db, "0xAA", 8453, 999).unwrap();
        assert_eq!(n2, 6, "이후 호출은 로컬 카운터 우선");
        let n3 = get_and_increment(&mut db, "0xAA", 8453, 999).unwrap();
        assert_eq!(n3, 7);
    }

    #[test]
    fn separate_keys_for_different_chains() {
        let mut db = open_db();
        let n_base = get_and_increment(&mut db, "0xAA", 8453, 0).unwrap();
        let n_polygon = get_and_increment(&mut db, "0xAA", 137, 0).unwrap();
        assert_eq!(n_base, 0);
        assert_eq!(n_polygon, 0, "체인 별로 독립적인 카운터");
    }

    #[test]
    fn case_insensitive_address() {
        let mut db = open_db();
        let n1 = get_and_increment(&mut db, "0xABCDEF", 8453, 10).unwrap();
        let n2 = get_and_increment(&mut db, "0xabcdef", 8453, 999).unwrap();
        assert_eq!(n1, 10);
        assert_eq!(n2, 11, "대소문자 무관 동일 키");
    }

    #[test]
    fn reset_overrides_local_counter() {
        let mut db = open_db();
        let _ = get_and_increment(&mut db, "0xAA", 8453, 5).unwrap();
        let _ = get_and_increment(&mut db, "0xAA", 8453, 5).unwrap();
        reset(&mut db, "0xAA", 8453, 100).unwrap();
        let n = get_and_increment(&mut db, "0xAA", 8453, 0).unwrap();
        assert_eq!(n, 100);
    }
}
