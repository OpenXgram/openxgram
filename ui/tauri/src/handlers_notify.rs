//! Notify (Discord/Telegram) 마법사 invoke 핸들러.
//!
//! `openxgram-cli::notify_setup` 의 helper 를 직접 호출해 토큰 검증 / chat_id
//! 자동 감지 / 저장 / 테스트 메시지 전송을 GUI 가 단계별로 트리거할 수 있게 한다.
//!
//! - `notify_telegram_validate` — getMe → bot username
//! - `notify_telegram_detect_chat` — getUpdates polling (1회) → chat_id
//! - `notify_telegram_save` — NotifyConfig.telegram = {token, chat_id} 저장 + 테스트 전송
//! - `notify_discord_validate` — /users/@me → bot label
//! - `notify_discord_save` — NotifyConfig.discord 저장 + webhook 테스트
//! - `notify_status` — 현재 저장된 token 존재 여부 (값은 노출하지 않음)

use serde::Serialize;
use tauri::State;

use openxgram_cli::notify_setup::{
    discord_api_base, discord_get_me, discord_send_webhook, telegram_api_base,
    telegram_detect_chat_id, telegram_get_me, telegram_send, DiscordConfig, NotifyConfig,
    TelegramConfig,
};

use crate::state::AppState;

#[derive(Serialize, Clone)]
pub struct TelegramValidateDto {
    pub bot_username: String,
}

#[tauri::command]
pub async fn notify_telegram_validate(token: String) -> Result<TelegramValidateDto, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("token 비어있음".into());
    }
    let api = telegram_api_base();
    let bot = telegram_get_me(&api, token)
        .await
        .map_err(|e| format!("Telegram getMe 실패: {e}"))?;
    let bot_username = bot
        .username
        .ok_or_else(|| "Telegram getMe 응답에 username 누락".to_string())?;
    Ok(TelegramValidateDto { bot_username })
}

/// 단발 polling — 25초 timeout 1회 시도. UI 가 반복 호출.
#[tauri::command]
pub async fn notify_telegram_detect_chat(token: String) -> Result<Option<String>, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("token 비어있음".into());
    }
    let api = telegram_api_base();
    let detected = telegram_detect_chat_id(&api, token, 1)
        .await
        .map_err(|e| format!("Telegram getUpdates 실패: {e}"))?;
    Ok(detected.map(|id| id.to_string()))
}

#[tauri::command]
pub async fn notify_telegram_save(
    state: State<'_, AppState>,
    token: String,
    chat_id: String,
    test_text: Option<String>,
) -> Result<String, String> {
    let token = token.trim().to_string();
    let chat_id = chat_id.trim().to_string();
    if token.is_empty() || chat_id.is_empty() {
        return Err("token / chat_id 비어있음".into());
    }
    let data_dir = state.data_dir.clone();
    let mut config = NotifyConfig::load(Some(&data_dir))
        .map_err(|e| format!("NotifyConfig load: {e}"))?;
    config.telegram = Some(TelegramConfig {
        bot_token: token.clone(),
        chat_id: chat_id.clone(),
    });
    let path = config
        .save(Some(&data_dir))
        .map_err(|e| format!("NotifyConfig save: {e}"))?;

    if let Some(text) = test_text.as_deref() {
        if !text.is_empty() {
            let api = telegram_api_base();
            telegram_send(&api, &token, &chat_id, text)
                .await
                .map_err(|e| format!("Telegram sendMessage 실패: {e}"))?;
        }
    }
    Ok(path.display().to_string())
}

#[derive(Serialize, Clone)]
pub struct DiscordValidateDto {
    pub bot_label: String,
}

#[tauri::command]
pub async fn notify_discord_validate(token: String) -> Result<DiscordValidateDto, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("token 비어있음".into());
    }
    let api = discord_api_base();
    let user = discord_get_me(&api, token)
        .await
        .map_err(|e| format!("Discord /users/@me 실패: {e}"))?;
    let label = match (user.username, user.discriminator) {
        (Some(u), Some(d)) if d != "0" => format!("{u}#{d}"),
        (Some(u), _) => u,
        _ => "(unknown)".into(),
    };
    Ok(DiscordValidateDto { bot_label: label })
}

#[tauri::command]
pub async fn notify_discord_save(
    state: State<'_, AppState>,
    token: String,
    channel_id: Option<String>,
    webhook_url: Option<String>,
    test_text: Option<String>,
) -> Result<String, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("token 비어있음".into());
    }
    let data_dir = state.data_dir.clone();
    let mut config = NotifyConfig::load(Some(&data_dir))
        .map_err(|e| format!("NotifyConfig load: {e}"))?;
    let channel_id = channel_id.and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    });
    let webhook_url = webhook_url.and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    });
    config.discord = Some(DiscordConfig {
        bot_token: token,
        channel_id,
        webhook_url: webhook_url.clone(),
    });
    let path = config
        .save(Some(&data_dir))
        .map_err(|e| format!("NotifyConfig save: {e}"))?;

    if let (Some(url), Some(text)) = (webhook_url.as_deref(), test_text.as_deref()) {
        if !url.is_empty() && !text.is_empty() {
            discord_send_webhook(url, text)
                .await
                .map_err(|e| format!("Discord webhook 실패: {e}"))?;
        }
    }
    Ok(path.display().to_string())
}

/// 저장된 notify.toml 의 어댑터 연결 상태 — 값은 노출 X (boolean 만).
#[derive(Serialize, Clone, Default)]
pub struct NotifyStatusDto {
    pub telegram_configured: bool,
    pub discord_configured: bool,
    pub discord_webhook_configured: bool,
}

#[tauri::command]
pub fn notify_status(state: State<'_, AppState>) -> Result<NotifyStatusDto, String> {
    let cfg = NotifyConfig::load(Some(&state.data_dir))
        .map_err(|e| format!("NotifyConfig load: {e}"))?;
    Ok(NotifyStatusDto {
        telegram_configured: cfg.telegram.is_some(),
        discord_configured: cfg.discord.is_some(),
        discord_webhook_configured: cfg
            .discord
            .as_ref()
            .and_then(|d| d.webhook_url.as_deref())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
    })
}
