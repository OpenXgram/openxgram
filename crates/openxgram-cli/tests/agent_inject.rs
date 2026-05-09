//! 1.5.1.5 — `POST /v1/agent/inject` HTTP 통합 테스트.
//!
//! 검증:
//! - mcp_token Bearer 인증 성공 → 200 + (message_id, session_id, conversation_id) 반환
//! - sender 비어있음 → 400
//! - 잘못된 토큰 → 401
//! - inbox-from-{sender} 세션 자동 생성 + 메시지 저장
//! - conversation_id 옵션 — 미지정 시 새 ID, 지정 시 그대로 thread

use std::net::TcpListener;

use openxgram_cli::daemon_gui::spawn_gui_server;
use openxgram_cli::mcp_tokens;
use openxgram_db::{Db, DbConfig};
use serde_json::json;
use tempfile::tempdir;

fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn boot_test_server() -> (tempfile::TempDir, u16, String) {
    let tmp = tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    // pre-create DB + token
    let mut db = Db::open(DbConfig {
        path: openxgram_core::paths::db_path(&dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let (_id, token_plain) =
        mcp_tokens::create_token(&mut db, "test-agent", Some("inject-test")).unwrap();
    drop(db);

    let port = pick_free_port();
    let bind = format!("127.0.0.1:{port}").parse().unwrap();
    spawn_gui_server(dir.clone(), bind).await.unwrap();
    // server is now running in background; small grace period
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (tmp, port, token_plain)
}

#[tokio::test]
async fn inject_with_valid_token_saves_message_and_returns_ids() {
    let (tmp, port, token) = boot_test_server().await;
    let url = format!("http://127.0.0.1:{port}/v1/agent/inject");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({
            "sender": "discord:master",
            "body": "안녕"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Bearer 인증 + 정상 body → 200");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["message_id"].as_str().unwrap().len() > 0);
    assert!(body["session_id"].as_str().unwrap().len() > 0);
    assert!(
        body["conversation_id"].as_str().unwrap().len() > 0,
        "conversation_id 자동 생성"
    );

    // DB 직접 검증 — inbox-from-discord:master 세션 + 메시지 1개
    let mut db = Db::open(DbConfig {
        path: openxgram_core::paths::db_path(tmp.path()),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages
             WHERE session_id = (SELECT id FROM sessions WHERE title = ?1)
               AND sender = 'discord:master' AND body = '안녕'",
            ["inbox-from-discord:master"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn inject_threads_existing_conversation_when_id_provided() {
    let (_tmp, port, token) = boot_test_server().await;
    let url = format!("http://127.0.0.1:{port}/v1/agent/inject");
    let client = reqwest::Client::new();

    let r1: serde_json::Value = client
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({"sender": "discord:m", "body": "first"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let conv = r1["conversation_id"].as_str().unwrap().to_string();

    let r2: serde_json::Value = client
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({
            "sender": "discord:m",
            "body": "second",
            "conversation_id": conv
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        r2["conversation_id"].as_str().unwrap(),
        conv,
        "지정한 conversation_id 그대로 저장"
    );
}

#[tokio::test]
async fn inject_rejects_empty_sender_with_400() {
    let (_tmp, port, token) = boot_test_server().await;
    let url = format!("http://127.0.0.1:{port}/v1/agent/inject");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({"sender": "", "body": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn inject_rejects_bad_token_with_401() {
    let (_tmp, port, _real_token) = boot_test_server().await;
    let url = format!("http://127.0.0.1:{port}/v1/agent/inject");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth("wrong-token")
        .json(&json!({"sender": "discord:m", "body": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}
