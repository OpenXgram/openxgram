//! conversation_id: inbound 시작 시 새 ID 부여, 응답·회신은 같은 ID 묶음, 회상 가능.

use openxgram_db::{Db, DbConfig};
use openxgram_memory::{DummyEmbedder, MessageStore};
use rusqlite::params;
use tempfile::tempdir;

fn open_db(dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: dir.join("test.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let now = "2026-05-09T14:00:00+09:00";
    for sid in ["inbox-from-discord:m", "outbox-to-discord:m"] {
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, last_active, home_machine)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sid, sid, now, now, "test-host"],
            )
            .unwrap();
    }
    db
}

#[test]
fn insert_with_none_generates_fresh_conversation_id() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;
    let mut store = MessageStore::new(&mut db, &embedder);

    let m1 = store
        .insert("inbox-from-discord:m", "discord:m", "hi 1", "discord", None)
        .unwrap();
    let m2 = store
        .insert("inbox-from-discord:m", "discord:m", "hi 2", "discord", None)
        .unwrap();

    assert!(!m1.conversation_id.is_empty());
    assert!(!m2.conversation_id.is_empty());
    assert_ne!(m1.conversation_id, m2.conversation_id, "각 inbound 는 서로 다른 conversation");
}

#[test]
fn insert_with_some_reuses_conversation_id() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;
    let mut store = MessageStore::new(&mut db, &embedder);

    let inbound = store
        .insert(
            "inbox-from-discord:m",
            "discord:m",
            "리뷰 부탁",
            "discord",
            None,
        )
        .unwrap();
    let outbound = store
        .insert(
            "outbox-to-discord:m",
            "Starian",
            "확인했습니다",
            "echo",
            Some(&inbound.conversation_id),
        )
        .unwrap();

    assert_eq!(inbound.conversation_id, outbound.conversation_id);
}

#[test]
fn list_for_conversation_returns_cross_session_thread() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;

    let conv_id;
    {
        let mut store = MessageStore::new(&mut db, &embedder);
        let inbound = store
            .insert(
                "inbox-from-discord:m",
                "discord:m",
                "안녕",
                "discord",
                None,
            )
            .unwrap();
        conv_id = inbound.conversation_id.clone();
        store
            .insert(
                "outbox-to-discord:m",
                "Starian",
                "안녕하세요",
                "echo",
                Some(&conv_id),
            )
            .unwrap();
        // 다른 conversation 메시지 — 섞이면 안 됨
        store
            .insert(
                "inbox-from-discord:m",
                "discord:m",
                "다른 대화",
                "discord",
                None,
            )
            .unwrap();
    }

    let mut store = MessageStore::new(&mut db, &embedder);
    let thread = store.list_for_conversation(&conv_id).unwrap();
    assert_eq!(thread.len(), 2, "원 inbound + 응답");
    let bodies: Vec<&str> = thread.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, vec!["안녕", "안녕하세요"]);
    let sessions: Vec<&str> = thread.iter().map(|m| m.session_id.as_str()).collect();
    assert!(sessions.contains(&"inbox-from-discord:m"));
    assert!(sessions.contains(&"outbox-to-discord:m"));
}

#[test]
fn migration_backfills_existing_messages() {
    let tmp = tempdir().unwrap();
    // step 1: open DB and migrate to latest, insert message — conv_id auto-generated
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;
    {
        let mut store = MessageStore::new(&mut db, &embedder);
        store
            .insert(
                "inbox-from-discord:m",
                "discord:m",
                "legacy",
                "discord",
                None,
            )
            .unwrap();
    }
    // step 2: simulate "legacy NULL" — clear conv_id and re-run migration backfill UPDATE.
    db.conn()
        .execute("UPDATE messages SET conversation_id = NULL", [])
        .unwrap();
    db.conn()
        .execute(
            "UPDATE messages SET conversation_id = lower(hex(randomblob(16))) WHERE conversation_id IS NULL",
            [],
        )
        .unwrap();

    let null_count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(null_count, 0, "backfill 후 NULL 잔존 없음");
}
