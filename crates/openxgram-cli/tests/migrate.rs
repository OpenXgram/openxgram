//! xgram migrate cli 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::migrate::{run_migrate, MigrateOpts};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "migrate-test".into(),
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
    }
}

#[test]
fn migrate_after_init_idempotent() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // init 이 이미 migrate 호출 — 다시 호출해도 idempotent
    run_migrate(&MigrateOpts {
        data_dir: data_dir.clone(),
        target: None,
    })
    .unwrap();
    run_migrate(&MigrateOpts {
        data_dir,
        target: None,
    })
    .unwrap();
}

#[test]
fn migrate_without_init_raises() {
    let tmp = tempdir().unwrap();
    let err = run_migrate(&MigrateOpts {
        data_dir: tmp.path().join("absent"),
        target: None,
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}

#[test]
fn migrate_with_target_warns_but_proceeds() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // Phase 1.5 까지 무시되지만 raise 안 함
    run_migrate(&MigrateOpts {
        data_dir,
        target: Some("2".into()),
    })
    .unwrap();
}
