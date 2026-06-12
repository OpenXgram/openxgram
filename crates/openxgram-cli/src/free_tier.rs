//! 마켓 (d)갈래 — free-tier 요금제 게이팅 (DB 백엔드).
//!
//! `openxgram_marketplace::FreeQuotaGate` trait 의 프로덕션 구현 + GUI route 가
//! 쓰는 config/usage 헬퍼.
//!
//! 모델:
//!   - `free_tier_config(agent_id, free_calls_per_day)` — 전역 기본(agent_id='*') +
//!     에이전트별 override. 가장 구체적 설정 우선 (에이전트별 > 전역).
//!   - `free_tier_usage(agent_id, day, used)` — UTC 날짜별 사용량 카운터. 날짜가 바뀌면
//!     새 row → 자동 리셋.
//!
//! 게이트 흐름 (purchase 전):
//!   1. agent 의 free_per_day 조회 (override 없으면 전역 기본).
//!   2. 오늘(UTC) used 조회. used < free_per_day 이면 used+1(원자적) 후 true 반환(무료 통과).
//!   3. 아니면 false (유료 결제 경로로).
//!
//! DB 하드코딩 금지(동적 구성): 모든 테이블/컬럼은 migration 0056 으로 생성, 런타임에
//! route/UI 로 free_calls_per_day 조절 가능.

use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use openxgram_db::{Db, DbConfig};
use openxgram_marketplace::{AgentId, FreeQuotaGate};
use rusqlite::params;
use serde::Serialize;

/// 전역 기본 설정의 sentinel agent_id.
pub const GLOBAL_AGENT: &str = "*";

/// 오늘(UTC) 날짜 문자열 'YYYY-MM-DD'.
fn today_utc() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

/// 에이전트의 유효 free_calls_per_day 조회 (override 없으면 전역 기본, 그것도 없으면 0).
fn effective_free_per_day(db: &mut Db, agent_id: &str) -> rusqlite::Result<i64> {
    let conn = db.conn();
    // 1. 에이전트별 override.
    let specific: Option<i64> = conn
        .query_row(
            "SELECT free_calls_per_day FROM free_tier_config WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .ok();
    if let Some(v) = specific {
        return Ok(v);
    }
    // 2. 전역 기본.
    let global: Option<i64> = conn
        .query_row(
            "SELECT free_calls_per_day FROM free_tier_config WHERE agent_id = ?1",
            params![GLOBAL_AGENT],
            |r| r.get(0),
        )
        .ok();
    Ok(global.unwrap_or(0))
}

/// 오늘(UTC) 사용량 조회 (없으면 0).
fn used_today(db: &mut Db, agent_id: &str) -> rusqlite::Result<i64> {
    let day = today_utc();
    let conn = db.conn();
    let used: Option<i64> = conn
        .query_row(
            "SELECT used FROM free_tier_usage WHERE agent_id = ?1 AND day = ?2",
            params![agent_id, day],
            |r| r.get(0),
        )
        .ok();
    Ok(used.unwrap_or(0))
}

/// free-tier 상태 DTO (UI 표시용).
#[derive(Debug, Serialize)]
pub struct FreeTierStatusDto {
    /// 조회 대상 agent_id.
    pub agent_id: String,
    /// 유효 1일 무료 한도 (override 또는 전역 기본).
    pub free_per_day: i64,
    /// 오늘(UTC) 사용량.
    pub used_today: i64,
    /// 오늘 남은 무료 횟수 (>= 0).
    pub remaining: i64,
    /// 이 agent 가 별도 override 를 가지는지 (false 면 전역 기본 적용).
    pub has_override: bool,
}

/// free-tier 설정 DTO (전역 기본 + 에이전트별 override 목록).
#[derive(Debug, Serialize)]
pub struct FreeTierConfigDto {
    /// 전역 기본 free_calls_per_day.
    pub global_free_per_day: i64,
    /// 에이전트별 override 항목.
    pub overrides: Vec<FreeTierOverrideDto>,
}

/// 에이전트별 override 1건.
#[derive(Debug, Serialize)]
pub struct FreeTierOverrideDto {
    /// 대상 agent_id.
    pub agent_id: String,
    /// free_calls_per_day.
    pub free_per_day: i64,
    /// 마지막 변경 시각.
    pub updated_at: String,
}

/// 상태 조회 (소비 없이) — UI 용. agent_id 생략 시 전역 기본 기준.
pub fn status(db: &mut Db, agent_id: &str) -> rusqlite::Result<FreeTierStatusDto> {
    let free_per_day = effective_free_per_day(db, agent_id)?;
    let used = used_today(db, agent_id)?;
    let has_override: bool = db
        .conn()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM free_tier_config WHERE agent_id = ?1)",
            params![agent_id],
            |r| r.get(0),
        )
        .unwrap_or(false);
    Ok(FreeTierStatusDto {
        agent_id: agent_id.to_string(),
        free_per_day,
        used_today: used,
        remaining: (free_per_day - used).max(0),
        has_override,
    })
}

