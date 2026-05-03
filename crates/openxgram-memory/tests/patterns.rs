//! L3 patterns 분류기 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_memory::{Classification, PatternStore};
use tempfile::tempdir;

fn open_db(dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: dir.join("test.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    db
}

#[test]
fn classification_thresholds() {
    assert_eq!(Classification::from_frequency(1), Classification::New);
    assert_eq!(Classification::from_frequency(2), Classification::Recurring);
    assert_eq!(Classification::from_frequency(4), Classification::Recurring);
    assert_eq!(Classification::from_frequency(5), Classification::Routine);
    assert_eq!(Classification::from_frequency(100), Classification::Routine);
}

#[test]
fn observe_creates_then_increments_frequency() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PatternStore::new(&mut db);

    let p1 = store.observe("아침에 차를 마신다").unwrap();
    assert_eq!(p1.frequency, 1);
    assert_eq!(p1.classification, Classification::New);

    let p2 = store.observe("아침에 차를 마신다").unwrap();
    assert_eq!(p2.frequency, 2);
    assert_eq!(p2.classification, Classification::Recurring);
    assert_eq!(p1.id, p2.id, "같은 pattern → 같은 id (upsert)");

    // 5 회로 routine 진입
    for _ in 2..5 {
        store.observe("아침에 차를 마신다").unwrap();
    }
    let p5 = store.observe("아침에 차를 마신다").unwrap();
    assert_eq!(p5.frequency, 6);
    assert_eq!(p5.classification, Classification::Routine);
}

#[test]
fn list_by_classification_buckets_correctly() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PatternStore::new(&mut db);

    // new: 1회
    store.observe("새로운 패턴").unwrap();
    // recurring: 3회
    for _ in 0..3 {
        store.observe("반복").unwrap();
    }
    // routine: 6회
    for _ in 0..6 {
        store.observe("일상").unwrap();
    }

    assert_eq!(
        store
            .list_by_classification(Classification::New)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .list_by_classification(Classification::Recurring)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .list_by_classification(Classification::Routine)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn migration_creates_patterns_table() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 4",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "0004 patterns 마이그레이션 적용");
    let _: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM patterns", [], |r| r.get(0))
        .unwrap();
}
