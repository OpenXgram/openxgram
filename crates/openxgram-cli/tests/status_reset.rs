//! xgram status + reset 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::reset::{run_reset, ResetOpts};
use openxgram_cli::status::{run_status, StatusOpts};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "test-machine".into(),
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
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
#[serial_test::file_serial]
fn status_after_init_succeeds() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_status(&StatusOpts {
        data_dir: data_dir.clone(),
    })
    .unwrap();
}

#[test]
#[serial_test::file_serial]
fn status_without_install_is_idempotent() {
    let tmp = tempdir().unwrap();
    // 미설치 디렉토리 — 안내 출력 후 OK 반환
    run_status(&StatusOpts {
        data_dir: tmp.path().join("absent"),
    })
    .unwrap();
}

#[test]
#[serial_test::file_serial]
fn reset_hard_round_trip_then_init_again() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");

    run_init(&init_opts(data_dir.clone())).unwrap();

    run_reset(&ResetOpts {
        data_dir: data_dir.clone(),
        hard: true,
        confirm: Some("RESET OPENXGRAM".into()),
        dry_run: false,
    })
    .unwrap();

    assert!(!data_dir.exists(), "reset --hard 후 data_dir 사라짐");

    // 재설치 가능
    run_init(&init_opts(data_dir.clone())).unwrap();
    assert!(data_dir.join("install-manifest.json").exists());
}

#[test]
#[serial_test::file_serial]
fn reset_requires_hard() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_reset(&ResetOpts {
        data_dir: data_dir.clone(),
        hard: false,
        confirm: Some("RESET OPENXGRAM".into()),
        dry_run: false,
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("--hard"));
    assert!(data_dir.exists());
}

#[test]
#[serial_test::file_serial]
fn reset_requires_confirm_string() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // confirm 없음
    let err = run_reset(&ResetOpts {
        data_dir: data_dir.clone(),
        hard: true,
        confirm: None,
        dry_run: false,
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("--confirm"));

    // 잘못된 confirm
    let err = run_reset(&ResetOpts {
        data_dir: data_dir.clone(),
        hard: true,
        confirm: Some("DELETE OPENXGRAM".into()), // uninstall 문자열, reset 아님
        dry_run: false,
    })
    .unwrap_err();
    assert!(format!("{err:#}").contains("불일치"));

    assert!(data_dir.exists(), "raise 후 data_dir 보존");
}

#[test]
#[serial_test::file_serial]
fn reset_dry_run_makes_no_changes() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_reset(&ResetOpts {
        data_dir: data_dir.clone(),
        hard: true,
        confirm: Some("RESET OPENXGRAM".into()),
        dry_run: true,
    })
    .unwrap();

    assert!(data_dir.join("install-manifest.json").exists());
    assert!(data_dir.join("db.sqlite").exists());
}
