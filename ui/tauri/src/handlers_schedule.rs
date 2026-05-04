//! Schedule invoke 핸들러 — `openxgram-orchestration::ScheduledStore` 직접 호출.

use serde::{Deserialize, Serialize};
use tauri::State;

use openxgram_orchestration::{
    kst_now_epoch, ScheduleKind, ScheduledStatus, ScheduledStore, TargetKind,
};

use crate::state::{with_db_optional, with_db_required, AppState};

#[derive(Serialize, Clone)]
pub struct ScheduleDto {
    pub id: String,
    pub target_kind: String,
    pub target: String,
    pub payload: String,
    pub msg_type: String,
    pub schedule_kind: String,
    pub schedule_value: String,
    pub status: String,
    pub created_at_kst: i64,
    pub next_due_at_kst: Option<i64>,
    pub last_error: Option<String>,
}

fn parse_target_kind(s: &str) -> Result<TargetKind, String> {
    match s {
        "role" => Ok(TargetKind::Role),
        "platform" => Ok(TargetKind::Platform),
        other => Err(format!("invalid target_kind: {other} (expect role|platform)")),
    }
}

fn parse_schedule_kind(s: &str) -> Result<ScheduleKind, String> {
    match s {
        "once" => Ok(ScheduleKind::Once),
        "cron" => Ok(ScheduleKind::Cron),
        other => Err(format!("invalid schedule_kind: {other} (expect once|cron)")),
    }
}

#[tauri::command]
pub fn schedule_list(state: State<'_, AppState>) -> Result<Vec<ScheduleDto>, String> {
    let out: Option<Vec<ScheduleDto>> = with_db_optional(&state, |db| {
        let store = ScheduledStore::new(db.conn());
        let rows = store.list(None).map_err(|e| format!("schedule list: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|m| ScheduleDto {
                id: m.id,
                target_kind: m.target_kind.as_str().to_string(),
                target: m.target,
                payload: m.payload,
                msg_type: m.msg_type,
                schedule_kind: m.schedule_kind.as_str().to_string(),
                schedule_value: m.schedule_value,
                status: m.status.as_str().to_string(),
                created_at_kst: m.created_at_kst,
                next_due_at_kst: m.next_due_at_kst,
                last_error: m.last_error,
            })
            .collect())
    })?;
    Ok(out.unwrap_or_default())
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct ScheduleCreateForm {
    pub target_kind: String,
    pub target: String,
    pub payload: String,
    pub msg_type: Option<String>,
    pub schedule_kind: String,
    pub schedule_value: String,
}

#[tauri::command]
pub fn schedule_create(
    state: State<'_, AppState>,
    target_kind: String,
    target: String,
    payload: String,
    msg_type: Option<String>,
    schedule_kind: String,
    schedule_value: String,
) -> Result<String, String> {
    let tk = parse_target_kind(&target_kind)?;
    let sk = parse_schedule_kind(&schedule_kind)?;
    let mt = msg_type.unwrap_or_else(|| "info".into());
    if target.trim().is_empty() {
        return Err("target 비어있음".into());
    }
    if payload.trim().is_empty() {
        return Err("payload 비어있음".into());
    }
    if schedule_value.trim().is_empty() {
        return Err("schedule_value 비어있음".into());
    }
    with_db_required(&state, |db| {
        let store = ScheduledStore::new(db.conn());
        store
            .insert(tk, &target, &payload, &mt, sk, &schedule_value)
            .map_err(|e| format!("schedule insert: {e}"))
    })
}

#[tauri::command]
pub fn schedule_cancel(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_db_required(&state, |db| {
        let store = ScheduledStore::new(db.conn());
        store.cancel(&id).map_err(|e| format!("schedule cancel: {e}"))
    })
}

/// 예약 통계 — 대기/완료/실패 카운트. dashboard 에 사용.
#[derive(Serialize, Clone, Default)]
pub struct ScheduleStats {
    pub pending: usize,
    pub sent: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[tauri::command]
pub fn schedule_stats(state: State<'_, AppState>) -> Result<ScheduleStats, String> {
    let out: Option<ScheduleStats> = with_db_optional(&state, |db| {
        let store = ScheduledStore::new(db.conn());
        let mut stats = ScheduleStats::default();
        for status in [
            ScheduledStatus::Pending,
            ScheduledStatus::Sent,
            ScheduledStatus::Failed,
            ScheduledStatus::Cancelled,
        ] {
            let rows = store
                .list(Some(status))
                .map_err(|e| format!("schedule list({status:?}): {e}"))?;
            match status {
                ScheduledStatus::Pending => stats.pending = rows.len(),
                ScheduledStatus::Sent => stats.sent = rows.len(),
                ScheduledStatus::Failed => stats.failed = rows.len(),
                ScheduledStatus::Cancelled => stats.cancelled = rows.len(),
            }
        }
        Ok(stats)
    })?;
    Ok(out.unwrap_or_default())
}

/// 현재 KST epoch — UI 가 next_due_at_kst 와 비교해 "곧 / 지연" 표시.
#[tauri::command]
pub fn schedule_now_kst() -> i64 {
    kst_now_epoch()
}
