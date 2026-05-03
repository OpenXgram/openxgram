//! xgram backup-push — session 통계 텍스트를 Discord/Telegram 으로 전송.
//!
//! Phase 1 first PR: 메타 요약만 push (alias/title/카운트/마지막 활동).
//! 전체 export JSON 분할 push 는 후속 PR (4096/4000자 제한 + file attach).

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_adapter::{Adapter, DiscordWebhookAdapter, TelegramBotAdapter};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{store_stats, EpisodeStore, SessionStore};

#[derive(Debug, Clone, Copy)]
pub enum BackupTarget {
    Discord,
    Telegram,
}

#[derive(Debug, Clone)]
pub struct BackupPushOpts {
    pub data_dir: std::path::PathBuf,
    pub session_id: String,
    pub target: BackupTarget,
}

pub async fn run_backup_push(opts: BackupPushOpts) -> Result<()> {
    let mut db = open_db(&opts.data_dir)?;
    let session = SessionStore::new(&mut db)
        .get_by_id(&opts.session_id)?
        .ok_or_else(|| anyhow!("session 없음: {}", opts.session_id))?;
    let episodes = EpisodeStore::new(&mut db)
        .list_for_session(&opts.session_id)?
        .len();
    let stats = store_stats(&mut db)?;

    let summary = format!(
        "📦 OpenXgram backup\n\
         session: {} ({})\n\
         home_machine: {}\n\
         last_active: {}\n\
         total messages (DB): {} / episodes: {}\n\
         session episodes: {}",
        session.title,
        session.id,
        session.home_machine,
        session.last_active,
        stats.messages,
        stats.episodes,
        episodes,
    );

    match opts.target {
        BackupTarget::Discord => {
            let url = std::env::var("DISCORD_WEBHOOK_URL")
                .map_err(|_| anyhow!("DISCORD_WEBHOOK_URL 환경변수 필요"))?;
            DiscordWebhookAdapter::new(url).send_text(&summary).await?;
            println!("✓ Discord 백업 알림 전송 완료");
        }
        BackupTarget::Telegram => {
            let token = std::env::var("TELEGRAM_BOT_TOKEN")
                .map_err(|_| anyhow!("TELEGRAM_BOT_TOKEN 환경변수 필요"))?;
            let chat_id = std::env::var("TELEGRAM_CHAT_ID")
                .map_err(|_| anyhow!("TELEGRAM_CHAT_ID 환경변수 필요"))?;
            TelegramBotAdapter::new(token, chat_id)
                .send_text(&summary)
                .await?;
            println!("✓ Telegram 백업 알림 전송 완료");
        }
    }
    Ok(())
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    if !path.exists() {
        bail!(
            "DB 미존재 ({}). `xgram init` 먼저 실행.",
            path.display()
        );
    }
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}