/// 전역 기본 + override 목록 조회 — 설정 UI 용.
pub fn get_config(db: &mut Db) -> rusqlite::Result<FreeTierConfigDto> {
    let conn = db.conn();
    let global: i64 = conn
        .query_row(
            "SELECT free_calls_per_day FROM free_tier_config WHERE agent_id = ?1",
            params![GLOBAL_AGENT],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let mut overrides = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT agent_id, free_calls_per_day, updated_at FROM free_tier_config \
         WHERE agent_id != ?1 ORDER BY agent_id ASC",
    )?;
    let rows = stmt.query_map(params![GLOBAL_AGENT], |r| {
        Ok(FreeTierOverrideDto {
            agent_id: r.get(0)?,
            free_per_day: r.get(1)?,
            updated_at: r.get(2)?,
        })
    })?;
    for r in rows {
        overrides.push(r?);
    }
    Ok(FreeTierConfigDto {
        global_free_per_day: global,
        overrides,
    })
}

/// free_calls_per_day 설정 (전역 기본 또는 에이전트별 override). UPSERT.
/// `agent_id="*"` 이면 전역 기본. `free_per_day < 0` 은 호출자가 거부.
pub fn set_config(db: &mut Db, agent_id: &str, free_per_day: i64) -> Result<(), String> {
    if free_per_day < 0 {
        return Err(format!("free_per_day 는 0 이상 (got {free_per_day})"));
    }
    let now = Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO free_tier_config (agent_id, free_calls_per_day, updated_at) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(agent_id) DO UPDATE SET free_calls_per_day = ?2, updated_at = ?3",
            params![agent_id, free_per_day, now],
        )
        .map_err(|e| format!("free_tier_config upsert: {e}"))?;
    Ok(())
}

/// 무료 1회 소비 시도 (원자적: 잔여 확인 + used+1). 게이트 본체.
///
/// 반환: Ok(true)=무료 소비됨(과금 X), Ok(false)=무료 없음(유료로), Err=조회 실패.
pub fn try_consume(db: &mut Db, agent_id: &str) -> Result<bool, String> {
    let free_per_day =
        effective_free_per_day(db, agent_id).map_err(|e| format!("free config: {e}"))?;
    if free_per_day <= 0 {
        return Ok(false);
    }
    let day = today_utc();
    let now = Utc::now().to_rfc3339();
    let conn = db.conn();
    let tx = conn
        .transaction()
        .map_err(|e| format!("free txn begin: {e}"))?;

    // 현재 used (트랜잭션 내 — 동시 호출 직렬화는 SQLite write lock 으로).
    let used: i64 = tx
        .query_row(
            "SELECT used FROM free_tier_usage WHERE agent_id = ?1 AND day = ?2",
            params![agent_id, day],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if used >= free_per_day {
        // 무료 소진 — 유료 경로로 (used 변경 없음).
        return Ok(false);
    }

    // used+1 (UPSERT).
    tx.execute(
        "INSERT INTO free_tier_usage (agent_id, day, used, updated_at) \
         VALUES (?1, ?2, 1, ?3) \
         ON CONFLICT(agent_id, day) DO UPDATE SET used = used + 1, updated_at = ?3",
        params![agent_id, day, now],
    )
    .map_err(|e| format!("free usage upsert: {e}"))?;
    tx.commit().map_err(|e| format!("free txn commit: {e}"))?;
    Ok(true)
}

/// 마켓 (d)갈래 — DB 백엔드 FreeQuotaGate. 자체 DB 연결(같은 sqlite 파일) 보유.
/// LedgerPaymentGateway 와 동일 패턴 (WAL + busy_timeout 으로 동시 접근 안전).
pub struct LedgerFreeQuotaGate {
    db: Mutex<Db>,
}

impl LedgerFreeQuotaGate {
    /// 같은 데이터 디렉토리의 db.sqlite 에 별도 연결을 연다.
    pub fn open(db_path: PathBuf) -> anyhow::Result<Self> {
        let mut db = Db::open(DbConfig {
            path: db_path,
            ..Default::default()
        })?;
        db.migrate()?; // free_tier 테이블 존재 보장 (idempotent).
        Ok(Self { db: Mutex::new(db) })
    }
}

#[async_trait]
impl FreeQuotaGate for LedgerFreeQuotaGate {
    async fn try_consume_free(&self, agent: &AgentId) -> Result<bool, String> {
        let mut db = self.db.lock().map_err(|e| format!("free db lock: {e}"))?;
        try_consume(&mut db, agent.as_str())
    }

    async fn quota_status(&self, agent: &AgentId) -> Result<(i64, i64), String> {
        let mut db = self.db.lock().map_err(|e| format!("free db lock: {e}"))?;
        let s = status(&mut db, agent.as_str()).map_err(|e| format!("free status: {e}"))?;
        Ok((s.free_per_day, s.used_today))
    }
}
