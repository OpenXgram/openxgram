//! OpenXgram Desktop — Tauri 2.x library entry.
//!
//! 본 crate 는 5개 모듈로 invoke 핸들러를 분리해 각 도메인 별로 응집한다:
//!   - [`state`] — `AppState` + DB 헬퍼 (lazy-open, fallback 금지).
//!   - [`handlers_core`] — vault / peer / memory / payment / onboarding (10).
//!   - [`handlers_schedule`] — 예약 메시지 list/create/cancel/stats (5).
//!   - [`handlers_chain`] — 체인 메시지 list/show/create_yaml/delete/run (5).
//!   - [`handlers_notify`] — Discord/Telegram 마법사 검증·저장·테스트 (6).
//!   - [`handlers_channel`] — 다중 에이전트 허브 status / recent messages (2).
//!
//! 핸들러 정책:
//!   - DB 는 lazy-open, AppState 의 `Mutex<Option<Db>>` 로 공유.
//!   - 빈 DB 에서도 read-only 핸들러는 빈 결과 반환 (UI smoke 가능).
//!   - 쓰기 명령은 명시 raise. silent fallback 금지.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::path::Path;

pub mod daemon_client;
pub mod handlers_channel;
pub mod handlers_chain;
pub mod handlers_core;
pub mod handlers_notify;
pub mod handlers_schedule;
pub mod state;

pub use state::AppState;

pub fn run() {
    let data_dir = AppState::default_data_dir().unwrap_or_else(|e| {
        eprintln!(
            "[openxgram-desktop] data_dir 결정 실패: {e} — /tmp/openxgram-fallback 사용 (DB 미존재 → 빈 결과)"
        );
        Path::new("/tmp/openxgram-fallback").to_path_buf()
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::new(data_dir))
        .invoke_handler(tauri::generate_handler![
            // legacy subprocess wrappers
            handlers_core::get_status,
            handlers_core::get_version,
            handlers_core::dump,
            // onboarding
            handlers_core::is_initialized,
            // store-direct: vault / memory / peer / payment
            handlers_core::vault_pending_list,
            handlers_core::vault_pending_approve,
            handlers_core::vault_pending_deny,
            handlers_core::memory_search,
            handlers_core::messages_recent,
            handlers_core::peers_list,
            handlers_core::peer_add,
            handlers_core::vault_get,
            handlers_core::payment_get_daily_limit,
            handlers_core::payment_set_daily_limit,
            // schedule
            handlers_schedule::schedule_list,
            handlers_schedule::schedule_create,
            handlers_schedule::schedule_cancel,
            handlers_schedule::schedule_stats,
            handlers_schedule::schedule_now_kst,
            // chain
            handlers_chain::chain_list,
            handlers_chain::chain_show,
            handlers_chain::chain_create_yaml,
            handlers_chain::chain_delete,
            handlers_chain::chain_run,
            // notify wizards
            handlers_notify::notify_telegram_validate,
            handlers_notify::notify_telegram_detect_chat,
            handlers_notify::notify_telegram_save,
            handlers_notify::notify_discord_validate,
            handlers_notify::notify_discord_save,
            handlers_notify::notify_status,
            // channel dashboard
            handlers_channel::channel_status,
            handlers_channel::channel_recent_messages,
        ])
        .run(tauri::generate_context!())
        .expect("OpenXgram desktop 실행 실패");
}

// ───────────────────────────── unit tests (DB-free) ──────────────────────────

#[cfg(test)]
mod tests {
    use super::state::*;

    #[test]
    fn appstate_with_db_optional_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::new(tmp.path().to_path_buf());
        let result: Result<Option<bool>, String> = with_db_optional(&state, |_db| Ok(true));
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn appstate_with_db_required_raises_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::new(tmp.path().to_path_buf());
        let result: Result<bool, String> = with_db_required(&state, |_db| Ok(true));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("DB 파일 미존재"));
    }

    #[test]
    fn appstate_is_data_initialized_false_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::new(tmp.path().to_path_buf());
        assert!(!is_data_initialized(&state));
    }
}
