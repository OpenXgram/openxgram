//! L2 memories store 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_memory::{MemoryKind, MemoryStore};
use rusqlite::params;
use tempfile::tempdir;

fn open_db(dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: dir.join("test.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();

    let now = "2026-05-03T14:00:00+09:00";
    db.conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, last_active, home_machine)
         VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["s1", "test", now, now, "test-host"],
        )
        .unwrap();
    db
}

#[test]
fn insert_and_list_by_kind() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = MemoryStore::new(&mut db);

    store
        .insert(Some("s1"), MemoryKind::Fact, "물은 100도에 끓는다")
        .unwrap();
    store
        .insert(Some("s1"), MemoryKind::Decision, "ChaCha20 사용")
        .unwrap();
    store
        .insert(None, MemoryKind::Fact, "Phase 1 마감 5월")
        .unwrap();

    let facts = store.list_by_kind(MemoryKind::Fact).unwrap();
    assert_eq!(facts.len(), 2);
    let decisions = store.list_by_kind(MemoryKind::Decision).unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].content, "ChaCha20 사용");
}

#[test]
fn pin_changes_listing_order() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = MemoryStore::new(&mut db);

    let m1 = store
        .insert(Some("s1"), MemoryKind::Reference, "first")
        .unwrap();
    let _m2 = store
        .insert(Some("s1"), MemoryKind::Reference, "second")
        .unwrap();
    let _m3 = store
        .insert(Some("s1"), MemoryKind::Reference, "third")
        .unwrap();

    // 처음에는 last_accessed 역순 (가장 최근 = third → second → first)
    let initial = store.list_by_kind(MemoryKind::Reference).unwrap();
    assert_eq!(initial[0].content, "third");

    // m1 을 pin → pinned DESC 우선이라 1위로 올라옴
    store.set_pinned(&m1.id, true).unwrap();
    let after_pin = store.list_by_kind(MemoryKind::Reference).unwrap();
    assert_eq!(after_pin[0].content, "first");
    assert!(after_pin[0].pinned);
}

#[test]
fn mark_accessed_increments_count() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = MemoryStore::new(&mut db);

    let m = store
        .insert(Some("s1"), MemoryKind::Rule, "fallback 금지")
        .unwrap();
    assert_eq!(m.access_count, 0);

    store.mark_accessed(&m.id).unwrap();
    store.mark_accessed(&m.id).unwrap();

    let listed = store.list_by_kind(MemoryKind::Rule).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].access_count, 2);
}

#[test]
fn set_pinned_unknown_id_raises() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = MemoryStore::new(&mut db);

    let err = store.set_pinned("nonexistent-uuid", true).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("affected rows"));
}

#[test]
fn parse_invalid_kind_raises() {
    let result = MemoryKind::parse("invalid");
    assert!(result.is_err());
}
