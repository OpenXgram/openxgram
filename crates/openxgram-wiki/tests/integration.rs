//! 통합 테스트 — write → read → sync → list → link.
//!
//! 모듈별 unit test는 각 src/*.rs 안의 #[cfg(test)]에 있음.
//! 본 파일은 end-to-end 시나리오.

use openxgram_wiki::{mcp::WikiTools, Syncer, WikiFs};
use rusqlite::Connection;
use tempfile::tempdir;

fn fresh_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    // 0018 마이그레이션 시뮬레이션
    conn.execute_batch(include_str!(
        "../../openxgram-db/migrations/0018_wiki_pages.sql"
    ))
    .unwrap();
    conn
}

#[tokio::test]
async fn end_to_end_flow() {
    let tmp = tempdir().unwrap();
    let wf = WikiFs::new(tmp.path().join("wiki"));
    wf.ensure_dirs().await.unwrap();

    let conn = fresh_db();
    let tools = WikiTools::new(&wf, &conn);

    // 1) 신규 페이지 2개 작성
    let r1 = tools
        .write("entity/alice", "# Alice\n\nFirst page.", None, None)
        .await
        .unwrap();
    assert!(r1.created);

    let r2 = tools
        .write("concept/teamwork", "# Teamwork\n\nConcept.", None, None)
        .await
        .unwrap();
    assert!(r2.created);

    // 2) 검색
    let hits = tools.search("Alice", Some(5), None).unwrap();
    assert!(!hits.is_empty());

    // 3) 링크
    tools
        .link("entity/alice", "concept/teamwork", Some("collab"))
        .await
        .unwrap();

    // 4) 목록
    let entries = tools.list(None).unwrap();
    assert_eq!(entries.len(), 2);

    let entity_only = tools.list(Some("entity")).unwrap();
    assert_eq!(entity_only.len(), 1);
    assert_eq!(entity_only[0].id, "entity/alice");

    // 5) 디스크 → DB 재동기화: 새 sync는 unchanged로 처리되어야
    let syncer = Syncer::new(&wf, &conn);
    let report = syncer.sync_disk_to_db().await.unwrap();
    assert_eq!(report.added, 0);
    assert_eq!(report.errors.len(), 0);
    // updated_count는 link로 인해 alice가 갱신됐을 수 있음
}

#[tokio::test]
async fn write_again_updates_not_creates() {
    let tmp = tempdir().unwrap();
    let wf = WikiFs::new(tmp.path().join("wiki"));
    wf.ensure_dirs().await.unwrap();
    let conn = fresh_db();
    let tools = WikiTools::new(&wf, &conn);

    let r1 = tools
        .write("entity/bob", "# Bob\n\nv1", None, None)
        .await
        .unwrap();
    assert!(r1.created);

    let r2 = tools
        .write("entity/bob", "# Bob\n\nv2", None, None)
        .await
        .unwrap();
    assert!(!r2.created);
    assert_ne!(r1.content_hash, r2.content_hash);
}
