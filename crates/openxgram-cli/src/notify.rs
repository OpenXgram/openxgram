//! xgram notify — Discord webhook / Telegram bot 송신 + Telegram 양방향 수신.
//!
//! - notify discord/telegram : 텍스트 송신 (webhook / sendMessage).
//! - notify telegram-listen  : long-polling 으로 받기. 옵션 `--store-session` 으로
//!   받은 메시지를 OpenXgram L0 messages 테이블에 저장 (이후 회상·reflection 대상).
//!
//! Discord 받기는 미구현 — webhook 은 송신 전용. 봇 게이트웨이(WebSocket) 의존성이
//! 크므로 별도 PR.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_adapter::{Adapter, DiscordWebhookAdapter, TelegramBotAdapter, TelegramUpdate};
use openxgram_core::env::require_password;
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keypair, Keystore};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};

const DISCORD_URL_ENV: &str = "DISCORD_WEBHOOK_URL";
const TELEGRAM_TOKEN_ENV: &str = "TELEGRAM_BOT_TOKEN";
const TELEGRAM_CHAT_ENV: &str = "TELEGRAM_CHAT_ID";
/// 테스트·self-host 환경에서 Telegram API base 를 교체할 때 사용.
const TELEGRAM_API_BASE_ENV: &str = "TELEGRAM_API_BASE";

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
    /// Telegram long-polling 으로 받기. 받은 텍스트를 stdout 으로 출력하고,
    /// `store_session_title` 이 있으면 해당 session 에 L0 message 로 저장.
    TelegramListen {
        bot_token: Option<String>,
        /// 지정 시 그 chat 으로 들어온 메시지만 통과.
        chat_id_filter: Option<i64>,
        /// 지정 시 OpenXgram L0 message 로 저장. session title 로 사용.
        store_session_title: Option<String>,
        /// L0 저장 시 사용할 data_dir (None → 기본 ~/.openxgram).
        data_dir: Option<PathBuf>,
        /// 한 번만 polling 후 종료 (테스트·debug 용). 기본 false (Ctrl+C 까지 loop).
        once: bool,
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
            adapter_with_base(TelegramBotAdapter::new(token, chat))
                .send_text(&text)
                .await?;
            println!("✓ Telegram 전송 완료 ({} chars)", text.chars().count());
        }
        NotifyAction::TelegramListen {
            bot_token,
            chat_id_filter,
            store_session_title,
            data_dir,
            once,
        } => {
            let token = resolve(bot_token, TELEGRAM_TOKEN_ENV, "--bot-token")?;
            // chat_id 는 listen 에서 선택. 송신용 placeholder.
            let adapter = adapter_with_base(TelegramBotAdapter::new(token, ""));
            run_telegram_listen(
                adapter,
                chat_id_filter,
                store_session_title.as_deref(),
                data_dir.as_deref(),
                once,
            )
            .await?;
        }
    }
    Ok(())
}

/// `TELEGRAM_API_BASE` 가 설정되면 (테스트·mock) 어댑터 base 교체.
fn adapter_with_base(a: TelegramBotAdapter) -> TelegramBotAdapter {
    if let Ok(base) = std::env::var(TELEGRAM_API_BASE_ENV) {
        if !base.is_empty() {
            return a.with_api_base(base);
        }
    }
    a
}

fn resolve(arg: Option<String>, env: &str, flag: &str) -> Result<String> {
    arg.or_else(|| std::env::var(env).ok())
        .ok_or_else(|| anyhow!("{flag} 또는 환경변수 {env} 가 필요합니다"))
}

// ── Telegram listen ──────────────────────────────────────────────────────

