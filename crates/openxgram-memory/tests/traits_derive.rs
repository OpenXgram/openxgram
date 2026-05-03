//! L3 → L4 자동 도출 (`derive_traits_from_patterns`) 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_memory::{
    derive_traits_from_patterns, pattern_to_trait_name, PatternStore, TraitSource, TraitStore,
    DERIVED_TRAIT_PREFIX,
};
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
fn pattern_name_uses_prefix() {
    let n = pattern_to_trait_name("morning greeting");
    assert!(n.starts_with(DERIVED_TRAIT_PREFIX));
    assert!(n.ends_with("morning greeting"));
}

#[test]
fn derive_skips_when_no_routine() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    // 1회 observe = NEW, 2회 = RECURRING — 5+ 가 아니므로 derive 안 됨
    PatternStore::new(&mut db).observe("hello").unwrap();
    PatternStore::new(&mut db).observe("hello").unwrap();
    let derived = derive_traits_from_patterns(&mut db).unwrap();
    assert!(derived.is_empty());
}

#[test]
fn derive_creates_trait_for_routine_pattern() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    // 5회 observe → ROUTINE
    for _ in 0..5 {
        PatternStore::new(&mut db).observe("daily standup").unwrap();
    }
    let derived = derive_traits_from_patterns(&mut db).unwrap();
    assert_eq!(derived.len(), 1);
    let t = &derived[0];
    assert_eq!(t.source, TraitSource::Derived);
    assert_eq!(t.value, "daily standup");
    assert!(t.name.starts_with(DERIVED_TRAIT_PREFIX));
    assert_eq!(t.source_refs.len(), 1);

    // list 에서도 보여야 함
    let listed = TraitStore::new(&mut db).list().unwrap();
    assert!(listed.iter().any(|x| x.name == t.name));
}

#[test]
fn derive_is_idempotent() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    for _ in 0..5 {
        PatternStore::new(&mut db).observe("x").unwrap();
    }
    let first = derive_traits_from_patterns(&mut db).unwrap();
    let second = derive_traits_from_patterns(&mut db).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(first[0].name, second[0].name);
    // 전체 trait 개수도 1 — 중복 row 없음
    let listed = TraitStore::new(&mut db).list().unwrap();
    assert_eq!(
        listed
            .iter()
            .filter(|t| t.name.starts_with(DERIVED_TRAIT_PREFIX))
            .count(),
        1
    );
}

#[test]
fn derive_preserves_manual_traits() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    // manual trait 등록 (다른 prefix)
    TraitStore::new(&mut db)
        .insert_or_update("tone", "concise", TraitSource::Manual, &[])
        .unwrap();
    // ROUTINE pattern 도출
    for _ in 0..5 {
        PatternStore::new(&mut db).observe("daily review").unwrap();
    }
    derive_traits_from_patterns(&mut db).unwrap();

    let listed = TraitStore::new(&mut db).list().unwrap();
    let manual = listed.iter().find(|t| t.name == "tone").unwrap();
    assert_eq!(manual.source, TraitSource::Manual);
    assert_eq!(manual.value, "concise"); // 변경되지 않음
    assert_eq!(listed.len(), 2);
}
