//! backup_push 통합 테스트.
//!
//! adapter wire-level 은 adapter crate (PR #10) wiremock 통합 테스트에서
//! 보장. 여기서는 cli wiring + env 누락 raise + session 검증.

use std::path::PathBuf;

use openxgram_cli::backup_push::{run_backup_push, BackupPushOpts, BackupTarget};
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "backup-push-test".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
        std::env::remove_var("XGRAM_SEED");
        std::env::remove_var("DISCORD_WEBHOOK_URL");
        std::env::remove_var("TELEGRAM_BOT_TOKEN");
        std::env::remove_var("TELEGRAM_CHAT_ID");
    }
}

#[tokio::test]
async fn discord_requires_webhook_env() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // session 1개 생성
    use openxgram_core::paths::db_path;
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::SessionStore;
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let s = SessionStore::new(&mut db).create("t", "host").unwrap();
    drop(db);

    let err = run_backup_push(BackupPushOpts {
        data_dir,
        session_id: s.id,
        target: BackupTarget::Discord,
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("DISCORD_WEBHOOK_URL"));
}

#[tokio::test]
async fn telegram_requires_token_and_chat_envs() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    use openxgram_core::paths::db_path;
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::SessionStore;
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let s = SessionStore::new(&mut db).create("t", "host").unwrap();
    drop(db);

    let err = run_backup_push(BackupPushOpts {
        data_dir: data_dir.clone(),
        session_id: s.id.clone(),
        target: BackupTarget::Telegram,
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("TELEGRAM_BOT_TOKEN"));

    // token 만 있을 때 chat 누락
    unsafe {
        std::env::set_var("TELEGRAM_BOT_TOKEN", "1:T");
    }
    let err = run_backup_push(BackupPushOpts {
        data_dir,
        session_id: s.id,
        target: BackupTarget::Telegram,
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("TELEGRAM_CHAT_ID"));
}

#[tokio::test]
async fn unknown_session_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_backup_push(BackupPushOpts {
        data_dir,
        session_id: "nonexistent".into(),
        target: BackupTarget::Discord,
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("session 없음"));
}

#[tokio::test]
async fn requires_init_first() {
    set_env();
    let tmp = tempdir().unwrap();
    let err = run_backup_push(BackupPushOpts {
        data_dir: tmp.path().join("absent"),
        session_id: "any".into(),
        target: BackupTarget::Discord,
    })
    .await
    .unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}
