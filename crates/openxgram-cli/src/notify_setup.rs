//! xgram notify setup-telegram / setup-discord — 인터랙티브 마법사.
//!
//! 마스터 결정: "디스코드든 텔레그램이든 연결하는 게 쉬워야해."
//! 5단계 가이드 카드 위에 한 줄 마법사 — 봇 토큰만 붙여넣으면
//! 토큰 검증 → chat_id 자동 감지 → 저장 → 테스트 메시지까지 자동.
//!
//! ## 흐름
//!
//! - `xgram notify setup-telegram`
//!   1. 토큰 입력 → `getMe` 로 검증 (bot username 확인)
//!   2. "본인 봇에게 /start" 안내 → `getUpdates` 로 chat_id 자동 감지
//!   3. `~/.openxgram/notify.toml` 저장 (perm 0600)
//!   4. 테스트 메시지 `sendMessage`
//!
//! - `xgram notify setup-discord`
//!   1. 토큰 입력 → Discord `/users/@me` 로 검증
//!   2. 봇 초대 URL 안내 + 채널 ID / webhook URL (옵션) 입력
//!   3. `~/.openxgram/notify.toml` 저장
//!   4. webhook 으로 테스트 메시지 (있을 때)
//!
//! ## 저장 위치
//!
//! `~/.openxgram/notify.toml` plain text + perm 0600. 마법사 단순함 우선.
//! 더 강한 보호가 필요하면 vault 사용 (vault put / get) — 별도 PR.
//!
//! ## 비대화 모드
//!
//! 테스트·CI 용으로 `OPENXGRAM_SETUP_TOKEN` / `OPENXGRAM_SETUP_CHAT_ID` /
//! `OPENXGRAM_SETUP_WEBHOOK_URL` / `OPENXGRAM_SETUP_CHANNEL_ID` 환경변수가
//! 있으면 stdin 입력 대신 사용한다 (silent 행 방지).

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

const TELEGRAM_API_BASE_DEFAULT: &str = "https://api.telegram.org";
const TELEGRAM_API_BASE_ENV: &str = "TELEGRAM_API_BASE";
const DISCORD_API_BASE_DEFAULT: &str = "https://discord.com/api/v10";
const DISCORD_API_BASE_ENV: &str = "DISCORD_API_BASE";

const SETUP_TOKEN_ENV: &str = "OPENXGRAM_SETUP_TOKEN";
const SETUP_CHAT_ID_ENV: &str = "OPENXGRAM_SETUP_CHAT_ID";
const SETUP_WEBHOOK_ENV: &str = "OPENXGRAM_SETUP_WEBHOOK_URL";
const SETUP_CHANNEL_ENV: &str = "OPENXGRAM_SETUP_CHANNEL_ID";
/// 비대화 / 테스트 모드 — stdin 무시 + 화면 안내·대기 최소화.
const SETUP_NONINTERACTIVE_ENV: &str = "OPENXGRAM_SETUP_NONINTERACTIVE";

