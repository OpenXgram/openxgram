//! xgram notify — Discord webhook / Telegram bot 으로 텍스트 전송.
//!
//! adapter crate 의 첫 cli 사용처. 토큰·URL 은 인자 또는 환경변수.
//! Phase 1 first PR: 단순 텍스트만. backup-push (session export → 어댑터)
//! 와 cron 자동 전송은 후속 PR.

use anyhow::{anyhow, Result};
use openxgram_adapter::{Adapter, DiscordWebhookAdapter, TelegramBotAdapter};

const DISCORD_URL_ENV: &str = "DISCORD_WEBHOOK_URL";
const TELEGRAM_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
const TELEGRAM_CHAT_ENV: &str = "TELEGRAM_CHAT_ID";

#[derive(Debug, Clone)]
pub enum NotifyAction {
    Discord {
        webhook_url: Option<String>,
        text: String,
    },
    Telegram {
        bot_token: Option<String>,
        chat_id: Option<String>,
        text: String,
    },
}

pub async fn run_notify(action: NotifyAction) -> Result<()> {
    match action {
        NotifyAction::Discord { webhook_url, text } => {
            let url = resolve(webhook_url, DISCORD_URL_ENV, "--webhook-url")?;
            DiscordWebhookAdapter::new(url).send_text(&text).await?;
            println!("✓ Discord 전송 완료 ({} chars)", text.chars().count());
        }
        NotifyAction::Telegram {
            bot_token,
            chat_id,
            text,
        } => {
            let token = resolve(bot_token, TELEGRAM_TOKEN_ENV, "--bot-token")?;
            let chat = resolve(chat_id, TELEGRAM_CHAT_ENV, "--chat-id")?;
            TelegramBotAdapter::new(token, chat).send_text(&text).await?;
            println!("✓ Telegram 전송 완료 ({} chars)", text.chars().count());
        }
    }
    Ok(())
}

fn resolve(arg: Option<String>, env: &str, flag: &str) -> Result<String> {
    arg.or_else(|| std::env::var(env).ok())
        .ok_or_else(|| anyhow!("{flag} 또는 환경변수 {env} 가 필요합니다"))
}
