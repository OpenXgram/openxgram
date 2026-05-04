//! 다중 에이전트 메신저 허브 — Channel Dashboard.
//!
//! 1차 PR 범위: peer / schedule / notify 어댑터 상태를 통합 표시.
//! 실시간 stream (Tauri Channel API) 은 후속 — 본 PR 은 폴링 기반 status.

use serde::Serialize;
use tauri::State;

use openxgram_orchestration::{ScheduledStatus, ScheduledStore};
use openxgram_peer::PeerStore;

use openxgram_cli::notify_setup::NotifyConfig;

use crate::state::{with_db_optional, AppState};

#[derive(Serialize, Clone, Default)]
pub struct ChannelAdapterStatus {
    pub platform: String,
    pub configured: bool,
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct ChannelStatusDto {
    pub adapters: Vec<ChannelAdapterStatus>,
    pub peer_count: usize,
    pub schedule_pending: usize,
}

#[tauri::command]
pub fn channel_status(state: State<'_, AppState>) -> Result<ChannelStatusDto, String> {
    // 1) notify.toml — 어댑터 설정 여부
    let notify = NotifyConfig::load(Some(&state.data_dir))
        .map_err(|e| format!("NotifyConfig load: {e}"))?;
    let mut adapters = Vec::new();
    adapters.push(ChannelAdapterStatus {
        platform: "telegram".into(),
        configured: notify.telegram.is_some(),
        note: notify
            .telegram
            .as_ref()
            .map(|t| format!("chat_id={}", t.chat_id)),
    });
    adapters.push(ChannelAdapterStatus {
        platform: "discord".into(),
        configured: notify.discord.is_some(),
        note: notify.discord.as_ref().map(|d| {
            let mut parts = Vec::new();
            if let Some(c) = &d.channel_id {
                parts.push(format!("channel={c}"));
            }
            if d.webhook_url.is_some() {
                parts.push("webhook".into());
            }
            if parts.is_empty() {
                "(token only)".into()
            } else {
                parts.join(" + ")
            }
        }),
    });

    // 2) peer / schedule 카운트 — DB 미존재 시 0
    let counts: Option<(usize, usize)> = with_db_optional(&state, |db| {
        let mut peer_store = PeerStore::new(db);
        let peers = peer_store.list().map_err(|e| format!("peer list: {e}"))?;
        let sched_store = ScheduledStore::new(db.conn());
        let pending = sched_store
            .list(Some(ScheduledStatus::Pending))
            .map_err(|e| format!("schedule list pending: {e}"))?;
        Ok((peers.len(), pending.len()))
    })?;
    let (peer_count, schedule_pending) = counts.unwrap_or((0, 0));

    Ok(ChannelStatusDto {
        adapters,
        peer_count,
        schedule_pending,
    })
}

#[derive(Serialize, Clone)]
pub struct RecentMessageDto {
    pub source: String,
    pub summary: String,
    pub timestamp_kst: i64,
}

/// 최근 메시지 (placeholder — real-time channel stream PR 후속).
/// 현재는 schedule 의 `sent` 항목을 시간순으로 보여준다 (UI 가 비어있지 않게).
#[tauri::command]
pub fn channel_recent_messages(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<RecentMessageDto>, String> {
    let limit = limit.unwrap_or(20).min(100);
    let out: Option<Vec<RecentMessageDto>> = with_db_optional(&state, |db| {
        let store = ScheduledStore::new(db.conn());
        let mut rows = store
            .list(Some(ScheduledStatus::Sent))
            .map_err(|e| format!("schedule list sent: {e}"))?;
        rows.sort_by(|a, b| b.created_at_kst.cmp(&a.created_at_kst));
        rows.truncate(limit);
        Ok(rows
            .into_iter()
            .map(|m| RecentMessageDto {
                source: format!("{}:{}", m.target_kind.as_str(), m.target),
                summary: m.payload,
                timestamp_kst: m.created_at_kst,
            })
            .collect())
    })?;
    Ok(out.unwrap_or_default())
}
