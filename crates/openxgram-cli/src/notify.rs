//! xgram notify — Discord (webhook 송신 / Gateway 수신) · Telegram bot 송신.
//!
//! 다중 에이전트 시스템에서 디스코드는 채팅방·라우팅 허브 역할을 한다.
//! webhook 단방향만으로는 부족 → Gateway WebSocket (봇) 으로 메시지 수신.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use openxgram_adapter::{
    Adapter, DiscordGatewayClient, DiscordIncomingMessage, DiscordWebhookAdapter,
    TelegramBotAdapter,
};
use openxgram_core::env::require_password;
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};

const DISCORD_URL_ENV: &str = "DISCORD_WEBHOOK_URL";
const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
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
    /// Discord Gateway 봇 — 채널/DM 수신 (WebSocket).
    DiscordListen {
        bot_token: Option<String>,
        /// 특정 channel 만 받기 (없으면 모든 channel + DM).
        channel_id: Option<u64>,
        /// 받은 메시지를 L0 messages 로 저장. 저장 시 session 이 미리 존재해야 한다
        /// (`xgram session new --title <NAME>` 로 만들고 ID 또는 title 을 넘긴다).
        store_session: Option<String>,
        /// 데이터 디렉토리 (store_session 사용 시 필요).
        data_dir: Option<PathBuf>,
        /// 사람 친화 출력 (false 면 한 줄 JSON).
        pretty: bool,
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
            TelegramBotAdapter::new(token, chat)
                .send_text(&text)
                .await?;
            println!("✓ Telegram 전송 완료 ({} chars)", text.chars().count());
        }
        NotifyAction::DiscordListen {
            bot_token,
            channel_id,
            store_session,
            data_dir,
            pretty,
        } => {
            run_discord_listen(bot_token, channel_id, store_session, data_dir, pretty).await?;
        }
    }
    Ok(())
}

fn resolve(arg: Option<String>, env: &str, flag: &str) -> Result<String> {
    arg.or_else(|| std::env::var(env).ok())
        .ok_or_else(|| anyhow!("{flag} 또는 환경변수 {env} 가 필요합니다"))
}

async fn run_discord_listen(
    bot_token: Option<String>,
    channel_id: Option<u64>,
    store_session: Option<String>,
    data_dir: Option<PathBuf>,
    pretty: bool,
) -> Result<()> {
    let token = resolve(bot_token, DISCORD_BOT_TOKEN_ENV, "--bot-token")?;

    // store-session 모드 사전 준비: keystore 패스워드 + master key 검증, session 존재
    // 검증을 시작 전에 끝낸다 (실패 시 즉시 raise — 절대 규칙 #1 fallback 금지).
    let mut store_ctx = if let Some(session_ref) = &store_session {
        let dir = data_dir
            .clone()
            .ok_or_else(|| anyhow!("--store-session 사용 시 --data-dir 또는 기본 디렉토리 필요"))?;
        Some(StoreContext::open(&dir, session_ref)?)
    } else {
        None
    };

    let client = DiscordGatewayClient::new(token);
    let stream = match channel_id {
        Some(cid) => Box::pin(client.listen_channel(cid).await?)
            as std::pin::Pin<Box<dyn futures_util::Stream<Item = DiscordIncomingMessage> + Send>>,
        None => Box::pin(client.connect().await?)
            as std::pin::Pin<Box<dyn futures_util::Stream<Item = DiscordIncomingMessage> + Send>>,
    };

    eprintln!(
        "✓ Discord Gateway 연결됨 — 메시지 수신 대기 중 (Ctrl+C 종료){}",
        store_ctx
            .as_ref()
            .map(|c| format!(" · L0 store=session/{}", c.session_id))
            .unwrap_or_default()
    );

    let mut stream = stream;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                eprintln!("\n✓ Ctrl+C — 종료");
                return Ok(());
            }
            next = stream.next() => {
                match next {
                    Some(msg) => {
                        emit(&msg, pretty);
                        if let Some(ctx) = store_ctx.as_mut() {
                            if let Err(e) = ctx.store(&msg) {
                                tracing::warn!(error = %e, "L0 저장 실패");
                                eprintln!("⚠ L0 저장 실패: {e:#}");
                            }
                        }
                    }
                    None => {
                        eprintln!("⚠ Discord Gateway stream 종료 (서버 측 disconnect)");
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn emit(msg: &DiscordIncomingMessage, pretty: bool) {
    if pretty {
        println!(
            "[{}] #{} <{}> {}",
            msg.timestamp_kst.format("%Y-%m-%d %H:%M:%S%:z"),
            msg.channel_id,
            msg.author_name,
            msg.content
        );
    } else {
        let line = serde_json::json!({
            "message_id": msg.message_id,
            "channel_id": msg.channel_id,
            "guild_id": msg.guild_id,
            "author_id": msg.author_id,
            "author_name": msg.author_name,
            "content": msg.content,
            "timestamp_kst": msg.timestamp_kst.to_rfc3339(),
        });
        println!("{line}");
    }
}

/// L0 저장 컨텍스트 — keystore + DB + session 검증 후 보관.
struct StoreContext {
    db: Db,
    data_dir: PathBuf,
    session_id: String,
    keystore_password: String,
}

impl StoreContext {
    fn open(data_dir: &Path, session_ref: &str) -> Result<Self> {
        let path = db_path(data_dir);
        if !path.exists() {
            bail!(
                "DB 파일 미존재 ({}). `xgram init --alias <NAME>` 먼저 실행.",
                path.display()
            );
        }
        let mut db = Db::open(DbConfig {
            path,
            ..Default::default()
        })?;
        // session 검색: ID 직접 일치 → 그 다음 title 매칭.
        let mut store = SessionStore::new(&mut db);
        let session = if let Some(s) = store.get_by_id(session_ref)? {
            s
        } else {
            let all = store.list()?;
            all.into_iter()
                .find(|s| s.title == session_ref)
                .ok_or_else(|| {
                    anyhow!(
                        "session 없음: id 또는 title='{session_ref}'. \
                         `xgram session new --title \"{session_ref}\"` 으로 먼저 생성."
                    )
                })?
        };
        let password = require_password()
            .context("XGRAM_KEYSTORE_PASSWORD 가 필요 — keystore 잠금 해제 실패")?;
        // master key 가 로드 가능한지만 검증 (실 메시지마다 다시 로드).
        let ks = FsKeystore::new(keystore_dir(data_dir));
        let _ = ks
            .load(MASTER_KEY_NAME, &password)
            .context("master 키 로드 실패 — keystore 패스워드 확인")?;

        Ok(Self {
            db,
            data_dir: data_dir.to_path_buf(),
            session_id: session.id,
            keystore_password: password,
        })
    }

    fn store(&mut self, msg: &DiscordIncomingMessage) -> Result<()> {
        let ks = FsKeystore::new(keystore_dir(&self.data_dir));
        let kp = ks.load(MASTER_KEY_NAME, &self.keystore_password)?;
        let signature_hex = hex::encode(kp.sign(msg.content.as_bytes()));
        let embedder = default_embedder()?;
        MessageStore::new(&mut self.db, embedder.as_ref()).insert(
            &self.session_id,
            &format!("discord:{}", msg.author_name),
            &msg.content,
            &signature_hex,
        )?;
        Ok(())
    }
}
