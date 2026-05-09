//! 예약 메시지 — `once` 또는 `cron` 으로 미래에 전송되는 메시지.
//!
//! 모든 시간은 KST (Asia/Seoul) 기준 epoch seconds.

use std::str::FromStr;

use chrono::{DateTime, TimeZone};
use chrono_tz::Asia::Seoul;
use cron::Schedule as CronSchedule;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{OrchestrationError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetKind {
    Role,
    Platform,
    /// 자기 자신에게 보내는 트리거 — agent 가 inbox-from-self:<target> 세션으로 inject.
    /// 자율 행동 (매일 정리 / periodic 재검토 등) 의 시작점.
    SelfTrigger,
}

impl TargetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::Platform => "platform",
            Self::SelfTrigger => "self",
        }
    }
}

impl FromStr for TargetKind {
    type Err = OrchestrationError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "role" => Ok(Self::Role),
            "platform" => Ok(Self::Platform),
            "self" => Ok(Self::SelfTrigger),
            other => Err(OrchestrationError::InvalidTargetKind(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleKind {
    Once,
    Cron,
}

impl ScheduleKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Cron => "cron",
        }
    }
}

impl FromStr for ScheduleKind {
    type Err = OrchestrationError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "once" => Ok(Self::Once),
            "cron" => Ok(Self::Cron),
            other => Err(OrchestrationError::InvalidScheduleKind(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduledStatus {
    Pending,
    Sent,
    Failed,
    Cancelled,
}

impl ScheduledStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for ScheduledStatus {
    type Err = OrchestrationError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "sent" => Ok(Self::Sent),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(OrchestrationError::InvalidStatus(other.to_string())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScheduledMessage {
    pub id: String,
    pub target_kind: TargetKind,
    pub target: String,
    pub payload: String,
    pub msg_type: String,
    pub schedule_kind: ScheduleKind,
    pub schedule_value: String,
    pub status: ScheduledStatus,
    pub created_at_kst: i64,
    pub last_sent_at_kst: Option<i64>,
    pub last_error: Option<String>,
    pub next_due_at_kst: Option<i64>,
    pub audit_row_id: Option<String>,
}

/// 현재 KST epoch seconds.
pub fn kst_now_epoch() -> i64 {
    chrono::Utc::now().with_timezone(&Seoul).timestamp()
}

/// ISO8601 (timezone 포함 또는 KST 가정) 을 epoch seconds 로 파싱.
pub fn parse_iso_kst(s: &str) -> Result<i64> {
    // 1) RFC3339/ISO8601 with offset
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp());
    }
    // 2) Naive — assume KST
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        let dt = Seoul
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(|| OrchestrationError::InvalidDateTime(format!("ambiguous KST: {s}")))?;
        return Ok(dt.timestamp());
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        let dt = Seoul
            .from_local_datetime(&naive)
            .single()
            .ok_or_else(|| OrchestrationError::InvalidDateTime(format!("ambiguous KST: {s}")))?;
        return Ok(dt.timestamp());
    }
    Err(OrchestrationError::InvalidDateTime(s.to_string()))
}

/// cron 표현식 (KST 해석) 의 다음 실행 시각을 epoch seconds 로 반환.
///
/// `cron` 크레이트는 6-field (sec min hour dom month dow) 와 7-field 를 받음.
/// 5-field 입력 (`0 9 * * *`) 도 6-field (`0 0 9 * * *`) 로 normalize.
pub fn compute_next_due_kst(cron_expr: &str, now_epoch: i64) -> Result<Option<i64>> {
    let normalized = normalize_cron(cron_expr);
    let schedule = CronSchedule::from_str(&normalized)
        .map_err(|e| OrchestrationError::InvalidCron(format!("{cron_expr}: {e}")))?;
    let now = Seoul
        .timestamp_opt(now_epoch, 0)
        .single()
        .ok_or_else(|| OrchestrationError::InvalidDateTime(format!("epoch {now_epoch}")))?;
    Ok(schedule.after(&now).next().map(|dt| dt.timestamp()))
}

fn normalize_cron(expr: &str) -> String {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    match parts.len() {
        5 => format!("0 {expr}"), // add seconds=0
        6 | 7 => expr.to_string(),
        _ => expr.to_string(),
    }
}

pub struct ScheduledStore<'a> {
    conn: &'a Connection,
}

impl<'a> ScheduledStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// 새 예약 메시지 INSERT. `next_due_at_kst` 자동 계산.
    /// 반환: 생성된 row id.
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &self,
        target_kind: TargetKind,
        target: &str,
        payload: &str,
        msg_type: &str,
        schedule_kind: ScheduleKind,
        schedule_value: &str,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = kst_now_epoch();
        let next_due = match schedule_kind {
            ScheduleKind::Once => Some(parse_iso_kst(schedule_value)?),
            ScheduleKind::Cron => compute_next_due_kst(schedule_value, now)?,
        };
        self.conn.execute(
            "INSERT INTO scheduled_messages \
             (id, target_kind, target, payload, msg_type, schedule_kind, schedule_value, \
              status, created_at_kst, next_due_at_kst) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?9)",
            params![
                id,
                target_kind.as_str(),
                target,
                payload,
                msg_type,
                schedule_kind.as_str(),
                schedule_value,
                now,
                next_due
            ],
        )?;
        Ok(id)
    }

    pub fn list(&self, status_filter: Option<ScheduledStatus>) -> Result<Vec<ScheduledMessage>> {
        let (sql, params): (&str, Vec<rusqlite::types::Value>) = match status_filter {
            Some(s) => (
                "SELECT id, target_kind, target, payload, msg_type, schedule_kind, schedule_value, \
                 status, created_at_kst, last_sent_at_kst, last_error, next_due_at_kst, audit_row_id \
                 FROM scheduled_messages WHERE status = ?1 ORDER BY created_at_kst",
                vec![rusqlite::types::Value::Text(s.as_str().to_string())],
            ),
            None => (
                "SELECT id, target_kind, target, payload, msg_type, schedule_kind, schedule_value, \
                 status, created_at_kst, last_sent_at_kst, last_error, next_due_at_kst, audit_row_id \
                 FROM scheduled_messages ORDER BY created_at_kst",
                vec![],
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), row_to_msg)?;
        rows.map(|r| r.map_err(OrchestrationError::from))
            .collect::<Result<Vec<_>>>()
    }

    pub fn list_pending(&self) -> Result<Vec<ScheduledMessage>> {
        self.list(Some(ScheduledStatus::Pending))
    }

    /// `now_epoch_kst` 시점에 발사 가능한 (status=pending, next_due<=now) 메시지.
    pub fn list_due(&self, now_epoch_kst: i64) -> Result<Vec<ScheduledMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, target_kind, target, payload, msg_type, schedule_kind, schedule_value, \
             status, created_at_kst, last_sent_at_kst, last_error, next_due_at_kst, audit_row_id \
             FROM scheduled_messages \
             WHERE status = 'pending' AND next_due_at_kst IS NOT NULL AND next_due_at_kst <= ?1 \
             ORDER BY next_due_at_kst",
        )?;
        let rows = stmt.query_map(params![now_epoch_kst], row_to_msg)?;
        rows.map(|r| r.map_err(OrchestrationError::from))
            .collect::<Result<Vec<_>>>()
    }

    /// 전송 성공 처리.
    /// - `Once` → status='sent', last_sent_at 갱신, next_due_at=NULL
    /// - `Cron` → status remains 'pending', next_due_at 다음 시각으로 갱신
    pub fn mark_sent(&self, id: &str) -> Result<()> {
        let msg = self.get(id)?;
        let now = kst_now_epoch();
        match msg.schedule_kind {
            ScheduleKind::Once => {
                self.conn.execute(
                    "UPDATE scheduled_messages \
                     SET status='sent', last_sent_at_kst=?1, next_due_at_kst=NULL, last_error=NULL \
                     WHERE id=?2",
                    params![now, id],
                )?;
            }
            ScheduleKind::Cron => {
                let next = compute_next_due_kst(&msg.schedule_value, now)?;
                self.conn.execute(
                    "UPDATE scheduled_messages \
                     SET last_sent_at_kst=?1, next_due_at_kst=?2, last_error=NULL \
                     WHERE id=?3",
                    params![now, next, id],
                )?;
            }
        }
        Ok(())
    }

    pub fn mark_failed(&self, id: &str, reason: &str) -> Result<()> {
        let now = kst_now_epoch();
        self.conn.execute(
            "UPDATE scheduled_messages \
             SET status='failed', last_error=?1, last_sent_at_kst=?2 \
             WHERE id=?3",
            params![reason, now, id],
        )?;
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        let affected = self.conn.execute(
            "UPDATE scheduled_messages SET status='cancelled', next_due_at_kst=NULL WHERE id=?1",
            params![id],
        )?;
        if affected == 0 {
            return Err(OrchestrationError::ScheduledNotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<ScheduledMessage> {
        self.conn
            .query_row(
                "SELECT id, target_kind, target, payload, msg_type, schedule_kind, schedule_value, \
                 status, created_at_kst, last_sent_at_kst, last_error, next_due_at_kst, audit_row_id \
                 FROM scheduled_messages WHERE id=?1",
                params![id],
                row_to_msg,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    OrchestrationError::ScheduledNotFound(id.to_string())
                }
                other => OrchestrationError::Db(other),
            })
    }
}