/// chat_id 자동 감지 long-poll 타임아웃 (한 번의 getUpdates 호출 기준, 초).
const TG_DETECT_TIMEOUT_SECS: u32 = 25;
/// 전체 감지 시도 한도 — 30초씩 6회 = 약 3분. 그 이후 사용자에게 수동 입력 안내.
const TG_DETECT_MAX_ATTEMPTS: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupTarget {
    Telegram,
    Discord,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupOpts {
    /// `~/.openxgram` 대신 임의 경로 (테스트용).
    pub data_dir: Option<PathBuf>,
    /// 자동 감지 시도 수 — 기본 [`TG_DETECT_MAX_ATTEMPTS`]. 테스트 1회 등.
    pub detect_attempts: Option<u32>,
}

pub async fn run_setup(target: SetupTarget, opts: SetupOpts) -> Result<()> {
    match target {
        SetupTarget::Telegram => run_telegram_setup(opts).await,
        SetupTarget::Discord => run_discord_setup(opts).await,
    }
}

// ── Telegram ─────────────────────────────────────────────────────────────

async fn run_telegram_setup(opts: SetupOpts) -> Result<()> {
    println!("\nOpenXgram Telegram 연결 마법사\n");

    println!("1단계 ─ Telegram 봇 만들기");
    println!("  📱 Telegram 에서 https://t.me/BotFather 로 이동");
    println!("  💬 /newbot 입력 → 봇 이름·username 응답 → 토큰 받기\n");

    let token = read_token_or_env(
        "봇 토큰을 붙여넣고 Enter (예: 1234:ABCDEF...): ",
        SETUP_TOKEN_ENV,
    )?;

    let api_base = telegram_api_base();
    let bot = telegram_get_me(&api_base, &token)
        .await
        .context("Telegram getMe 호출 실패 — 토큰을 다시 확인하세요")?;
    let bot_username = bot
        .username
        .ok_or_else(|| anyhow!("Telegram getMe 응답에 username 누락"))?;
    println!("✓ bot @{bot_username} 검증됨\n");

    println!("2단계 ─ 본인 chat_id 감지");
    println!("  📱 본인 Telegram 에서 @{bot_username} 에게 아무 메시지 (예: /start)");

    let chat_id = if is_noninteractive() {
        std::env::var(SETUP_CHAT_ID_ENV).map_err(|_| {
            anyhow!("비대화 모드 — 환경변수 {SETUP_CHAT_ID_ENV} 가 필요합니다 (chat_id)")
        })?
    } else {
        match telegram_detect_chat_id(
            &api_base,
            &token,
            opts.detect_attempts.unwrap_or(TG_DETECT_MAX_ATTEMPTS),
        )
        .await?
        {
            Some(cid) => {
                println!("✓ chat_id 자동 감지: {cid}\n");
                cid.to_string()
            }
            None => {
                println!("\n  ⚠ 자동 감지 실패 — 수동 입력으로 진행합니다.");
                println!("    Telegram 에서 @userinfobot 에게 /start → 본인 chat_id 확인\n");
                read_token_or_env("chat_id 를 입력하고 Enter: ", SETUP_CHAT_ID_ENV)?
            }
        }
    };

    println!("3단계 ─ 저장 + 테스트");
    let mut config = NotifyConfig::load(opts.data_dir.as_deref())?;
    config.telegram = Some(TelegramConfig {
        bot_token: token.clone(),
        chat_id: chat_id.clone(),
    });
    let saved_path = config.save(opts.data_dir.as_deref())?;
    println!("  ✓ {} 에 저장 (perm 0600)", saved_path.display());

    let test_text = "OpenXgram 연결 성공 ✓ (xgram notify setup-telegram)";
    telegram_send(&api_base, &token, &chat_id, test_text)
        .await
        .context("Telegram 테스트 메시지 전송 실패")?;
    println!("  ✓ 테스트 메시지 전송 완료");
    println!("  📱 Telegram 으로 받았는지 확인하세요\n");

    println!("연결 완료. 이후 명령:");
    println!("  xgram notify telegram --text \"...\"           # 보내기");
    println!("  xgram notify telegram-listen --store-session # 받기");
    Ok(())
}

// ── Discord ──────────────────────────────────────────────────────────────

async fn run_discord_setup(opts: SetupOpts) -> Result<()> {
    println!("\nOpenXgram Discord 연결 마법사\n");

    println!("1단계 ─ Discord 봇 만들기 (수동, 1회)");
    println!("  🌐 https://discord.com/developers/applications 열기");
    println!("  📋 New Application → Bot 탭 → MESSAGE CONTENT INTENT 활성");
    println!("  📋 Bot → Reset Token → 복사\n");

    let token = read_token_or_env("봇 토큰을 붙여넣고 Enter (예: MTI...): ", SETUP_TOKEN_ENV)?;

    let api_base = discord_api_base();
    let bot = discord_get_me(&api_base, &token)
        .await
        .context("Discord /users/@me 호출 실패 — 토큰을 다시 확인하세요")?;
    let bot_label = match (&bot.username, &bot.discriminator) {
        (Some(u), Some(d)) if d != "0" => format!("{u}#{d}"),
        (Some(u), _) => u.clone(),
        _ => "(unknown)".into(),
    };
    println!("✓ bot {bot_label} 검증됨\n");

    println!("2단계 ─ 봇을 서버에 초대");
    println!("  🌐 OAuth2 URL Generator 권한:");
    println!("     bot, applications.commands");
    println!("     Send Messages, Read Message History, View Channels");
    println!("  🔗 생성된 URL 을 브라우저에서 열어 본인 서버에 초대\n");
    if !is_noninteractive() {
        wait_for_enter("서버에 초대했으면 Enter ↵ ")?;
    }

    println!("\n3단계 ─ 채널 ID (옵션, 개발자 모드 → 채널 우클릭 → ID 복사)");
    let channel_id = read_optional_or_env("채널 ID (생략 가능, Enter 만): ", SETUP_CHANNEL_ENV)?;

    println!("\n4단계 ─ Webhook URL (옵션, 송신용)");
    println!("  채널 설정 → 연동 → 웹훅 만들기 → URL 복사");
    let webhook_url =
        read_optional_or_env("webhook URL (생략 가능, Enter 만): ", SETUP_WEBHOOK_ENV)?;

    println!("\n5단계 ─ 저장 + 테스트");
    let mut config = NotifyConfig::load(opts.data_dir.as_deref())?;
    config.discord = Some(DiscordConfig {
        bot_token: token.clone(),
        channel_id: channel_id.clone(),
        webhook_url: webhook_url.clone(),
    });
    let saved_path = config.save(opts.data_dir.as_deref())?;
    println!("  ✓ {} 에 저장 (perm 0600)", saved_path.display());

    let test_text = "OpenXgram 연결 성공 ✓ (xgram notify setup-discord)";
    if let Some(url) = webhook_url.as_deref() {
        discord_send_webhook(url, test_text)
            .await
            .context("Discord webhook 테스트 메시지 전송 실패")?;
        println!("  ✓ webhook 테스트 메시지 전송 완료");
    } else {
        println!("  (webhook 미설정 — 테스트 메시지 생략)");
    }

    println!("\n연결 완료. 이후 명령:");
    println!("  xgram notify discord --text \"...\"   # 보내기 (webhook)");
    println!("  xgram notify discord-listen        # 받기 (Gateway, 후속 PR)");
    Ok(())
}

// ── HTTP helpers ─────────────────────────────────────────────────────────

pub fn telegram_api_base() -> String {
    std::env::var(TELEGRAM_API_BASE_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| TELEGRAM_API_BASE_DEFAULT.into())
}

pub fn discord_api_base() -> String {
    std::env::var(DISCORD_API_BASE_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DISCORD_API_BASE_DEFAULT.into())
}

#[derive(Debug, Deserialize)]
struct TelegramGetMeResp {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    result: Option<TelegramBotInfo>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramBotInfo {
    #[serde(default)]
    pub username: Option<String>,
}

pub async fn telegram_get_me(api_base: &str, token: &str) -> Result<TelegramBotInfo> {
    let url = format!("{}/bot{}/getMe", api_base.trim_end_matches('/'), token);
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("Telegram getMe HTTP {} : {}", status.as_u16(), body);
    }
    let parsed: TelegramGetMeResp = resp.json().await?;
    if !parsed.ok {
        bail!(
            "Telegram getMe ok=false: {}",
            parsed
                .description
                .unwrap_or_else(|| "(no description)".into())
        );
    }
    parsed
        .result
        .ok_or_else(|| anyhow!("Telegram getMe result 누락"))
}

#[derive(Debug, Deserialize)]
struct TelegramGetUpdatesResp {
    ok: bool,
    #[serde(default)]
    result: Option<Vec<TelegramRawUpdate>>,
}

#[derive(Debug, Deserialize)]
struct TelegramRawUpdate {
    #[serde(default)]
    message: Option<TelegramRawMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramRawMessage {
    chat: TelegramRawChat,
}

#[derive(Debug, Deserialize)]
struct TelegramRawChat {
    id: i64,
}

pub async fn telegram_detect_chat_id(
    api_base: &str,
    token: &str,
    max_attempts: u32,
) -> Result<Option<i64>> {
    let max = max_attempts.max(1);
    println!(
        "  ⏳ 첫 메시지를 기다리는 중... (최대 {} 회 × {}초)",
        max, TG_DETECT_TIMEOUT_SECS
    );
    let client = reqwest::Client::new();
    for attempt in 1..=max {
        let url = format!(
            "{}/bot{}/getUpdates?offset=-1&timeout={}",
            api_base.trim_end_matches('/'),
            token,
            TG_DETECT_TIMEOUT_SECS
        );
        let resp = client
            .get(&url)
            .timeout(Duration::from_secs((TG_DETECT_TIMEOUT_SECS + 5) as u64))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Telegram getUpdates HTTP {} : {}", status.as_u16(), body);
        }
        let parsed: TelegramGetUpdatesResp = resp.json().await?;
        if !parsed.ok {
            bail!("Telegram getUpdates ok=false");
        }
        if let Some(updates) = parsed.result {
            if let Some(first) = updates
                .into_iter()
                .find_map(|u| u.message.map(|m| m.chat.id))
            {
                return Ok(Some(first));
            }
        }
        println!("  ... (대기 {attempt}/{max})");
    }
    Ok(None)
}

pub async fn telegram_send(api_base: &str, token: &str, chat_id: &str, text: &str) -> Result<()> {
    let url = format!(
        "{}/bot{}/sendMessage",
        api_base.trim_end_matches('/'),
        token
    );
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        chat_id: &'a str,
        text: &'a str,
    }
    let resp = reqwest::Client::new()
        .post(&url)
        .timeout(Duration::from_secs(15))
        .json(&Payload { chat_id, text })
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("Telegram sendMessage HTTP {} : {}", status.as_u16(), body);
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct DiscordUserResp {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub discriminator: Option<String>,
}

pub async fn discord_get_me(api_base: &str, token: &str) -> Result<DiscordUserResp> {
    let url = format!("{}/users/@me", api_base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bot {token}"))
        .header("User-Agent", "OpenXgram-Setup/0.2")
        .timeout(Duration::from_secs(15))
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("Discord /users/@me HTTP {} : {}", status.as_u16(), body);
    }
    Ok(resp.json().await?)
}

pub async fn discord_send_webhook(url: &str, text: &str) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Payload<'a> {
        content: &'a str,
    }
    let resp = reqwest::Client::new()
        .post(url)
        .timeout(Duration::from_secs(15))
        .json(&Payload { content: text })
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("Discord webhook HTTP {} : {}", status.as_u16(), body);
    }
    Ok(())
}

