//! Discord/Telegram 어댑터 통합 테스트 (wiremock).

use openxgram_adapter::{Adapter, AdapterError, DiscordWebhookAdapter, TelegramBotAdapter};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn discord_posts_content_field() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook/test"))
        .and(body_partial_json(json!({"content": "hello world"})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/hook/test", server.uri());
    DiscordWebhookAdapter::new(url)
        .send_text("hello world")
        .await
        .unwrap();
}

#[tokio::test]
async fn discord_raises_on_4xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hook/x"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let url = format!("{}/hook/x", server.uri());
    let err = DiscordWebhookAdapter::new(url)
        .send_text("hi")
        .await
        .unwrap_err();
    assert!(matches!(err, AdapterError::ServerError { status: 404, .. }));
}

#[tokio::test]
async fn telegram_posts_chat_id_and_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot7869683671:TOKEN/sendMessage"))
        .and(body_partial_json(
            json!({"chat_id": "6565914284", "text": "안녕"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    TelegramBotAdapter::new("7869683671:TOKEN", "6565914284")
        .with_api_base(server.uri())
        .send_text("안녕")
        .await
        .unwrap();
}

#[tokio::test]
async fn telegram_raises_on_invalid_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot999:WRONG/sendMessage"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"ok": false, "description": "Unauthorized"})),
        )
        .mount(&server)
        .await;

    let err = TelegramBotAdapter::new("999:WRONG", "1")
        .with_api_base(server.uri())
        .send_text("hi")
        .await
        .unwrap_err();
    assert!(matches!(err, AdapterError::ServerError { status: 401, .. }));
}

#[tokio::test]
async fn discord_connection_failure_raises_http() {
    // 사용 안 하는 포트 — connection refused
    let err = DiscordWebhookAdapter::new("http://127.0.0.1:1/hook")
        .send_text("hi")
        .await
        .unwrap_err();
    assert!(matches!(err, AdapterError::Http(_)), "got {err:?}");
}
