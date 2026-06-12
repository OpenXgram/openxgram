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

/// 지갑 거래 원장 항목 (wallet_ledger row → UI).
#[derive(Debug, Serialize)]
pub struct LedgerEntryDto {
    pub id: String,
    pub agent_id: String,
    pub kind: String, // 'topup' | 'purchase' | 'earn'
    pub amount_micro: i64,
    pub chain: Option<String>,
    pub counterparty: Option<String>,
    pub intent_id: Option<String>,
    pub tx_ref: Option<String>,
    pub memo: Option<String>,
    pub created_at: String,
}

/// 원장 + 집계 (수익 탭·거래내역 탭 공용).
#[derive(Debug, Serialize)]
pub struct LedgerDto {
    pub entries: Vec<LedgerEntryDto>,
    /// 누적 충전 (micro).
    pub total_topup_micro: i64,
    /// 누적 구매(지출, 양수 절대값) (micro).
    pub total_purchase_micro: i64,
    /// 누적 수익 (micro).
    pub total_earned_micro: i64,
}

/// 마켓 (c)갈래 — 결제로 인한 잔액 차감 1건을 원장에 기록.
///
/// 가짜 영수증 금지: 호출 전에 LedgerPaymentGateway 가 sub_wallets 잔액을
/// 실제 검증·차감(spent_micro += amount)한 뒤 이 함수로 ledger row 만 남긴다.
#[allow(clippy::too_many_arguments)]
pub fn record_ledger(
    db: &mut Db,
    agent_id: &str,
    kind: &str,
    amount_micro: i64,
    chain: Option<&str>,
    counterparty: Option<&str>,
    intent_id: Option<&str>,
    tx_ref: Option<&str>,
    memo: Option<&str>,
) -> rusqlite::Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO wallet_ledger \
         (id, agent_id, kind, amount_micro, chain, counterparty, intent_id, tx_ref, memo, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id, agent_id, kind, amount_micro, chain, counterparty, intent_id, tx_ref, memo, now
        ],
    )?;
    Ok(id)
}

