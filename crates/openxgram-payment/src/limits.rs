//! Payment daily limit store (PRD §16) — agent 별 / chain 별 microUSDC 한도.
//!
//! **배경**
//!   PR #93 의 Tauri payment_get/set_daily_limit 핸들러는 vault_acl row
//!   (key_pattern="payment.usdc.transfer", agent="default") 의 daily_limit
//!   컬럼을 의미적으로 재사용했다. ACL 권한과 결제 한도는 다른 관심사 —
//!   별도 store/table 로 정식 분리한다 (마이그레이션 0015).
//!
//! **단위**
//!   - microUSDC (1 USDC = 1_000_000 micro)
//!   - 0 = 한도 미설정 → 의미적으로 "결제 차단"
//!
//! **시간대**
//!   - updated_at_kst 는 KST(Asia/Seoul) RFC3339 문자열 (CLAUDE.md 규칙).

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::Db;
use serde::{Deserialize, Serialize};

use crate::{PaymentError, Result};

/// 단일 (agent_id, chain_id) row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyLimit {
    pub id: i64,
    pub agent_id: String,
    pub chain_id: String,
    pub daily_micro: i64,
    pub updated_at_kst: DateTime<FixedOffset>,
}

/// 결제 한도 store. ACL 과 무관 — payment 전용.
pub struct DailyLimitStore<'a> {
    db: &'a mut Db,
}

impl<'a> DailyLimitStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// (agent_id, chain_id) 한도 조회. row 미존재 → Ok(None).
    /// 호출자가 0 / None 둘 중 어느 의미인지 명시적으로 처리하도록 Option 반환.
    pub fn get(&mut self, agent_id: &str, chain_id: &str) -> Result<Option<DailyLimit>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, agent_id, chain_id, daily_micro, updated_at_kst
             FROM payment_daily_limits
             WHERE agent_id = ?1 AND chain_id = ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![agent_id, chain_id])?;
        if let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let agent_id: String = row.get(1)?;
            let chain_id: String = row.get(2)?;
            let daily_micro: i64 = row.get(3)?;
            let updated_at: String = row.get(4)?;
            let updated_at_kst = DateTime::parse_from_rfc3339(&updated_at)
                .map_err(|e| PaymentError::InvalidTimestamp(format!("{updated_at}: {e}")))?;
            Ok(Some(DailyLimit {
                id,
                agent_id,
                chain_id,
                daily_micro,
                updated_at_kst,
            }))
        } else {
            Ok(None)
        }
    }

    /// (agent_id, chain_id) 한도 upsert. daily_micro 음수 → InvalidAmount.
    /// updated_at_kst 는 호출 시점 KST.
    pub fn set(&mut self, agent_id: &str, chain_id: &str, daily_micro: i64) -> Result<DailyLimit> {
        if daily_micro < 0 {
            return Err(PaymentError::InvalidAmount(format!(
                "daily_micro must be >= 0 (got {daily_micro})"
            )));
        }
        let now = kst_now();
        let now_rfc = now.to_rfc3339();
        let affected = self.db.conn().execute(
            "INSERT INTO payment_daily_limits (agent_id, chain_id, daily_micro, updated_at_kst)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, chain_id)
             DO UPDATE SET daily_micro = excluded.daily_micro,
                           updated_at_kst = excluded.updated_at_kst",
            rusqlite::params![agent_id, chain_id, daily_micro, now_rfc],
        )?;
        // silent error 방지 — INSERT 또는 UPDATE 정확히 1 row.
        if affected != 1 {
            return Err(PaymentError::InvalidState(format!(
                "upsert affected {affected} rows (expected 1)"
            )));
        }
        // re-read 로 정확한 row 반환 (id 포함).
        self.get(agent_id, chain_id)?
            .ok_or_else(|| PaymentError::NotFound(format!("{agent_id}/{chain_id} after upsert")))
    }

    /// 모든 한도 row 나열. UI / dump 용도.
    pub fn list_all(&mut self) -> Result<Vec<DailyLimit>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, agent_id, chain_id, daily_micro, updated_at_kst
             FROM payment_daily_limits
             ORDER BY agent_id, chain_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let agent_id: String = row.get(1)?;
            let chain_id: String = row.get(2)?;
            let daily_micro: i64 = row.get(3)?;
            let updated_at: String = row.get(4)?;
            Ok((id, agent_id, chain_id, daily_micro, updated_at))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, agent_id, chain_id, daily_micro, updated_at) = r?;
            let updated_at_kst = DateTime::parse_from_rfc3339(&updated_at)
                .map_err(|e| PaymentError::InvalidTimestamp(format!("{updated_at}: {e}")))?;
            out.push(DailyLimit {
                id,
                agent_id,
                chain_id,
                daily_micro,
                updated_at_kst,
            });
        }
        Ok(out)
    }
}

