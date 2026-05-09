//! SelfTrigger TargetKind: cron 등록 → list_due 출현 → mark_sent 후 재예약.

use openxgram_db::{Db, DbConfig};
use openxgram_orchestration::{
    kst_now_epoch, ScheduleKind, ScheduledStatus, ScheduledStore, TargetKind,
};

fn open_db() -> Db {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().to_path_buf(),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    db.migrate().unwrap();
    std::mem::forget(tmp);
    db
}

#[test]
fn self_trigger_cron_round_trip() {
    let mut db = open_db();
    let store = ScheduledStore::new(db.conn());
    let id = store
        .insert(
            TargetKind::SelfTrigger,
            "morning-briefing",
            "오늘 작업 정리",
            "info",
            ScheduleKind::Cron,
            "0 9 * * *",
        )
        .unwrap();
    let msg = store.get(&id).unwrap();
    assert!(matches!(msg.target_kind, TargetKind::SelfTrigger));
    assert_eq!(msg.target, "morning-briefing");
    assert_eq!(msg.target_kind.as_str(), "self");
    assert_eq!(msg.status, ScheduledStatus::Pending);
}

#[test]
fn self_trigger_once_due_then_sent() {
    let mut db = open_db();
    let store = ScheduledStore::new(db.conn());
    let id = store
        .insert(
            TargetKind::SelfTrigger,
            "kick",
            "ping",
            "info",
            ScheduleKind::Once,
            "2000-01-01T09:00:00+09:00",
        )
        .unwrap();
    let due = store.list_due(kst_now_epoch()).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, id);
    assert!(matches!(due[0].target_kind, TargetKind::SelfTrigger));

    store.mark_sent(&id).unwrap();
    let after = store.get(&id).unwrap();
    assert_eq!(after.status, ScheduledStatus::Sent);
    assert_eq!(after.next_due_at_kst, None);
}

#[test]
fn parse_self_target_kind_string() {
    let tk: TargetKind = "self".parse().unwrap();
    assert!(matches!(tk, TargetKind::SelfTrigger));
}
