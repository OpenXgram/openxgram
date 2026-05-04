//! `xgram notify channel` — Starian Channel MCP HTTP gateway 어댑터의 CLI wiring
//! 통합 테스트.
//!
//! - wiremock 으로 HTTP gateway 의 `POST /mcp` (JSON-RPC tools/call) 모킹.
//! - 시나리오: send_to_platform 성공 / send_message 성공 / 미설정 env raise.

use openxgram_cli::notify::{
    run_notify, ChannelMode, NotifyAction, CHANNEL_MCP_TOKEN_ENV, CHANNEL_MCP_URL_ENV,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn clear_channel_env() {
    unsafe {
        std::env::remove_var(CHANNEL_MCP_URL_ENV);
        std::env::remove_var(CHANNEL_MCP_TOKEN_ENV);
    }
}

#[tokio::test]
#[serial_test::file_serial]
async fn channel_send_to_platform_succeeds_via_jsonrpc() {
    clear_channel_env();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "success": true,
                "message_id": "discord-msg-42"
            }
        })))
        .mount(&server)
        .await;

    let action = NotifyAction::Channel {
        mcp_url: Some(server.uri()),
        auth_token: None,
        mode: ChannelMode::Platform {
            platform: "discord".into(),
            channel_id: "12345".into(),
            text: "hello".into(),
            reply_to: None,
        },
    };
    run_notify(action).await.expect("send_to_platform 성공해야 함");
}

#[tokio::test]
#[serial_test::file_serial]
async fn channel_send_message_peer_routing_succeeds() {
    clear_channel_env();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [
                    {"type": "text", "text": "{\"success\":true,\"message_id\":\"peer-7\"}"}
                ]
            }
        })))
        .mount(&server)
        .await;

    let action = NotifyAction::Channel {
        mcp_url: Some(server.uri()),
        auth_token: Some("test-token".into()),
        mode: ChannelMode::Peer {
            to_role: "res".into(),
            summary: "조사 요청".into(),
            msg_type: "request".into(),
        },
    };
    run_notify(action).await.expect("send_message 성공해야 함");
}

#[tokio::test]
#[serial_test::file_serial]
async fn channel_missing_url_raises_explicit_error() {
    clear_channel_env();

    let action = NotifyAction::Channel {
        mcp_url: None,
        auth_token: None,
        mode: ChannelMode::ListAdapters,
    };
    let err = run_notify(action).await.expect_err("미설정 시 raise 해야 함");
    let msg = format!("{err}");
    assert!(
        msg.contains("--mcp-url") || msg.contains(CHANNEL_MCP_URL_ENV),
        "에러 메시지에 옵션/환경변수 안내 필요: {msg}"
    );
}

#[tokio::test]
#[serial_test::file_serial]
async fn channel_list_adapters_prints_registered() {
    clear_channel_env();
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": [
                {"platform": "discord", "connected": true, "channel_id": "111"},
                {"platform": "telegram", "connected": false}
            ]
        })))
        .mount(&server)
        .await;

    let action = NotifyAction::Channel {
        mcp_url: Some(server.uri()),
        auth_token: None,
        mode: ChannelMode::ListAdapters,
    };
    run_notify(action).await.expect("list_adapters 성공해야 함");
}
