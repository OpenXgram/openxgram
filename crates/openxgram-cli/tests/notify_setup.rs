//! `xgram notify setup-telegram / setup-discord` 인터랙티브 마법사 통합 테스트.
//!
//! - 비대화 모드 (`OPENXGRAM_SETUP_NONINTERACTIVE=1` + `OPENXGRAM_SETUP_TOKEN`)
//!   로 stdin 입력 우회.
//! - `TELEGRAM_API_BASE` / `DISCORD_API_BASE` 환경변수로 mock 서버 교체.
//! - `data_dir` 옵션을 tempdir 으로 격리해 `~/.openxgram/notify.toml` 오염 방지.

use openxgram_cli::notify_setup::{
    run_setup, NotifyConfig, SetupOpts, SetupTarget,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn clear_setup_env() {
    unsafe {
        std::env::remove_var("OPENXGRAM_SETUP_NONINTERACTIVE");
        std::env::remove_var("OPENXGRAM_SETUP_TOKEN");
        std::env::remove_var("OPENXGRAM_SETUP_CHAT_ID");
        std::env::remove_var("OPENXGRAM_SETUP_WEBHOOK_URL");
        std::env::remove_var("OPENXGRAM_SETUP_CHANNEL_ID");
        std::env::remove_var("TELEGRAM_API_BASE");
        std::env::remove_var("DISCORD_API_BASE");
    }
}

#[tokio::test]
#[serial_test::file_serial]
async fn telegram_setup_validates_saves_and_sends_test_message() {
    clear_setup_env();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/botTOKEN/getMe"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": {"username": "myxgram_bot"}
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/botTOKEN/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("TELEGRAM_API_BASE", server.uri());
        std::env::set_var("OPENXGRAM_SETUP_NONINTERACTIVE", "1");
        std::env::set_var("OPENXGRAM_SETUP_TOKEN", "TOKEN");
        std::env::set_var("OPENXGRAM_SETUP_CHAT_ID", "12345");
    }

    let result = run_setup(
        SetupTarget::Telegram,
        SetupOpts {
            data_dir: Some(tmp.path().to_path_buf()),
            detect_attempts: Some(1),
        },
    )
    .await;

    clear_setup_env();
    result.expect("telegram setup should succeed");

    // 저장된 notify.toml 확인 + perm 0600.
    let cfg = NotifyConfig::load(Some(tmp.path())).unwrap();
    let tg = cfg.telegram.expect("telegram entry");
    assert_eq!(tg.bot_token, "TOKEN");
    assert_eq!(tg.chat_id, "12345");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp.path().join("notify.toml");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "notify.toml must be perm 0600");
    }
}

#[tokio::test]
#[serial_test::file_serial]
async fn telegram_setup_rejects_invalid_token() {
    clear_setup_env();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/botBADTOKEN/getMe"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false,
            "description": "Unauthorized"
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("TELEGRAM_API_BASE", server.uri());
        std::env::set_var("OPENXGRAM_SETUP_NONINTERACTIVE", "1");
        std::env::set_var("OPENXGRAM_SETUP_TOKEN", "BADTOKEN");
    }

    let err = run_setup(
        SetupTarget::Telegram,
        SetupOpts {
            data_dir: Some(tmp.path().to_path_buf()),
            detect_attempts: Some(1),
        },
    )
    .await
    .unwrap_err();

    clear_setup_env();
    let msg = format!("{err:#}");
    assert!(msg.contains("getMe") || msg.contains("Telegram"), "msg={msg}");
    // 토큰이 검증되지 않았으니 notify.toml 도 저장되지 않아야 한다.
    assert!(!tmp.path().join("notify.toml").exists());
}

#[tokio::test]
#[serial_test::file_serial]
async fn discord_setup_validates_saves_and_sends_webhook_test() {
    clear_setup_env();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "username": "xgram-bot",
            "discriminator": "0"
        })))
        .mount(&server)
        .await;

    // webhook URL 도 같은 mock 서버로 — Discord webhook 은 임의 URL 이라 OK.
    Mock::given(method("POST"))
        .and(path("/webhooks/123/abc"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let webhook_url = format!("{}/webhooks/123/abc", server.uri());
    let tmp = tempfile::tempdir().unwrap();

    unsafe {
        std::env::set_var("DISCORD_API_BASE", server.uri());
        std::env::set_var("OPENXGRAM_SETUP_NONINTERACTIVE", "1");
        std::env::set_var("OPENXGRAM_SETUP_TOKEN", "MTIDISCORD");
        std::env::set_var("OPENXGRAM_SETUP_CHANNEL_ID", "999888");
        std::env::set_var("OPENXGRAM_SETUP_WEBHOOK_URL", &webhook_url);
    }

    let result = run_setup(
        SetupTarget::Discord,
        SetupOpts {
            data_dir: Some(tmp.path().to_path_buf()),
            detect_attempts: None,
        },
    )
    .await;

    clear_setup_env();
    result.expect("discord setup should succeed");

    let cfg = NotifyConfig::load(Some(tmp.path())).unwrap();
    let dc = cfg.discord.expect("discord entry");
    assert_eq!(dc.bot_token, "MTIDISCORD");
    assert_eq!(dc.channel_id.as_deref(), Some("999888"));
    assert_eq!(dc.webhook_url.as_deref(), Some(webhook_url.as_str()));
}

#[tokio::test]
#[serial_test::file_serial]
async fn discord_setup_skips_test_when_no_webhook() {
    clear_setup_env();
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "username": "xgram-bot",
            "discriminator": "0"
        })))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("DISCORD_API_BASE", server.uri());
        std::env::set_var("OPENXGRAM_SETUP_NONINTERACTIVE", "1");
        std::env::set_var("OPENXGRAM_SETUP_TOKEN", "MTIONLY");
        // channel_id, webhook 누락 — 마법사가 skip 해야 함.
    }

    let result = run_setup(
        SetupTarget::Discord,
        SetupOpts {
            data_dir: Some(tmp.path().to_path_buf()),
            detect_attempts: None,
        },
    )
    .await;

    clear_setup_env();
    result.expect("discord setup should succeed without webhook");

    let cfg = NotifyConfig::load(Some(tmp.path())).unwrap();
    let dc = cfg.discord.expect("discord entry");
    assert_eq!(dc.bot_token, "MTIONLY");
    assert!(dc.channel_id.is_none());
    assert!(dc.webhook_url.is_none());
}
