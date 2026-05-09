//! L0 → L1 reflection 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_memory::{reflect_session, DummyEmbedder, EpisodeStore, MessageStore};
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
fn reflect_empty_session_returns_none() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let ep = reflect_session(&mut db, "s1").unwrap();
    assert!(ep.is_none());
}

#[test]
fn reflect_session_creates_episode_with_correct_stats() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;

    {
        let mut store = MessageStore::new(&mut db, &embedder);
        store.insert("s1", "alice", "hello", "sig", None).unwrap();
        store.insert("s1", "bob", "hi", "sig", None).unwrap();
        store.insert("s1", "alice", "how are you", "sig", None).unwrap();
    }

    let ep = reflect_session(&mut db, "s1").unwrap().unwrap();
    assert_eq!(ep.session_id, "s1");
    assert_eq!(ep.message_count, 3);
    assert!(ep.summary.contains("3 messages"));
    assert!(ep.summary.contains("2 sender"));
    assert!(ep.started_at <= ep.ended_at);
}

#[test]
fn reflect_lists_episode_in_store() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;

    {
        let mut store = MessageStore::new(&mut db, &embedder);
        store.insert("s1", "alice", "msg1", "sig", None).unwrap();
    }

    let created = reflect_session(&mut db, "s1").unwrap().unwrap();

    let mut episodes = EpisodeStore::new(&mut db);
    let listed = episodes.list_for_session("s1").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].message_count, 1);
}

#[test]
fn reflect_multiple_calls_accumulate_episodes() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;

    {
        let mut store = MessageStore::new(&mut db, &embedder);
        store.insert("s1", "alice", "first batch", "sig", None).unwrap();
    }
    let _ep1 = reflect_session(&mut db, "s1").unwrap().unwrap();

    {
        let mut store = MessageStore::new(&mut db, &embedder);
        store.insert("s1", "alice", "second batch", "sig", None).unwrap();
    }
    let _ep2 = reflect_session(&mut db, "s1").unwrap().unwrap();

    // Phase 1: idempotent 보장 안 함, 누적. Phase 1.5 에서 boundaries 추가
    let mut episodes = EpisodeStore::new(&mut db);
    let listed = episodes.list_for_session("s1").unwrap();
    assert_eq!(listed.len(), 2);
}