// ── stdin helpers ────────────────────────────────────────────────────────

fn is_noninteractive() -> bool {
    std::env::var(SETUP_NONINTERACTIVE_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn read_token_or_env(prompt: &str, env_var: &str) -> Result<String> {
    if let Ok(v) = std::env::var(env_var) {
        if !v.is_empty() {
            return Ok(v);
        }
    }
    if is_noninteractive() {
        bail!("비대화 모드 — 환경변수 {env_var} 가 필요합니다");
    }
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    let stdin = io::stdin();
    stdin
        .lock()
        .read_line(&mut line)
        .context("stdin 읽기 실패")?;
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        bail!("입력이 비어있습니다 — 마법사를 다시 실행하세요");
    }
    Ok(trimmed)
}

fn read_optional_or_env(prompt: &str, env_var: &str) -> Result<Option<String>> {
    if let Ok(v) = std::env::var(env_var) {
        if !v.is_empty() {
            return Ok(Some(v));
        }
    }
    if is_noninteractive() {
        return Ok(None);
    }
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    let stdin = io::stdin();
    stdin
        .lock()
        .read_line(&mut line)
        .context("stdin 읽기 실패")?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn wait_for_enter(prompt: &str) -> Result<()> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .context("stdin 읽기 실패")?;
    Ok(())
}

// ── Config (~/.openxgram/notify.toml) ────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NotifyConfig {
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordConfig {
    pub bot_token: String,
    pub channel_id: Option<String>,
    pub webhook_url: Option<String>,
}

impl NotifyConfig {
    pub fn config_path(data_dir: Option<&Path>) -> Result<PathBuf> {
        Ok(resolve_data_dir(data_dir)?.join("notify.toml"))
    }

    pub fn load(data_dir: Option<&Path>) -> Result<Self> {
        let path = Self::config_path(data_dir)?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("notify.toml 읽기 실패: {}", path.display()))?;
        Ok(parse_notify_toml(&raw))
    }

    pub fn save(&self, data_dir: Option<&Path>) -> Result<PathBuf> {
        let path = Self::config_path(data_dir)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("data_dir 생성 실패: {}", parent.display()))?;
        }
        let body = render_notify_toml(self);
        std::fs::write(&path, body)
            .with_context(|| format!("notify.toml 쓰기 실패: {}", path.display()))?;
        set_perm_0600(&path)?;
        Ok(path)
    }
}

