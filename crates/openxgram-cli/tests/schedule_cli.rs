//! `xgram schedule` / `xgram chain` 통합 — DB round-trip + clap parsing.

use openxgram_cli::orchestration::{ChainAction, ScheduleAction};
use openxgram_db::{Db, DbConfig};
use openxgram_orchestration::{
    kst_now_epoch, ChainStore, ScheduledStatus, ScheduledStore,
};
use std::path::Path;
use tempfile::tempdir;

fn setup_data_dir() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    // ensure the openxgram tree exists
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    dir
}

fn db_for(data_dir: &Path) -> Db {
    let path = openxgram_core::paths::db_path(data_dir);
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    db
}

#[test]
fn schedule_once_round_trip() {
    let dir = setup_data_dir();
    openxgram_cli::orchestration::run_schedule(
        dir.path(),
        ScheduleAction::Once {
            at: "2099-01-01T09:00:00+09:00".to_string(),
            to_role: Some("res".to_string()),
            to_platform: None,
            channel_id: None,
            text: "hi".to_string(),
            msg_type: "info".to_string(),
        },
    )
    .unwrap();

    let mut db = db_for(dir.path());
    let store = ScheduledStore::new(db.conn());
    let pending = store.list_pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].target, "res");
    assert_eq!(pending[0].status, ScheduledStatus::Pending);
}

#[test]
fn schedule_cron_then_list_then_cancel() {
    let dir = setup_data_dir();
    openxgram_cli::orchestration::run_schedule(
        dir.path(),
        ScheduleAction::Cron {
            cron_expr: "0 9 * * *".to_string(),
            to_role: None,
            to_platform: Some("discord".to_string()),
            channel_id: Some("12345".to_string()),
            text: "standup".to_string(),
            msg_type: "info".to_string(),
        },
    )
    .unwrap();
    // list should run without panic
    openxgram_cli::orchestration::run_schedule(
        dir.path(),
        ScheduleAction::List { status: None },
    )
    .unwrap();
    // cancel by reading id
    let id = {
        let mut db = db_for(dir.path());
        let store = ScheduledStore::new(db.conn());
        store.list_pending().unwrap()[0].id.clone()
    };
    openxgram_cli::orchestration::run_schedule(
        dir.path(),
        ScheduleAction::Cancel { id: id.clone() },
    )
    .unwrap();
    let mut db = db_for(dir.path());
    let store = ScheduledStore::new(db.conn());
    let msg = store.get(&id).unwrap();
    assert_eq!(msg.status, ScheduledStatus::Cancelled);
}

#[test]
fn schedule_run_pending_drains_due() {
    let dir = setup_data_dir();
    // insert past-time directly
    {
        let mut db = db_for(dir.path());
        let store = ScheduledStore::new(db.conn());
        store
            .insert(
                openxgram_orchestration::TargetKind::Role,
                "res",
                "due",
                "info",
                openxgram_orchestration::ScheduleKind::Once,
                "2000-01-01T09:00:00+09:00",
            )
            .unwrap();
    }
    openxgram_cli::orchestration::run_schedule(dir.path(), ScheduleAction::RunPending).unwrap();
    let mut db = db_for(dir.path());
    let store = ScheduledStore::new(db.conn());
    let due = store.list_due(kst_now_epoch()).unwrap();
    assert!(due.is_empty(), "due should be drained after RunPending");
}

#[test]
fn chain_create_show_run_delete() {
    let dir = setup_data_dir();
    let yaml = r#"
name: morning-routine
description: morning standup
steps:
  - to_role: master
    text: "오늘 일정 어떠세요?"
    delay_secs: 0
  - to_role: res
    text: "오늘 뉴스 요약 부탁"
    delay_secs: 0
  - to_platform: discord
    channel_id: "12345"
    text: "✓ standup 시작"
    delay_secs: 0
"#;
    let yaml_path = dir.path().join("chain.yaml");
    std::fs::write(&yaml_path, yaml).unwrap();
    openxgram_cli::orchestration::run_chain(
        dir.path(),
        ChainAction::Create {
            file: yaml_path.clone(),
        },
    )
    .unwrap();
    {
        let mut db = db_for(dir.path());
        let store = ChainStore::new(db.conn());
        let (_chain, steps) = store.get_by_name("morning-routine").unwrap();
        assert_eq!(steps.len(), 3);
    }
    openxgram_cli::orchestration::run_chain(dir.path(), ChainAction::List).unwrap();
    openxgram_cli::orchestration::run_chain(
        dir.path(),
        ChainAction::Show {
            name: "morning-routine".to_string(),
        },
    )
    .unwrap();
    // Run with NoopSender — no failure
    openxgram_cli::orchestration::run_chain(
        dir.path(),
        ChainAction::Run {
            name: "morning-routine".to_string(),
        },
    )
    .unwrap();
    openxgram_cli::orchestration::run_chain(
        dir.path(),
        ChainAction::Delete {
            name: "morning-routine".to_string(),
        },
    )
    .unwrap();
    let mut db = db_for(dir.path());
    let store = ChainStore::new(db.conn());
    assert!(store.list().unwrap().is_empty());
}
