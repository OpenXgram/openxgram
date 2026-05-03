//! L0 messages insert + 임베딩 + sqlite-vec KNN 회상 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_memory::{DummyEmbedder, Embedder, MessageStore, EMBED_DIM};
use rusqlite::params;
use tempfile::tempdir;

fn open_db(dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: dir.join("test.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();

    // sessions 테이블에 sample 세션 (FK 제약 충족)
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
fn dummy_embedder_is_deterministic_and_normalized() {
    let e = DummyEmbedder;
    let v1 = e.embed("hello world");
    let v2 = e.embed("hello world");
    assert_eq!(v1, v2);
    assert_eq!(v1.len(), EMBED_DIM);

    // L2 norm ≈ 1
    let norm: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4, "norm={norm}");

    let v3 = e.embed("different text");
    assert_ne!(v1, v3);
}

#[test]
fn insert_and_recall_self_returns_distance_zero() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;

    {
        let mut store = MessageStore::new(&mut db, &embedder);
        store.insert("s1", "alice", "hello world", "sig").unwrap();
        store.insert("s1", "bob", "foo bar baz", "sig").unwrap();
        store.insert("s1", "alice", "openxgram memory", "sig").unwrap();
    }

    let mut store = MessageStore::new(&mut db, &embedder);
    let hits = store.recall_top_k("hello world", 3).unwrap();

    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].message.body, "hello world", "자기 자신이 1 위");
    assert!(hits[0].distance < 1e-4, "self distance ≈ 0");
    // 정렬 검증
    assert!(hits[0].distance <= hits[1].distance);
    assert!(hits[1].distance <= hits[2].distance);
}

#[test]
fn recall_top_k_limits_results() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let embedder = DummyEmbedder;

    {
        let mut store = MessageStore::new(&mut db, &embedder);
        for i in 0..10 {
            store
                .insert("s1", "alice", &format!("message {i}"), "sig")
                .unwrap();
        }
    }

    let mut store = MessageStore::new(&mut db, &embedder);
    let hits = store.recall_top_k("query", 3).unwrap();
    assert_eq!(hits.len(), 3);
}

#[test]
fn migrations_create_message_embeddings_tables() {
    let tmp = tempdir().unwrap();
    let mut db = Db::open(DbConfig {
        path: tmp.path().join("test.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();

    // schema_migrations 에 version 1, 2 모두 적용
    let conn = db.conn();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version IN (1, 2)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);

    // vec0 가상 테이블 정상 사용 가능 — schema 검증
    let _ok: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM message_embedding_map",
            [],
            |r| r.get(0),
        )
        .unwrap();
}
