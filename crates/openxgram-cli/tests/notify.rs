//! xgram notify cli 통합 테스트.
//!
//! adapter 자체 wiremock 테스트는 adapter crate. 여기서는 cli wiring + env
//! 누락 raise 만 검증 (cli 가 토큰·URL 을 전달하는지).

use openxgram_cli::notify::{run_notify, NotifyAction};

fn clear_env() {
    unsafe {
        std::env::remove_var("DISCORD_WEBHOOK_URL");
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        std::env::remove_var("TELEGRAM_CHAT_ID");
    }
}

#[tokio::test]
async fn discord_requires_url() {
    clear_env();
    let err = run_notify(NotifyAction::Discord {
        webhook_url: None,
        text: "hi".into(),
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("DISCORD_WEBHOOK_URL"));
}

#[tokio::test]
async fn telegram_requires_token_and_chat() {
    clear_env();
    let err = run_notify(NotifyAction::Telegram {
        bot_token: None,
        chat_id: None,
        text: "hi".into(),
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("TELEGRAM_BOT_TOKEN"));

    // bot_token 만 있을 때 chat_id 누락 raise
    let err2 = run_notify(NotifyAction::Telegram {
        bot_token: Some("123:TOKEN".into()),
        chat_id: None,
        text: "hi".into(),
    })
    .await
    .unwrap_err();
    assert!(format!("{err2:#}").contains("TELEGRAM_CHAT_ID"));
}
