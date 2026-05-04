//! `xgram notify telegram-listen` 통합 테스트.
//!
//! TELEGRAM_API_BASE 환경변수로 어댑터 base 를 mock 서버로 교체.
//! once=true 모드로 1회 polling 후 종료. L0 저장은 keystore·DB 셋업이 무거우므로
//! 별도 테스트(생략) — 여기서는 wiring 만 검증.

use openxgram_cli::notify::{run_notify, NotifyAction};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn clear_env() {
    unsafe {
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        std::env::remove_var("TELEGRAM_CHAT_ID");
        std::env::remove_var("TELEGRAM_API_BASE");
    }
}

#[tokio::test]
#[serial_test::file_serial]
async fn telegram_listen_requires_token() {
    clear_env();
    let err = run_notify(NotifyAction::TelegramListen {
        bot_token: None,
        chat_id_filter: None,
        store_session_title: None,
        data_dir: None,
        once: true,
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("TELEGRAM_BOT_TOKEN"));
}

#[tokio::test]
#[serial_test::file_serial]
async fn telegram_listen_once_polls_and_exits() {
    clear_env();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/botTOKEN/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": [
                {"update_id": 1, "message": {
                    "chat": {"id": 999},
                    "from": {"username": "tester"},
                    "text": "hi from mock"
                }}
            ]
        })))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("TELEGRAM_API_BASE", server.uri());
    }

    let result = run_notify(NotifyAction::TelegramListen {
        bot_token: Some("TOKEN".into()),
        chat_id_filter: None,
        store_session_title: None, // L0 저장 안 함 (DB 셋업 우회)
        data_dir: None,
        once: true,
    })
    .await;

    unsafe {
        std::env::remove_var("TELEGRAM_API_BASE");
    }
    result.expect("listen once should succeed");
}

#[tokio::test]
#[serial_test::file_serial]
async fn telegram_listen_chat_filter_excludes_others() {
    clear_env();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/botTOKEN/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": [
                {"update_id": 5, "message": {
                    "chat": {"id": 111},
                    "text": "from chat 111"
                }},
                {"update_id": 6, "message": {
                    "chat": {"id": 222},
                    "text": "from chat 222"
                }}
            ]
        })))
        .mount(&server)
        .await;

    unsafe {
        std::env::set_var("TELEGRAM_API_BASE", server.uri());
    }

    // chat_id_filter=111 만 통과해야 함. 통과·차단 모두 stdout 만 영향, 에러 없이 끝나면 OK.
    let result = run_notify(NotifyAction::TelegramListen {
        bot_token: Some("TOKEN".into()),
        chat_id_filter: Some(111),
        store_session_title: None,
        data_dir: None,
        once: true,
    })
    .await;

    unsafe {
        std::env::remove_var("TELEGRAM_API_BASE");
    }
    result.expect("listen with filter should succeed");
}
