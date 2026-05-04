//! xgram notify discord-listen smoke test.
//!
//! - 일반 모드: env 누락 시 raise 검증 (CLI wiring + bot_token resolve).
//! - 실 Gateway 호출: `RUN_DISCORD=1 DISCORD_BOT_TOKEN=...` 환경에서만 (#[ignore]).
//!
//! adapter crate 의 `DiscordIncomingMessage::from_event` 단위 테스트는 그쪽에.

use std::time::Duration;

use openxgram_cli::notify::{run_notify, NotifyAction};

fn clear_env() {
    unsafe {
        std::env::remove_var("DISCORD_BOT_TOKEN");
    }
}

#[tokio::test]
#[serial_test::file_serial]
async fn discord_listen_requires_bot_token() {
    clear_env();
    let err = run_notify(NotifyAction::DiscordListen {
        bot_token: None,
        channel_id: None,
        store_session: None,
        data_dir: None,
        pretty: false,
    })
    .await
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("DISCORD_BOT_TOKEN") || msg.contains("--bot-token"),
        "expected DISCORD_BOT_TOKEN error, got: {msg}"
    );
}

#[tokio::test]
#[serial_test::file_serial]
async fn discord_listen_store_without_data_dir_raises() {
    // bot_token 은 있고 store_session 도 있지만 data_dir 미지정 → raise.
    clear_env();
    let err = run_notify(NotifyAction::DiscordListen {
        bot_token: Some("dummy.token.value".into()),
        channel_id: None,
        store_session: Some("nonexistent-session".into()),
        data_dir: None,
        pretty: false,
    })
    .await
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("data_dir") || msg.contains("data-dir") || msg.contains("디렉토리"),
        "expected data_dir error, got: {msg}"
    );
}

/// 실 Discord Gateway 연결 — RUN_DISCORD=1 + DISCORD_BOT_TOKEN 필요.
/// 5초 타임아웃 안에 연결만 확인 (메시지 수신은 unit test 범위 외).
#[tokio::test]
#[ignore = "requires RUN_DISCORD=1 and DISCORD_BOT_TOKEN env"]
async fn discord_listen_live_connect() {
    if std::env::var("RUN_DISCORD").ok().as_deref() != Some("1") {
        return;
    }
    let _ = std::env::var("DISCORD_BOT_TOKEN").expect("DISCORD_BOT_TOKEN required");
    // 5초 후 timeout — 봇이 invalid 면 Shard 가 즉시 fatal 종료한다.
    let res = tokio::time::timeout(
        Duration::from_secs(5),
        run_notify(NotifyAction::DiscordListen {
            bot_token: None,
            channel_id: None,
            store_session: None,
            data_dir: None,
            pretty: true,
        }),
    )
    .await;
    // timeout (= 정상 stream 진행) 또는 Ok 둘 다 허용. Err 는 token 무효.
    match res {
        Err(_) => { /* timeout — stream 은 정상 동작 중 */ }
        Ok(Ok(())) => { /* ctrl_c 등으로 정상 종료 */ }
        Ok(Err(e)) => panic!("gateway connect failed: {e:#}"),
    }
}
