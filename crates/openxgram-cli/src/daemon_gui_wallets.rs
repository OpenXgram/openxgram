//! UI-MESSENGER-SPEC v1.3 §2.4 + M-3 + M-6 + L4 + S6 — 서브 지갑.
//!
//! 마스터(🔑 신원) + 세션별 서브(메신저). HD derivation.
//! L4: derivation_index 영구 점유 — Decommissioned 도 재사용 X.
//! S6: daily_limit = LLM 토큰비 + x402 합산.
//!
//! 안티패턴 10 준수: 직접 SQL — 단, openxgram-db crate 내에 별도 store
//! crate 만드는 게 정석이지만 MVP 는 daemon_gui 모듈 내부 SQL 사용.

use chrono::Utc;
use openxgram_db::Db;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct SubWalletDto {
    pub agent_id: String,
    pub derivation_index: u32,
    pub derived_address: String,
    pub allocated_micro: i64,
    pub spent_micro: i64,
    pub earned_micro: i64,
    pub balance_micro: i64,
    pub daily_limit_micro: i64,
    pub monthly_limit_micro: i64,
    pub auto_approve_below_micro: i64,
    pub auto_topup_enabled: bool,
    pub auto_topup_threshold_micro: i64,
    pub auto_topup_amount_micro: i64,
    pub auto_topup_max_per_day_micro: i64,
    pub auto_topup_consumed_today_micro: i64,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct MasterWalletDto {
    pub address: Option<String>,
    pub free_micro: i64,
    pub last_synced_at: String,
}

#[derive(Debug, Serialize)]
pub struct WalletsDto {
    pub master: MasterWalletDto,
    pub sub_wallets: Vec<SubWalletDto>,
    pub next_hd_index: u32, // L4: 가장 큰 derivation_index + 1
}

#[derive(Debug, Deserialize)]
pub struct CreateSubWalletBody {
    pub agent_id: String,
    /// 옵션. 미지정 시 자동 할당 (max+1, hd_index_history 도 확인 L4).
    pub derivation_index: Option<u32>,
    /// 옵션. 미지정 시 데모용 deterministic 주소 생성.
    pub derived_address: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TopupBody {
    pub agent_id: String,
    pub amount_micro: i64,
}

pub fn list_wallets(db: &mut Db) -> rusqlite::Result<WalletsDto> {
    let conn = db.conn();
    // 마스터 view
    let master = conn
        .query_row(
            "SELECT master_address, free_micro, last_synced_at FROM master_wallet_view WHERE id = 1",
            [],
            |r| {
                Ok(MasterWalletDto {
                    address: r.get(0)?,
                    free_micro: r.get(1)?,
                    last_synced_at: r.get(2)?,
                })
            },
        )
        .unwrap_or(MasterWalletDto {
            address: None,
            free_micro: 0,
            last_synced_at: "".into(),
        });

    let mut stmt = conn.prepare(
        "SELECT agent_id, derivation_index, derived_address, allocated_micro, spent_micro, \
                earned_micro, daily_limit_micro, monthly_limit_micro, auto_approve_below_micro, \
                auto_topup_enabled, auto_topup_threshold_micro, auto_topup_amount_micro, \
                auto_topup_max_per_day_micro, auto_topup_consumed_today_micro, status, \
                created_at, updated_at \
         FROM sub_wallets ORDER BY derivation_index ASC",
    )?;
    let rows: Vec<SubWalletDto> = stmt
        .query_map([], |r| {
            let allocated_micro: i64 = r.get(3)?;
            let spent_micro: i64 = r.get(4)?;
            let earned_micro: i64 = r.get(5)?;
            let auto_topup_enabled_i: i64 = r.get(9)?;
            Ok(SubWalletDto {
                agent_id: r.get(0)?,
                derivation_index: r.get::<_, i64>(1)? as u32,
                derived_address: r.get(2)?,
                allocated_micro,
                spent_micro,
                earned_micro,
                balance_micro: allocated_micro - spent_micro + earned_micro,
                daily_limit_micro: r.get(6)?,
                monthly_limit_micro: r.get(7)?,
                auto_approve_below_micro: r.get(8)?,
                auto_topup_enabled: auto_topup_enabled_i != 0,
                auto_topup_threshold_micro: r.get(10)?,
                auto_topup_amount_micro: r.get(11)?,
                auto_topup_max_per_day_micro: r.get(12)?,
                auto_topup_consumed_today_micro: r.get(13)?,
                status: r.get(14)?,
                created_at: r.get(15)?,
                updated_at: r.get(16)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // L4: 다음 인덱스 = max(sub_wallets, hd_index_history) + 1
    let next: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(idx), -1) + 1 FROM ( \
                SELECT derivation_index AS idx FROM sub_wallets \
                UNION ALL \
                SELECT derivation_index AS idx FROM hd_index_history \
             )",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    Ok(WalletsDto {
        master,
        sub_wallets: rows,
        next_hd_index: next.max(0) as u32,
    })
}

pub fn create_sub_wallet(db: &mut Db, body: CreateSubWalletBody) -> rusqlite::Result<SubWalletDto> {
    // L4 영구 점유: derivation_index 가 sub_wallets 또는 hd_index_history 에 이미
    // 있으면 거부.
    let conn = db.conn();
    // 자동 할당 시 next_hd_index 사용.
    let next_idx: i64 = conn.query_row(
        "SELECT COALESCE(MAX(idx), -1) + 1 FROM ( \
            SELECT derivation_index AS idx FROM sub_wallets \
            UNION ALL \
            SELECT derivation_index AS idx FROM hd_index_history \
         )",
        [],
        |r| r.get(0),
    )?;
    let idx: i64 = body.derivation_index.map(|i| i as i64).unwrap_or(next_idx);
    // 이미 점유 여부 확인
    let occupied: i64 = conn.query_row(
        "SELECT COUNT(*) FROM ( \
            SELECT derivation_index FROM sub_wallets WHERE derivation_index = ?1 \
            UNION ALL \
            SELECT derivation_index FROM hd_index_history WHERE derivation_index = ?1 \
         )",
        params![idx],
        |r| r.get(0),
    )?;
    if occupied > 0 {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some(format!("derivation_index {idx} 이미 점유 (L4 영구 — 재사용 X)")),
        ));
    }
    let address = body
        .derived_address
        .unwrap_or_else(|| format!("0xDEMO{:08x}{}", idx, &body.agent_id[..6.min(body.agent_id.len())]));
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO sub_wallets (agent_id, derivation_index, derived_address, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![body.agent_id, idx, address, now],
    )?;
    conn.execute(
        "INSERT INTO hd_index_history (derivation_index, agent_id, derived_address, occupied_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![idx, body.agent_id, address, now],
    )?;
    list_one(db, &body.agent_id)
}

fn list_one(db: &mut Db, agent_id: &str) -> rusqlite::Result<SubWalletDto> {
    let conn = db.conn();
    conn.query_row(
        "SELECT agent_id, derivation_index, derived_address, allocated_micro, spent_micro, \
                earned_micro, daily_limit_micro, monthly_limit_micro, auto_approve_below_micro, \
                auto_topup_enabled, auto_topup_threshold_micro, auto_topup_amount_micro, \
                auto_topup_max_per_day_micro, auto_topup_consumed_today_micro, status, \
                created_at, updated_at \
         FROM sub_wallets WHERE agent_id = ?1",
        params![agent_id],
        |r| {
            let allocated_micro: i64 = r.get(3)?;
            let spent_micro: i64 = r.get(4)?;
            let earned_micro: i64 = r.get(5)?;
            let auto_topup_enabled_i: i64 = r.get(9)?;
            Ok(SubWalletDto {
                agent_id: r.get(0)?,
                derivation_index: r.get::<_, i64>(1)? as u32,
                derived_address: r.get(2)?,
                allocated_micro,
                spent_micro,
                earned_micro,
                balance_micro: allocated_micro - spent_micro + earned_micro,
                daily_limit_micro: r.get(6)?,
                monthly_limit_micro: r.get(7)?,
                auto_approve_below_micro: r.get(8)?,
                auto_topup_enabled: auto_topup_enabled_i != 0,
                auto_topup_threshold_micro: r.get(10)?,
                auto_topup_amount_micro: r.get(11)?,
                auto_topup_max_per_day_micro: r.get(12)?,
                auto_topup_consumed_today_micro: r.get(13)?,
                status: r.get(14)?,
                created_at: r.get(15)?,
                updated_at: r.get(16)?,
            })
        },
    )
}

/// V8 — 마스터 → 서브 즉시 이체 (인라인 모달).
pub fn topup(db: &mut Db, body: TopupBody) -> rusqlite::Result<SubWalletDto> {
    let conn = db.conn();
    let now = Utc::now().to_rfc3339();
    // 마스터 free 차감 (음수 허용 안 함은 향후)
    conn.execute(
        "UPDATE master_wallet_view SET free_micro = free_micro - ?1, last_synced_at = ?2 WHERE id = 1",
        params![body.amount_micro, now],
    )?;
    // 서브 allocated 증가
    conn.execute(
        "UPDATE sub_wallets SET allocated_micro = allocated_micro + ?1, updated_at = ?2 WHERE agent_id = ?3",
        params![body.amount_micro, now, body.agent_id],
    )?;
    list_one(db, &body.agent_id)
}
