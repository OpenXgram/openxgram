//! TelegramBotAdapter::poll_updates 통합 테스트 (wiremock).

use openxgram_adapter::TelegramBotAdapter;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn poll_updates_returns_text_messages() {
    let server = MockServer::start().await;

    let body = json!({
        "ok": true,
        "result": [
            {
                "update_id": 42,
                "message": {
                    "chat": {"id": 1234},
                    "from": {"username": "alice"},
                    "text": "hello from telegram"
                }
            },
            {
                "update_id": 43,
                "message": {
                    "chat": {"id": 5678},
                    "text": "no sender"
                }
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/botTOKEN/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let adapter = TelegramBotAdapter::new("TOKEN", "1234").with_api_base(server.uri());
    let updates = adapter.poll_updates(0, Some(1)).await.expect("poll ok");

    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].update_id, 42);
    assert_eq!(updates[0].chat_id, 1234);
    assert_eq!(updates[0].text, "hello from telegram");
    assert_eq!(updates[0].sender_username.as_deref(), Some("alice"));
    assert_eq!(updates[1].update_id, 43);
    assert_eq!(updates[1].sender_username, None);
}

#[tokio::test]
async fn poll_updates_propagates_server_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/botTOKEN/getUpdates"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&server)
        .await;

    let adapter = TelegramBotAdapter::new("TOKEN", "1234").with_api_base(server.uri());
    let err = adapter
        .poll_updates(0, Some(1))
        .await
        .expect_err("should fail");
    let s = err.to_string();
    assert!(s.contains("401"), "expected 401 in error, got: {s}");
}

#[tokio::test]
async fn poll_updates_handles_ok_false() {
    let server = MockServer::start().await;
    let body = json!({"ok": false, "description": "bad token"});
    Mock::given(method("GET"))
        .and(path("/botTOKEN/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let adapter = TelegramBotAdapter::new("TOKEN", "1234").with_api_base(server.uri());
    let err = adapter
        .poll_updates(0, Some(1))
        .await
        .expect_err("ok=false should error");
    let s = err.to_string();
    assert!(s.contains("bad token"), "expected description, got: {s}");
}