fn resolve_data_dir(data_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(d) = data_dir {
        return Ok(d.to_path_buf());
    }
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME 환경변수 누락"))?;
    Ok(PathBuf::from(home).join(".openxgram"))
}

#[cfg(unix)]
fn set_perm_0600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(path)?.permissions();
    perm.set_mode(0o600);
    std::fs::set_permissions(path, perm)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_perm_0600(_path: &Path) -> Result<()> {
    // Windows: ACL 으로 사용자 한정. 마법사 단순함 우선 — no-op 후 경고.
    eprintln!("⚠ Windows: notify.toml perm 제한 미적용 (Unix 0600 동등 필요).");
    Ok(())
}

// ── 미니 TOML 직렬화 ─────────────────────────────────────────────────────
//
// 의존성 추가 없이 단순 key=value 만 다룬다. 토큰·chat_id 등은 ASCII·digit·콜론
// 만 들어오므로 escape 가 거의 필요 없지만, 안전하게 backslash·quote 만 처리.

fn render_notify_toml(cfg: &NotifyConfig) -> String {
    let mut out = String::new();
    out.push_str("# OpenXgram notify.toml — xgram notify setup-* 마법사가 생성·갱신합니다.\n");
    out.push_str("# 평문 저장입니다. 파일 perm 0600 으로 사용자 외 접근 불가.\n");
    out.push_str("# 더 강한 보호가 필요하면 vault put 사용 (별도 PR).\n\n");
    if let Some(t) = &cfg.telegram {
        out.push_str("[telegram]\n");
        out.push_str(&toml_kv("bot_token", &t.bot_token));
        out.push_str(&toml_kv("chat_id", &t.chat_id));
        out.push('\n');
    }
    if let Some(d) = &cfg.discord {
        out.push_str("[discord]\n");
        out.push_str(&toml_kv("bot_token", &d.bot_token));
        if let Some(c) = &d.channel_id {
            out.push_str(&toml_kv("channel_id", c));
        }
        if let Some(w) = &d.webhook_url {
            out.push_str(&toml_kv("webhook_url", w));
        }
        out.push('\n');
    }
    out
}