/// 원장 조회 — agent_id 옵션 (None 이면 전체), 최신순 limit.
pub fn list_ledger(
    db: &mut Db,
    agent_id: Option<&str>,
    limit: u32,
) -> rusqlite::Result<LedgerDto> {
    let conn = db.conn();
    let lim = limit.clamp(1, 500) as i64;
    let mut entries: Vec<LedgerEntryDto> = Vec::new();
    if let Some(aid) = agent_id {
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, kind, amount_micro, chain, counterparty, intent_id, tx_ref, memo, created_at \
             FROM wallet_ledger WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![aid, lim], map_ledger_row)?;
        for r in rows {
            entries.push(r?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, kind, amount_micro, chain, counterparty, intent_id, tx_ref, memo, created_at \
             FROM wallet_ledger ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![lim], map_ledger_row)?;
        for r in rows {
            entries.push(r?);
        }
    }

    // 집계 — 전체(agent 필터 무시) 또는 해당 agent 누적.
    let (topup, purchase, earned): (i64, i64, i64) = {
        let agg_sql = if agent_id.is_some() {
            "SELECT \
               COALESCE(SUM(CASE WHEN kind='topup' THEN amount_micro ELSE 0 END),0), \
               COALESCE(SUM(CASE WHEN kind='purchase' THEN -amount_micro ELSE 0 END),0), \
               COALESCE(SUM(CASE WHEN kind='earn' THEN amount_micro ELSE 0 END),0) \
             FROM wallet_ledger WHERE agent_id = ?1"
        } else {
            "SELECT \
               COALESCE(SUM(CASE WHEN kind='topup' THEN amount_micro ELSE 0 END),0), \
               COALESCE(SUM(CASE WHEN kind='purchase' THEN -amount_micro ELSE 0 END),0), \
               COALESCE(SUM(CASE WHEN kind='earn' THEN amount_micro ELSE 0 END),0) \
             FROM wallet_ledger"
        };
        if let Some(aid) = agent_id {
            conn.query_row(agg_sql, params![aid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        } else {
            conn.query_row(agg_sql, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        }
    };

    Ok(LedgerDto {
        entries,
        total_topup_micro: topup,
        total_purchase_micro: purchase,
        total_earned_micro: earned,
    })
}

fn map_ledger_row(r: &rusqlite::Row) -> rusqlite::Result<LedgerEntryDto> {
    Ok(LedgerEntryDto {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        kind: r.get(2)?,
        amount_micro: r.get(3)?,
        chain: r.get(4)?,
        counterparty: r.get(5)?,
        intent_id: r.get(6)?,
        tx_ref: r.get(7)?,
        memo: r.get(8)?,
        created_at: r.get(9)?,
    })
}

/// 마켓 (c)갈래 결제 차감 — 실제 잔액 검증 + spent_micro 증가 + 원장 기록 (원자적).
///
/// `agent_id` 서브 지갑이 없으면 에러(가짜 성공 금지). 잔액(allocated - spent + earned)이
/// 부족하면 에러. 성공 시 (intent_id, tx_ref) 반환.
pub fn debit_for_purchase(
    db: &mut Db,
    agent_id: &str,
    amount_micro: i64,
    chain: &str,
    payee: &str,
    intent_id: &str,
    memo: Option<&str>,
) -> Result<String, String> {
    if amount_micro <= 0 {
        return Err(format!("amount must be > 0 (got {amount_micro})"));
    }
    let now = Utc::now().to_rfc3339();
    let tx_ref = format!("ledger:{}", uuid::Uuid::new_v4());
    let conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|e| format!("txn begin: {e}"))?;

    // 1. 잔액 확인 — sub_wallet 존재 + balance >= amount.
    let bal: Option<i64> = tx
        .query_row(
            "SELECT allocated_micro - spent_micro + earned_micro FROM sub_wallets WHERE agent_id = ?1 AND status = 'Active'",
            params![agent_id],
            |r| r.get(0),
        )
        .ok();
    let balance = match bal {
        Some(b) => b,
        None => {
            return Err(format!(
                "sub_wallet 미존재 또는 비활성: {agent_id} (먼저 지갑 생성·충전 필요)"
            ))
        }
    };
    if balance < amount_micro {
        return Err(format!(
            "잔액 부족: balance={balance} micro < amount={amount_micro} micro"
        ));
    }

    // 2. spent_micro 증가 (잔액 차감).
    tx.execute(
        "UPDATE sub_wallets SET spent_micro = spent_micro + ?1, updated_at = ?2 WHERE agent_id = ?3",
        params![amount_micro, now, agent_id],
    )
    .map_err(|e| format!("debit update: {e}"))?;

    // 3. 원장 기록 (구매 = 음수 amount).
    let lid = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO wallet_ledger \
         (id, agent_id, kind, amount_micro, chain, counterparty, intent_id, tx_ref, memo, created_at) \
         VALUES (?1, ?2, 'purchase', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![lid, agent_id, -amount_micro, chain, payee, intent_id, tx_ref, memo, now],
    )
    .map_err(|e| format!("ledger insert: {e}"))?;

    tx.commit().map_err(|e| format!("txn commit: {e}"))?;
    Ok(tx_ref)
}

/// 마켓 (c)갈래 — 외부 사용으로 인한 수익 적립 (earned_micro 증가 + 원장).
/// (수익 UI 데모/실데이터 모두 이 경로 사용. 외부 결제 수신 배선 시 호출.)
pub fn credit_earning(
    db: &mut Db,
    agent_id: &str,
    amount_micro: i64,
    counterparty: Option<&str>,
    memo: Option<&str>,
) -> Result<String, String> {
    if amount_micro <= 0 {
        return Err(format!("amount must be > 0 (got {amount_micro})"));
    }
    let now = Utc::now().to_rfc3339();
    let conn = db.conn();
    let tx = conn.transaction().map_err(|e| format!("txn begin: {e}"))?;
    let affected = tx
        .execute(
            "UPDATE sub_wallets SET earned_micro = earned_micro + ?1, updated_at = ?2 WHERE agent_id = ?3",
            params![amount_micro, now, agent_id],
        )
        .map_err(|e| format!("earn update: {e}"))?;
    if affected != 1 {
        return Err(format!("sub_wallet 미존재: {agent_id}"));
    }
    let lid = uuid::Uuid::new_v4().to_string();
    let tx_ref = format!("ledger:{}", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO wallet_ledger \
         (id, agent_id, kind, amount_micro, counterparty, tx_ref, memo, created_at) \
         VALUES (?1, ?2, 'earn', ?3, ?4, ?5, ?6, ?7)",
        params![lid, agent_id, amount_micro, counterparty, tx_ref, memo, now],
    )
    .map_err(|e| format!("ledger insert: {e}"))?;
    tx.commit().map_err(|e| format!("txn commit: {e}"))?;
    Ok(tx_ref)
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
    // 원장 기록 (충전 = 양수 amount). 실패해도 잔액 변경은 유지(원장은 감사 보조).
    let lid = uuid::Uuid::new_v4().to_string();
    let tx_ref = format!("ledger:{}", uuid::Uuid::new_v4());
    conn.execute(
        "INSERT INTO wallet_ledger \
         (id, agent_id, kind, amount_micro, counterparty, tx_ref, memo, created_at) \
         VALUES (?1, ?2, 'topup', ?3, 'master', ?4, '마스터 → 서브 충전', ?5)",
        params![lid, body.agent_id, body.amount_micro, tx_ref, now],
    )?;
    list_one(db, &body.agent_id)
}
