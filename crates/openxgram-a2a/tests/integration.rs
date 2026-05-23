//! 통합 테스트 — mockito 로 외부 A2A 에이전트 simulate.
//!
//! 모듈별 unit test는 각 src/*.rs 안의 #[cfg(test)] 에 있음.
//! 본 파일은 end-to-end 시나리오.

use openxgram_a2a::{A2aError, A2aTools, TaskState};
use openxgram_a2a::mcp::SendTaskArgs;
use serde_json::json;

const AGENT_CARD_FIXTURE: &str = r#"{
    "name": "Translation Agent",
    "description": "Translates text between languages",
    "url": "https://translate.example.com/agent",
    "version": "1.0.0",
    "authentication": { "schemes": ["bearer"] },
    "skills": [{
        "id": "translate",
        "name": "Translate text",
        "description": "Translate text from one language to another",
        "inputModes": ["text"],
        "outputModes": ["text"]
    }],
    "capabilities": { "streaming": true, "pushNotifications": false }
}"#;

fn rpc_ok(result: serde_json::Value) -> String {
    json!({ "jsonrpc": "2.0", "id": "anything", "result": result }).to_string()
}

fn rpc_err(code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": "anything",
        "error": { "code": code, "message": message }
    })
    .to_string()
}

#[tokio::test]
async fn discover_fetches_agent_card() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/.well-known/agent-card.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(AGENT_CARD_FIXTURE)
        .create_async()
        .await;

    let tools = A2aTools::new();
    let card = tools.discover(&server.url()).await.unwrap();

    assert_eq!(card.name, "Translation Agent");
    assert_eq!(card.skills.len(), 1);
    assert_eq!(card.skills[0].id, "translate");
    assert!(card.capabilities.streaming);
    m.assert_async().await;
}

#[tokio::test]
async fn discover_404_returns_typed_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/.well-known/agent-card.json")
        .with_status(404)
        .with_body("not found")
        .create_async()
        .await;

    let tools = A2aTools::new();
    let err = tools.discover(&server.url()).await.unwrap_err();
    match err {
        A2aError::AgentCardFetch { status, .. } => assert_eq!(status, 404),
        other => panic!("expected AgentCardFetch, got {other:?}"),
    }
}

#[tokio::test]
async fn send_task_returns_task_with_state() {
    let mut server = mockito::Server::new_async().await;
    let body = rpc_ok(json!({
        "id": "t-abc",
        "status": { "state": "working" }
    }));
    let m = server
        .mock("POST", "/")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create_async()
        .await;

    let tools = A2aTools::new();
    let task = tools
        .send_task(SendTaskArgs {
            agent_url: server.url(),
            skill: "translate".into(),
            params: json!({ "text": "hello", "target": "ko" }),
            session_id: None,
        })
        .await
        .unwrap();

    assert_eq!(task.id, "t-abc");
    assert_eq!(task.status.state, TaskState::Working);
    assert!(!task.status.state.is_terminal());
    m.assert_async().await;
}

#[tokio::test]
async fn get_task_returns_completed_task() {
    let mut server = mockito::Server::new_async().await;
    let body = rpc_ok(json!({
        "id": "t-done",
        "status": { "state": "completed" },
        "artifacts": [ { "type": "text", "text": "안녕" } ]
    }));
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create_async()
        .await;

    let tools = A2aTools::new();
    let task = tools.get_task(&server.url(), "t-done").await.unwrap();

    assert_eq!(task.id, "t-done");
    assert_eq!(task.status.state, TaskState::Completed);
    assert!(task.status.state.is_terminal());
    assert_eq!(task.artifacts.len(), 1);
    m.assert_async().await;
}

#[tokio::test]
async fn cancel_task_returns_canceled() {
    let mut server = mockito::Server::new_async().await;
    let body = rpc_ok(json!({
        "id": "t-x",
        "status": { "state": "canceled" }
    }));
    let m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create_async()
        .await;

    let tools = A2aTools::new();
    let task = tools.cancel_task(&server.url(), "t-x").await.unwrap();

    assert_eq!(task.status.state, TaskState::Canceled);
    m.assert_async().await;
}

#[tokio::test]
async fn rpc_error_propagates_typed() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(rpc_err(-32001, "task not found"))
        .create_async()
        .await;

    let tools = A2aTools::new();
    let err = tools.get_task(&server.url(), "missing").await.unwrap_err();
    match err {
        A2aError::RpcError { code, message } => {
            assert_eq!(code, -32001);
            assert!(message.contains("not found"));
        }
        other => panic!("expected RpcError, got {other:?}"),
    }
}

#[tokio::test]
async fn bearer_token_is_forwarded() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/.well-known/agent-card.json")
        .match_header("authorization", "Bearer secret-tok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(AGENT_CARD_FIXTURE)
        .create_async()
        .await;

    let tools = A2aTools::new().with_default_bearer("secret-tok");
    let card = tools.discover(&server.url()).await.unwrap();
    assert_eq!(card.name, "Translation Agent");
    m.assert_async().await;
}

#[tokio::test]
async fn invalid_rpc_response_typed_error() {
    let mut server = mockito::Server::new_async().await;
    // 응답에 result도 error도 없음
    let _m = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"jsonrpc":"2.0","id":"x"}"#)
        .create_async()
        .await;

    let tools = A2aTools::new();
    let err = tools.get_task(&server.url(), "t").await.unwrap_err();
    assert!(matches!(err, A2aError::InvalidRpcResponse));
}
