//! 통합 — 30초 후 도달하는 메시지가 list_due 에 노출되는지 검증.
//! (실제로 30초 sleep 하지 않고, 미래 시각으로 INSERT 후 now=future+1 로 호출)

use openxgram_db::{Db, DbConfig};
use openxgram_orchestration::{
    kst_now_epoch, ScheduleKind, ScheduledStore, ScheduledStatus, TargetKind,
};

fn temp_db() -> (tempfile::NamedTempFile, Db) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let cfg = DbConfig {
        path: tmp.path().to_path_buf(),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    db.migrate().unwrap();
    (tmp, db)
}

#[test]
fn future_message_appears_after_due() {
    let (_tmp, mut db) = temp_db();
    let store = ScheduledStore::new(db.conn());
    let now = kst_now_epoch();
    let future = now + 30;
    let iso = chrono::DateTime::from_timestamp(future, 0)
        .unwrap()
        .with_timezone(&chrono_tz::Asia::Seoul)
        .to_rfc3339();
    let id = store
        .insert(
            TargetKind::Role,
            "res",
            "ping",
            "info",
            ScheduleKind::Once,
            &iso,
        )
        .unwrap();
    // not yet
    assert!(store.list_due(now).unwrap().is_empty());
    // after 30s
    let due = store.list_due(future + 1).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, id);
    store.mark_sent(&id).unwrap();
    let after = store.get(&id).unwrap();
    assert_eq!(after.status, ScheduledStatus::Sent);
}
