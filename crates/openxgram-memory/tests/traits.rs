//! L4 traits 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_memory::{AgentTrait, TraitSource, TraitStore};
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
fn migration_creates_traits_table() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 5",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let _: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM traits", [], |r| r.get(0))
        .unwrap();
}

#[test]
fn insert_and_get_by_name() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = TraitStore::new(&mut db);

    let t = store
        .insert_or_update(
            "language_preference",
            "ko",
            TraitSource::Manual,
            &[],
        )
        .unwrap();
    assert_eq!(t.name, "language_preference");
    assert_eq!(t.value, "ko");
    assert_eq!(t.source, TraitSource::Manual);
    assert!(t.source_refs.is_empty());

    let got = store.get_by_name("language_preference").unwrap().unwrap();
    assert_eq!(got, t);
}

#[test]
fn upsert_updates_value_and_source() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = TraitStore::new(&mut db);

    let t1 = store
        .insert_or_update("work_hours_kst", "09-18", TraitSource::Manual, &[])
        .unwrap();
    let t2 = store
        .insert_or_update(
            "work_hours_kst",
            "10-19",
            TraitSource::Derived,
            &["pat-id-1".into(), "mem-id-2".into()],
        )
        .unwrap();
    assert_eq!(t1.id, t2.id, "같은 name → 같은 id");
    assert_eq!(t2.value, "10-19");
    assert_eq!(t2.source, TraitSource::Derived);
    assert_eq!(t2.source_refs, vec!["pat-id-1", "mem-id-2"]);
    assert!(t2.updated_at >= t1.updated_at);
}

#[test]
fn list_returns_traits_in_updated_order() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = TraitStore::new(&mut db);

    store
        .insert_or_update("a", "1", TraitSource::Manual, &[])
        .unwrap();
    store
        .insert_or_update("b", "2", TraitSource::Manual, &[])
        .unwrap();
    store
        .insert_or_update("c", "3", TraitSource::Manual, &[])
        .unwrap();
    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 3);
    let names: Vec<&str> = listed.iter().map(|t| t.name.as_str()).collect();
    // updated_at DESC — 가장 최근(c) 이 1위
    assert_eq!(names[0], "c");
}

#[test]
fn get_unknown_returns_none() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = TraitStore::new(&mut db);
    assert!(store.get_by_name("nonexistent").unwrap().is_none());
}

#[test]
fn invalid_source_string_raises() {
    assert!(TraitSource::parse("bogus").is_err());
}

#[test]
fn agent_trait_serializes_to_json() {
    let t = AgentTrait {
        id: "id-1".into(),
        name: "lang".into(),
        value: "ko".into(),
        source: TraitSource::Manual,
        source_refs: vec!["a".into()],
        created_at: chrono::Utc::now()
            .with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap()),
        updated_at: chrono::Utc::now()
            .with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap()),
    };
    let json = serde_json::to_string(&t).unwrap();
    let back: AgentTrait = serde_json::from_str(&json).unwrap();
    assert_eq!(t, back);
}
