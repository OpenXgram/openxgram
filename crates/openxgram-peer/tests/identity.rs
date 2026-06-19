use openxgram_db::{Db, DbConfig};
use openxgram_peer::IdentityStore;
use tempfile::TempDir;

fn fresh_db(tmp: &TempDir) -> Db {
    let cfg = DbConfig {
        path: tmp.path().join("db.sqlite"),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    db.migrate().unwrap();
    db
}

#[test]
fn test_upsert_and_resolve() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);
    let mut store = IdentityStore::new(&mut db);
    assert_eq!(store.resolve("star").unwrap(), None);
    store
        .upsert_alias("star", "0xDADA", true, "active", "2026-06-20T00:00:00+09:00")
        .unwrap();
    store
        .upsert_alias("starian", "0xDADA", false, "active", "2026-06-20T00:00:00+09:00")
        .unwrap();
    assert_eq!(store.resolve("star").unwrap(), Some("0xDADA".to_string()));
    assert_eq!(store.resolve("starian").unwrap(), Some("0xDADA".to_string()));
    store
        .upsert_alias("star", "0xBEEF", false, "active", "2026-06-20T00:00:00+09:00")
        .unwrap();
    assert_eq!(store.resolve("star").unwrap(), Some("0xBEEF".to_string()));
}

fn insert_peer(
    db: &mut openxgram_db::Db,
    alias: &str,
    eth: Option<&str>,
    sid: Option<&str>,
    role: &str,
) {
    // public_key_hex has a UNIQUE constraint, so we use the alias as a stand-in key.
    db.conn()
        .execute(
            "INSERT INTO peers (id, alias, public_key_hex, address, role, created_at, eth_address, session_identifier)
             VALUES (?1, ?2, ?2, 'http://x', ?3, '2026-06-20T00:00:00+09:00', ?4, ?5)",
            rusqlite::params![alias, alias, role, eth, sid],
        )
        .unwrap();
}

#[test]
fn test_reconcile_groups_by_session_then_address() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);
    insert_peer(&mut db, "star", Some("0xAAA"), Some("aoe_star_549029"), "primary");
    insert_peer(&mut db, "starian", Some("0xBBB"), Some("aoe_star_549029"), "worker");
    insert_peer(&mut db, "akashic", Some("0xCCC"), Some("aoe_akashic_1"), "worker");
    insert_peer(&mut db, "orphan", None, None, "worker");

    let mut store = IdentityStore::new(&mut db);
    store.reconcile("2026-06-20T00:00:00+09:00").unwrap();

    assert_eq!(store.resolve("star").unwrap(), Some("0xAAA".to_string()));
    assert_eq!(store.resolve("starian").unwrap(), Some("0xAAA".to_string()));
    assert_eq!(store.resolve("akashic").unwrap(), Some("0xCCC".to_string()));

    let status: String = db
        .conn()
        .query_row(
            "SELECT status FROM identity_aliases WHERE alias='orphan'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "quarantined");
}
