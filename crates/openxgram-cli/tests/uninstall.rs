//! xgram uninstall — 비대화 모드 통합 테스트.
//!
//! init → uninstall 라운드트립 + idempotent + dry-run + 확인 문자열 검증.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::uninstall::{run_uninstall, UninstallOpts};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";
const CONFIRM: &str = "DELETE OPENXGRAM";

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

fn uninstall_opts(data_dir: PathBuf) -> UninstallOpts {
    UninstallOpts {
        data_dir,
        cold_backup_to: None,
        no_backup: true,
        confirm: Some(CONFIRM.into()),
        dry_run: false,
    }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
fn round_trip_init_then_uninstall_leaves_no_data_dir() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");

    run_init(&init_opts(data_dir.clone())).unwrap();
    assert!(data_dir.join("install-manifest.json").exists());

    run_uninstall(&uninstall_opts(data_dir.clone())).unwrap();

    assert!(!data_dir.exists(), "data_dir 가 사라져야 함");
}

#[test]
fn uninstall_idempotent_when_no_manifest() {
    set_env();
    let tmp = tempdir().unwrap();
    // data_dir 자체가 없는 상태에서 uninstall — graceful exit
    run_uninstall(&uninstall_opts(tmp.path().join("absent"))).unwrap();
}

#[test]
fn uninstall_requires_no_backup_in_phase_1() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut o = uninstall_opts(data_dir.clone());
    o.no_backup = false;

    let err = run_uninstall(&o).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("--no-backup"), "msg={msg}");
    assert!(data_dir.exists(), "raise 후 data_dir 보존");
}

#[test]
fn uninstall_requires_confirm_string() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // confirm 없음
    let mut o = uninstall_opts(data_dir.clone());
    o.confirm = None;
    let err = run_uninstall(&o).unwrap_err();
    assert!(format!("{err:#}").contains("--confirm"));

    // confirm 잘못
    let mut o = uninstall_opts(data_dir.clone());
    o.confirm = Some("delete openxgram".into()); // 소문자
    let err = run_uninstall(&o).unwrap_err();
    assert!(format!("{err:#}").contains("불일치"));

    assert!(data_dir.exists(), "둘 모두 raise → data_dir 보존");
}

#[test]
fn uninstall_dry_run_makes_no_changes() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut o = uninstall_opts(data_dir.clone());
    o.dry_run = true;
    run_uninstall(&o).unwrap();

    assert!(data_dir.join("install-manifest.json").exists());
    assert!(data_dir.join("db.sqlite").exists());
}

#[test]
fn uninstall_then_uninstall_idempotent() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_uninstall(&uninstall_opts(data_dir.clone())).unwrap();
    // 두 번째 호출도 OK
    run_uninstall(&uninstall_opts(data_dir.clone())).unwrap();
}