async fn run_telegram_listen(
    adapter: TelegramBotAdapter,
    chat_id_filter: Option<i64>,
    store_session_title: Option<&str>,
    data_dir: Option<&Path>,
    once: bool,
) -> Result<()> {
    println!(
        "✓ Telegram listen 시작 (chat_id_filter={:?}, store_session={:?}, once={})",
        chat_id_filter, store_session_title, once
    );

    // 저장 모드면 미리 DB · keystore · session · embedder 준비.
    let mut store_ctx = if let Some(title) = store_session_title {
        let dir = resolve_data_dir(data_dir)?;
        Some(StoreCtx::open(&dir, title)?)
    } else {
        None
    };

    let stop = Arc::new(AtomicBool::new(false));
    // graceful Ctrl+C. 실패해도 listen 자체는 진행. handle 은 join 하지 않음 (loop 종료 시 자동 drop).
    let _signal_handle = {
        let stop = stop.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                stop.store(true, Ordering::SeqCst);
                eprintln!("\n[telegram-listen] Ctrl+C 감지 — 종료 중...");
            }
        })
    };

    let mut offset: i64 = 0;
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let updates = match adapter.poll_updates(offset, Some(if once { 1 } else { 25 })).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[telegram-listen] poll 오류: {e} — 5초 후 재시도");
                if once {
                    return Err(e.into());
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        for u in &updates {
            offset = offset.max(u.update_id + 1);
            if let Some(filter) = chat_id_filter {
                if u.chat_id != filter {
                    continue;
                }
            }
            handle_update(u, store_ctx.as_mut())?;
        }

        if once {
            break;
        }
    }
    println!("✓ Telegram listen 종료 (마지막 offset={})", offset);
    Ok(())
}

fn handle_update(u: &TelegramUpdate, store: Option<&mut StoreCtx>) -> Result<()> {
    let sender = u.sender_username.as_deref().unwrap_or("(anonymous)");
    println!(
        "[{}] tg chat={} from=@{} update_id={}: {}",
        kst_now().to_rfc3339(),
        u.chat_id,
        sender,
        u.update_id,
        u.text,
    );
    if let Some(ctx) = store {
        ctx.append(u)?;
    }
    Ok(())
}

struct StoreCtx {
    db: Db,
    session_id: String,
    signing_key: Keypair,
}

impl StoreCtx {
    fn open(data_dir: &Path, session_title: &str) -> Result<Self> {
        let path = db_path(data_dir);
        if !path.exists() {
            bail!(
                "DB 파일 미존재 ({}). `xgram init` 먼저 실행.",
                path.display()
            );
        }
        let mut db = Db::open(DbConfig {
            path: path.clone(),
            ..Default::default()
        })
        .with_context(|| format!("DB open 실패: {}", path.display()))?;

        // session ensure_by_title — 없으면 생성. home_machine 은 hostname (cmd_new 와 동일).
        let home_machine = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
        let session = SessionStore::new(&mut db).ensure_by_title(session_title, &home_machine)?;

        // 마스터 키 로드 (서명용). XGRAM_KEYSTORE_PASSWORD 필수.
        let password = require_password()?;
        let ks = FsKeystore::new(keystore_dir(data_dir));
        let kp = ks
            .load(MASTER_KEY_NAME, &password)
            .context("master 키 로드 실패 — keystore 패스워드 확인")?;

        println!(
            "✓ store-session 모드 — session={} ({}), 메시지를 L0 으로 저장합니다.",
            session.id, session_title
        );
        Ok(Self {
            db,
            session_id: session.id,
            signing_key: kp,
        })
    }

    fn append(&mut self, u: &TelegramUpdate) -> Result<()> {
        let sender = format!(
            "telegram:{}",
            u.sender_username.as_deref().unwrap_or("anonymous")
        );
        let signature = hex::encode(self.signing_key.sign(u.text.as_bytes()));
        let embedder = default_embedder()?;
        let msg = MessageStore::new(&mut self.db, embedder.as_ref()).insert(
            &self.session_id,
            &sender,
            &u.text,
            &signature,
        )?;
        println!("  → L0 저장 (id={})", msg.id);
        Ok(())
    }
}

fn resolve_data_dir(opt: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = opt {
        return Ok(p.to_path_buf());
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow!("HOME 환경변수 없음 — --data-dir 명시 필요"))?;
    Ok(PathBuf::from(home).join(".openxgram"))
}
