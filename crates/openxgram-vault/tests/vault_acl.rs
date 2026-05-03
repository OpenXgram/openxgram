//! Vault ACL · 일일 한도 · 감사 로그 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_vault::{AclAction, AclPolicy, VaultError, VaultStore, MASTER_AGENT};
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
fn migration_creates_acl_and_audit_tables() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 7",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn no_acl_means_non_master_denied() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("discord/bot", b"TOK", PW, &[]).unwrap();

    let err = v.get_as("discord/bot", PW, "0xAlice").unwrap_err();
    assert!(matches!(err, VaultError::AclDenied(_)));
    let msg = format!("{err}");
    assert!(msg.contains("no acl matches"));
}

#[test]
fn master_bypasses_acl() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"VAL", PW, &[]).unwrap();
    let bytes = v.get_as("k", PW, MASTER_AGENT).unwrap();
    assert_eq!(bytes, b"VAL");
}

#[test]
fn exact_acl_grants_get() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("discord/bot", b"TOK", PW, &[]).unwrap();
    v.upsert_acl(
        "discord/bot",
        "0xAlice",
        &[AclAction::Get],
        0,
        AclPolicy::Auto,
    )
    .unwrap();

    let bytes = v.get_as("discord/bot", PW, "0xAlice").unwrap();
    assert_eq!(bytes, b"TOK");
}

#[test]
fn action_not_in_allowed_actions_is_denied() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    // Get 만 허용
    v.upsert_acl("k", "0xAlice", &[AclAction::Get], 0, AclPolicy::Auto)
        .unwrap();

    let err = v.delete_as("k", "0xAlice").unwrap_err();
    assert!(format!("{err}").contains("action delete not allowed"));
}

#[test]
fn wildcard_agent_acl_matches_any_agent() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "*", &[AclAction::Get], 0, AclPolicy::Auto)
        .unwrap();

    assert_eq!(v.get_as("k", PW, "0xAlice").unwrap(), b"V");
    assert_eq!(v.get_as("k", PW, "0xBob").unwrap(), b"V");
}

#[test]
fn daily_limit_enforced_per_agent() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xAlice", &[AclAction::Get], 2, AclPolicy::Auto)
        .unwrap();

    assert_eq!(v.get_as("k", PW, "0xAlice").unwrap(), b"V");
    assert_eq!(v.get_as("k", PW, "0xAlice").unwrap(), b"V");
    let err = v.get_as("k", PW, "0xAlice").unwrap_err();
    assert!(format!("{err}").contains("daily limit exceeded"));
}

#[test]
fn audit_log_records_allowed_and_denied() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xAlice", &[AclAction::Get], 0, AclPolicy::Auto)
        .unwrap();

    // 1번 allowed, 1번 denied (액션 없음)
    v.get_as("k", PW, "0xAlice").unwrap();
    let _ = v.delete_as("k", "0xAlice"); // denied

    let conn = db.conn();
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM vault_audit", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 2);
    let allowed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM vault_audit WHERE allowed = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(allowed, 1);
}

#[test]
fn upsert_acl_replaces_existing() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.upsert_acl("k", "0xAlice", &[AclAction::Get], 5, AclPolicy::Auto)
        .unwrap();
    v.upsert_acl(
        "k",
        "0xAlice",
        &[AclAction::Get, AclAction::Set],
        10,
        AclPolicy::Confirm,
    )
    .unwrap();
    let list = v.list_acl().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].daily_limit, 10);
    assert_eq!(list[0].policy, AclPolicy::Confirm);
    assert!(list[0].allowed_actions.contains(&AclAction::Set));
}

#[test]
fn delete_acl_idempotent_only_when_present() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.upsert_acl("k", "0xAlice", &[AclAction::Get], 0, AclPolicy::Auto)
        .unwrap();
    v.delete_acl("k", "0xAlice").unwrap();
    let err = v.delete_acl("k", "0xAlice").unwrap_err();
    assert!(matches!(err, VaultError::NotFound(_)));
}