// ─────────────────────────────── tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_db::DbConfig;
    use tempfile::tempdir;

    fn open_db(dir: &std::path::Path) -> Db {
        let mut db = Db::open(DbConfig {
            path: dir.join("test.sqlite"),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        db
    }

    #[test]
    fn migration_creates_payment_daily_limits_table() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 15",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn get_missing_returns_none() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        assert!(store.get("default", "base").unwrap().is_none());
    }

    #[test]
    fn set_then_get_roundtrip() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        let row = store.set("default", "base", 5_000_000).unwrap();
        assert_eq!(row.agent_id, "default");
        assert_eq!(row.chain_id, "base");
        assert_eq!(row.daily_micro, 5_000_000);

        let fetched = store.get("default", "base").unwrap().unwrap();
        assert_eq!(fetched.daily_micro, 5_000_000);
        assert_eq!(fetched.id, row.id);
    }

    #[test]
    fn set_upsert_updates_existing_row() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        let r1 = store.set("default", "base", 1_000_000).unwrap();
        let r2 = store.set("default", "base", 9_999_999).unwrap();
        assert_eq!(r1.id, r2.id, "same (agent, chain) must reuse row id");
        assert_eq!(r2.daily_micro, 9_999_999);
        assert!(r2.updated_at_kst >= r1.updated_at_kst);

        let list = store.list_all().unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn negative_daily_micro_rejected() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        let err = store.set("default", "base", -1).unwrap_err();
        assert!(matches!(err, PaymentError::InvalidAmount(_)));
    }

    #[test]
    fn zero_daily_micro_allowed_means_blocked() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        let row = store.set("default", "base", 0).unwrap();
        assert_eq!(row.daily_micro, 0);
    }

    #[test]
    fn list_all_orders_by_agent_then_chain() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        store.set("zeta", "polygon", 100).unwrap();
        store.set("alpha", "ethereum", 200).unwrap();
        store.set("alpha", "base", 300).unwrap();
        let list = store.list_all().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].agent_id, "alpha");
        assert_eq!(list[0].chain_id, "base");
        assert_eq!(list[1].agent_id, "alpha");
        assert_eq!(list[1].chain_id, "ethereum");
        assert_eq!(list[2].agent_id, "zeta");
    }

    #[test]
    fn separate_agents_isolated() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        store.set("agent_a", "base", 111).unwrap();
        store.set("agent_b", "base", 222).unwrap();
        assert_eq!(
            store.get("agent_a", "base").unwrap().unwrap().daily_micro,
            111
        );
        assert_eq!(
            store.get("agent_b", "base").unwrap().unwrap().daily_micro,
            222
        );
    }

    #[test]
    fn separate_chains_isolated() {
        let tmp = tempdir().unwrap();
        let mut db = open_db(tmp.path());
        let mut store = DailyLimitStore::new(&mut db);
        store.set("default", "base", 111).unwrap();
        store.set("default", "polygon", 222).unwrap();
        assert_eq!(
            store.get("default", "base").unwrap().unwrap().daily_micro,
            111
        );
        assert_eq!(
            store
                .get("default", "polygon")
                .unwrap()
                .unwrap()
                .daily_micro,
            222
        );
    }
}