fn toml_kv(key: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{key} = \"{escaped}\"\n")
}

fn parse_notify_toml(raw: &str) -> NotifyConfig {
    // 단일 섹션 + 단일 key="value" 만 허용. 외부 입력이 아니라 우리가 쓴 파일을
    // 우리가 다시 읽는 시나리오라 단순 파서로 충분.
    let mut cfg = NotifyConfig::default();
    let mut section: Option<String> = None;
    let mut tg_token: Option<String> = None;
    let mut tg_chat: Option<String> = None;
    let mut dc_token: Option<String> = None;
    let mut dc_channel: Option<String> = None;
    let mut dc_webhook: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = Some(trimmed.trim_matches(|c| c == '[' || c == ']').to_string());
            continue;
        }
        if let Some((k, v)) = parse_kv(trimmed) {
            match (section.as_deref(), k.as_str()) {
                (Some("telegram"), "bot_token") => tg_token = Some(v),
                (Some("telegram"), "chat_id") => tg_chat = Some(v),
                (Some("discord"), "bot_token") => dc_token = Some(v),
                (Some("discord"), "channel_id") => dc_channel = Some(v),
                (Some("discord"), "webhook_url") => dc_webhook = Some(v),
                _ => {}
            }
        }
    }
    if let (Some(t), Some(c)) = (tg_token, tg_chat) {
        cfg.telegram = Some(TelegramConfig {
            bot_token: t,
            chat_id: c,
        });
    }
    if let Some(t) = dc_token {
        cfg.discord = Some(DiscordConfig {
            bot_token: t,
            channel_id: dc_channel,
            webhook_url: dc_webhook,
        });
    }
    cfg
}

fn parse_kv(line: &str) -> Option<(String, String)> {
    let (k, rest) = line.split_once('=')?;
    let key = k.trim().to_string();
    let value_raw = rest.trim();
    let unquoted = value_raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))?;
    let value = unquoted.replace("\\\"", "\"").replace("\\\\", "\\");
    Some((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_and_parse_roundtrip() {
        let cfg = NotifyConfig {
            telegram: Some(TelegramConfig {
                bot_token: "1234:ABC".into(),
                chat_id: "999".into(),
            }),
            discord: Some(DiscordConfig {
                bot_token: "MTI...".into(),
                channel_id: Some("777".into()),
                webhook_url: Some("https://discord.com/api/webhooks/1/abc".into()),
            }),
        };
        let body = render_notify_toml(&cfg);
        let back = parse_notify_toml(&body);
        assert_eq!(back, cfg);
    }

    #[test]
    fn parse_partial_telegram_only() {
        let raw = "# comment\n\n[telegram]\nbot_token = \"abc\"\nchat_id = \"42\"\n";
        let cfg = parse_notify_toml(raw);
        assert_eq!(
            cfg.telegram,
            Some(TelegramConfig {
                bot_token: "abc".into(),
                chat_id: "42".into()
            })
        );
        assert!(cfg.discord.is_none());
    }

    #[test]
    fn save_load_roundtrip_writes_file_with_perm_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = NotifyConfig {
            telegram: Some(TelegramConfig {
                bot_token: "T".into(),
                chat_id: "1".into(),
            }),
            discord: None,
        };
        let path = cfg.save(Some(tmp.path())).unwrap();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let loaded = NotifyConfig::load(Some(tmp.path())).unwrap();
        assert_eq!(loaded, cfg);
    }
}
