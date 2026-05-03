//! Peer registry 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_peer::{PeerError, PeerRole, PeerStore};
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
fn migration_creates_peers_table() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 10",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn add_then_get_round_trip() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PeerStore::new(&mut db);
    let p = store
        .add(
            "mac-mini",
            "0303030303030303030303030303030303030303030303030303030303030303",
            "http://192.168.1.10:7300",
            PeerRole::Secondary,
            Some("home server"),
        )
        .unwrap();
    assert_eq!(p.alias, "mac-mini");
    assert_eq!(p.role, PeerRole::Secondary);
    assert_eq!(p.notes.as_deref(), Some("home server"));
    assert!(p.last_seen.is_none());

    let by_alias = store.get_by_alias("mac-mini").unwrap().unwrap();
    assert_eq!(by_alias.id, p.id);
}

#[test]
fn duplicate_alias_raises() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PeerStore::new(&mut db);
    store
        .add(
            "alpha",
            "11".repeat(33).as_str(),
            "http://x:1",
            PeerRole::Worker,
            None,
        )
        .unwrap();
    let err = store
        .add(
            "alpha",
            "22".repeat(33).as_str(),
            "http://x:2",
            PeerRole::Worker,
            None,
        )
        .unwrap_err();
    // unique 충돌 → sqlite error
    assert!(matches!(err, PeerError::Sqlite(_)));
}

#[test]
fn list_returns_creation_order() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PeerStore::new(&mut db);
    for (i, alias) in ["a", "b", "c"].iter().enumerate() {
        store
            .add(
                alias,
                format!("{:02x}", i + 1).repeat(33).as_str(),
                "http://x",
                PeerRole::Worker,
                None,
            )
            .unwrap();
    }
    let list = store.list().unwrap();
    let aliases: Vec<&str> = list.iter().map(|p| p.alias.as_str()).collect();
    assert_eq!(aliases, vec!["a", "b", "c"]);
}

#[test]
fn touch_updates_last_seen() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PeerStore::new(&mut db);
    store
        .add(
            "x",
            "ab".repeat(33).as_str(),
            "http://x",
            PeerRole::Worker,
            None,
        )
        .unwrap();
    assert!(store
        .get_by_alias("x")
        .unwrap()
        .unwrap()
        .last_seen
        .is_none());
    store.touch("x").unwrap();
    assert!(store
        .get_by_alias("x")
        .unwrap()
        .unwrap()
        .last_seen
        .is_some());
}

#[test]
fn delete_removes_peer() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PeerStore::new(&mut db);
    store
        .add(
            "to-delete",
            "cd".repeat(33).as_str(),
            "http://x",
            PeerRole::Worker,
            None,
        )
        .unwrap();
    store.delete("to-delete").unwrap();
    assert!(store.get_by_alias("to-delete").unwrap().is_none());
    let err = store.delete("to-delete").unwrap_err();
    assert!(matches!(err, PeerError::NotFound(_)));
}

#[test]
fn add_with_eth_address_and_touch_by_eth() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PeerStore::new(&mut db);
    let p = store
        .add_with_eth(
            "with-eth",
            &"42".repeat(33),
            "http://x",
            Some("0xDeadBeef000000000000000000000000DEadBEEf"),
            PeerRole::Worker,
            None,
        )
        .unwrap();
    assert_eq!(
        p.eth_address.as_deref(),
        Some("0xDeadBeef000000000000000000000000DEadBEEf")
    );
    // 매칭 1
    let n = store
        .touch_by_eth_address("0xDeadBeef000000000000000000000000DEadBEEf")
        .unwrap();
    assert_eq!(n, 1);
    let after = store.get_by_alias("with-eth").unwrap().unwrap();
    assert!(after.last_seen.is_some());
    // 미등록 주소 — 0
    let n = store.touch_by_eth_address("0xunknown").unwrap();
    assert_eq!(n, 0);
}

#[test]
fn get_by_public_key_works() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PeerStore::new(&mut db);
    let pk = "ef".repeat(33);
    store
        .add("p", &pk, "http://x", PeerRole::Primary, None)
        .unwrap();
    let p = store.get_by_public_key(&pk).unwrap().unwrap();
    assert_eq!(p.alias, "p");
    assert_eq!(p.role, PeerRole::Primary);
}
