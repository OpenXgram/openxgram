//! Vault 통합 테스트 — set/get/list/delete + 암호화 round-trip.

use openxgram_db::{Db, DbConfig};
use openxgram_vault::{VaultError, VaultStore};
use tempfile::tempdir;

const PW: &str = "vault-test-password-12345";

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
fn migration_creates_vault_table() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 6",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn set_then_get_round_trip() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = VaultStore::new(&mut db);

    let secret = b"DISCORD_BOT_TOKEN=abc123";
    store
        .set(
            "discord/bot",
            secret,
            PW,
            &["discord".into(), "prod".into()],
        )
        .unwrap();

    let got = store.get("discord/bot", PW).unwrap();
    assert_eq!(got, secret);
}

#[test]
fn get_with_wrong_password_raises() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = VaultStore::new(&mut db);
    store.set("k", b"v", PW, &[]).unwrap();

    let err = store.get("k", "wrong-password-99").unwrap_err();
    matches!(err, VaultError::Keystore(_))
        .then_some(())
        .expect("InvalidPassword 또는 keystore raise");
}

#[test]
fn upsert_overwrites_value() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = VaultStore::new(&mut db);
    store.set("k", b"v1", PW, &[]).unwrap();
    store.set("k", b"v2", PW, &[]).unwrap();
    assert_eq!(store.get("k", PW).unwrap(), b"v2");
}

#[test]
fn list_returns_metadata_only() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = VaultStore::new(&mut db);
    store.set("a", b"1", PW, &["t1".into()]).unwrap();
    store.set("b", b"2", PW, &[]).unwrap();
    let entries = store.list().unwrap();
    assert_eq!(entries.len(), 2);
    let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
    assert!(keys.contains(&"a"));
    assert!(keys.contains(&"b"));
}

#[test]
fn get_unknown_key_raises_not_found() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = VaultStore::new(&mut db);
    let err = store.get("nonexistent", PW).unwrap_err();
    assert!(matches!(err, VaultError::NotFound(_)));
}

#[test]
fn delete_removes_entry() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = VaultStore::new(&mut db);
    store.set("k", b"v", PW, &[]).unwrap();
    store.delete("k").unwrap();
    let err = store.get("k", PW).unwrap_err();
    assert!(matches!(err, VaultError::NotFound(_)));
}

#[test]
fn delete_unknown_raises_not_found() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = VaultStore::new(&mut db);
    let err = store.delete("nonexistent").unwrap_err();
    assert!(matches!(err, VaultError::NotFound(_)));
}