fn row_to_msg(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledMessage> {
    let target_kind_str: String = row.get(1)?;
    let schedule_kind_str: String = row.get(5)?;
    let status_str: String = row.get(7)?;
    Ok(ScheduledMessage {
        id: row.get(0)?,
        target_kind: target_kind_str.parse().map_err(|e: OrchestrationError| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?,
        target: row.get(2)?,
        payload: row.get(3)?,
        msg_type: row.get(4)?,
        schedule_kind: schedule_kind_str.parse().map_err(|e: OrchestrationError| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?,
        schedule_value: row.get(6)?,
        status: status_str.parse().map_err(|e: OrchestrationError| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
        })?,
        created_at_kst: row.get(8)?,
        last_sent_at_kst: row.get(9)?,
        last_error: row.get(10)?,
        next_due_at_kst: row.get(11)?,
        audit_row_id: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_db::{Db, DbConfig};

    fn open_db() -> Db {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let cfg = DbConfig {
            path: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let mut db = Db::open(cfg).unwrap();
        db.migrate().unwrap();
        // keep tmp alive via leak (test scope)
        std::mem::forget(tmp);
        db
    }

    #[test]
    fn cron_normalize_5_to_6() {
        let next = compute_next_due_kst("0 9 * * *", kst_now_epoch()).unwrap();
        assert!(next.is_some());
    }

    #[test]
    fn iso_kst_with_offset() {
        let ts = parse_iso_kst("2099-01-01T09:00:00+09:00").unwrap();
        // 2099-01-01 00:00:00 UTC = 4070908800
        assert_eq!(ts, 4070908800);
    }

    #[test]
    fn iso_naive_kst() {
        let ts = parse_iso_kst("2099-01-01T09:00:00").unwrap();
        assert_eq!(ts, 4070908800);
    }

    #[test]
    fn insert_once_round_trip() {
        let mut db = open_db();
        let store = ScheduledStore::new(db.conn());
        let id = store
            .insert(
                TargetKind::Role,
                "res",
                "morning briefing",
                "info",
                ScheduleKind::Once,
                "2099-01-01T09:00:00+09:00",
            )
            .unwrap();
        let msg = store.get(&id).unwrap();
        assert_eq!(msg.target, "res");
        assert_eq!(msg.status, ScheduledStatus::Pending);
        assert_eq!(msg.next_due_at_kst, Some(4070908800));
    }

    #[test]
    fn list_due_returns_only_past() {
        let mut db = open_db();
        let store = ScheduledStore::new(db.conn());
        let _future = store
            .insert(
                TargetKind::Role,
                "res",
                "future",
                "info",
                ScheduleKind::Once,
                "2099-01-01T09:00:00+09:00",
            )
            .unwrap();
        let past = store
            .insert(
                TargetKind::Role,
                "res",
                "past",
                "info",
                ScheduleKind::Once,
                "2000-01-01T09:00:00+09:00",
            )
            .unwrap();
        let due = store.list_due(kst_now_epoch()).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, past);
    }

    #[test]
    fn mark_sent_once_terminal() {
        let mut db = open_db();
        let store = ScheduledStore::new(db.conn());
        let id = store
            .insert(
                TargetKind::Role,
                "res",
                "p",
                "info",
                ScheduleKind::Once,
                "2000-01-01T09:00:00+09:00",
            )
            .unwrap();
        store.mark_sent(&id).unwrap();
        let msg = store.get(&id).unwrap();
        assert_eq!(msg.status, ScheduledStatus::Sent);
        assert_eq!(msg.next_due_at_kst, None);
    }

    #[test]
    fn mark_sent_cron_reschedules() {
        let mut db = open_db();
        let store = ScheduledStore::new(db.conn());
        let id = store
            .insert(
                TargetKind::Platform,
                "discord:123",
                "standup",
                "info",
                ScheduleKind::Cron,
                "0 9 * * *",
            )
            .unwrap();
        store.mark_sent(&id).unwrap();
        let msg = store.get(&id).unwrap();
        assert_eq!(msg.status, ScheduledStatus::Pending);
        assert!(msg.next_due_at_kst.unwrap() > kst_now_epoch());
    }

    #[test]
    fn mark_failed_records_error() {
        let mut db = open_db();
        let store = ScheduledStore::new(db.conn());
        let id = store
            .insert(
                TargetKind::Role,
                "res",
                "p",
                "info",
                ScheduleKind::Once,
                "2000-01-01T09:00:00+09:00",
            )
            .unwrap();
        store.mark_failed(&id, "network error").unwrap();
        let msg = store.get(&id).unwrap();
        assert_eq!(msg.status, ScheduledStatus::Failed);
        assert_eq!(msg.last_error.as_deref(), Some("network error"));
    }

    #[test]
    fn cancel_unknown_errors() {
        let mut db = open_db();
        let store = ScheduledStore::new(db.conn());
        let err = store.cancel("does-not-exist").unwrap_err();
        assert!(matches!(err, OrchestrationError::ScheduledNotFound(_)));
    }

    #[test]
    fn invalid_cron_rejected() {
        let err = compute_next_due_kst("not a cron", kst_now_epoch()).unwrap_err();
        assert!(matches!(err, OrchestrationError::InvalidCron(_)));
    }
}
