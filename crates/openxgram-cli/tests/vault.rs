//! xgram vault CLI 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::vault::{run_vault, VaultAction};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "vault-cli-test".into(),
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
fn set_then_list_and_delete() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_vault(
        &data_dir,
        VaultAction::Set {
            key: "discord/bot".into(),
            value: "TOKEN_VALUE".into(),
            tags: vec!["discord".into(), "prod".into()],
        },
    )
    .unwrap();

    run_vault(&data_dir, VaultAction::List).unwrap();
    run_vault(
        &data_dir,
        VaultAction::Delete {
            key: "discord/bot".into(),
        },
    )
    .unwrap();
}

#[test]
fn requires_init_first() {
    let tmp = tempdir().unwrap();
    let err = run_vault(
        &tmp.path().join("absent"),
        VaultAction::List,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}

#[test]
fn delete_unknown_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let err = run_vault(
        &data_dir,
        VaultAction::Delete {
            key: "nonexistent".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("nonexistent"));
}

#[test]
fn get_unknown_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let err = run_vault(
        &data_dir,
        VaultAction::Get {
            key: "nonexistent".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("nonexistent"));
}
