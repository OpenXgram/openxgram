//! 1.11 자율 트리거 e2e — `ensure_default_self_cron` + `poll_self_trigger` 흐름 검증.
//! - default cron 등록 idempotent
//! - 도래한 SelfTrigger 가 inbox-from-self:<target> 세션에 inject 되고 status=sent (cron) 후 다음 next_due 재계산

use openxgram_cli::agent::{ensure_default_self_cron, poll_self_trigger};
use openxgram_db::{Db, DbConfig};
use openxgram_orchestration::{
    kst_now_epoch, ScheduleKind, ScheduledStatus, ScheduledStore, TargetKind,
};
use tempfile::tempdir;

fn open_db(data_dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: data_dir.join("xgram.db"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    db
}

#[test]
fn default_self_cron_is_idempotent() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    ensure_default_self_cron(dir).unwrap();
    ensure_default_self_cron(dir).unwrap(); // 두 번 호출해도 한 번만 등록

    let mut db = open_db(dir);
    let store = ScheduledStore::new(db.conn());
    let all = store.list(None).unwrap();
    let self_briefing: Vec<_> = all
        .iter()
        .filter(|m| {
            matches!(m.target_kind, TargetKind::SelfTrigger) && m.target == "morning-briefing"
        })
        .collect();
    assert_eq!(self_briefing.len(), 1, "기본 cron 단 1개만 등록");
    assert!(matches!(self_briefing[0].schedule_kind, ScheduleKind::Cron));
}

#[tokio::test]
async fn poll_self_trigger_injects_due_message_and_marks_sent() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    let mut db = open_db(dir);

    // 과거 시각 once 예약 → 즉시 due
    let id = {
        let store = ScheduledStore::new(db.conn());
        store
            .insert(
                TargetKind::SelfTrigger,
                "test-target",
                "ping body",
                "info",
                ScheduleKind::Once,
                "2000-01-01T09:00:00+09:00",
            )
            .unwrap()
    };
    drop(db);

    let count = poll_self_trigger(dir).await.unwrap();
    assert_eq!(count, 1, "1개 inject 됨");

    let mut db = open_db(dir);
    // 1) status=sent
    let after = ScheduledStore::new(db.conn()).get(&id).unwrap();
    assert_eq!(after.status, ScheduledStatus::Sent);

    // 2) inbox-from-self:test-target 세션에 메시지 1개
    let conn = db.conn();
    let session_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE title = ?1",
            ["inbox-from-self:test-target"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(session_count, 1, "self inbox 세션 생성");

    let msg_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages
             WHERE session_id = (SELECT id FROM sessions WHERE title = ?1)
               AND sender = 'self:test-target' AND body = 'ping body'",
            ["inbox-from-self:test-target"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(msg_count, 1, "self inbox 메시지 저장");
}

#[tokio::test]
async fn poll_self_trigger_cron_reschedules_after_fire() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path();
    let mut db = open_db(dir);

    // cron: 매분 0초 → list_due 가 즉시 잡도록 next_due_at_kst 를 강제로 과거로 백데이트.
    let id = {
        let store = ScheduledStore::new(db.conn());
        store
            .insert(
                TargetKind::SelfTrigger,
                "tick-test",
                "tick",
                "info",
                ScheduleKind::Cron,
                "0 * * * *", // 매시 정각
            )
            .unwrap()
    };
    db.conn()
        .execute(
            "UPDATE scheduled_messages SET next_due_at_kst = ?1 WHERE id = ?2",
            rusqlite::params![kst_now_epoch() - 60, id],
        )
        .unwrap();
    drop(db);

    let count = poll_self_trigger(dir).await.unwrap();
    assert_eq!(count, 1);

    let mut db = open_db(dir);
    let after = ScheduledStore::new(db.conn()).get(&id).unwrap();
    // cron 은 sent 후에도 status=Pending, next_due 가 미래로 재계산
    assert_eq!(after.status, ScheduledStatus::Pending);
    assert!(
        after.next_due_at_kst.unwrap() > kst_now_epoch(),
        "next_due 가 미래로 재계산"
    );
}
