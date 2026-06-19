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
