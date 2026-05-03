//! systemd unit 생성기 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::systemd::{
    install_user_unit, render_unit, uninstall_user_unit, UnitOpts,
};
use tempfile::tempdir;

fn sample_opts() -> UnitOpts {
    UnitOpts {
        binary: PathBuf::from("/usr/local/bin/xgram"),
        data_dir: PathBuf::from("/home/user/.openxgram"),
        bind: "127.0.0.1:7300".into(),
    }
}

#[test]
fn render_unit_includes_required_sections() {
    let unit = render_unit(&sample_opts());
    assert!(unit.contains("[Unit]"));
    assert!(unit.contains("[Service]"));
    assert!(unit.contains("[Install]"));
    assert!(unit.contains("ExecStart=/usr/local/bin/xgram daemon"));
    assert!(unit.contains("--bind 127.0.0.1:7300"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
fn install_creates_file_with_unit_content() {
    let tmp = tempdir().unwrap();
    let target = tmp.path().join("openxgram-sidecar.service");
    install_user_unit(&target, &sample_opts()).unwrap();
    let body = std::fs::read_to_string(&target).unwrap();
    assert!(body.contains("[Service]"));
}

#[test]
fn install_existing_target_raises() {
    let tmp = tempdir().unwrap();
    let target = tmp.path().join("openxgram-sidecar.service");
    install_user_unit(&target, &sample_opts()).unwrap();
    let result = install_user_unit(&target, &sample_opts());
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("이미 존재"));
}

#[test]
fn install_creates_parent_dirs() {
    let tmp = tempdir().unwrap();
    let nested = tmp.path().join("a").join("b").join("c").join("unit.service");
    install_user_unit(&nested, &sample_opts()).unwrap();
    assert!(nested.exists());
}

#[test]
fn uninstall_removes_existing_file() {
    let tmp = tempdir().unwrap();
    let target = tmp.path().join("unit.service");
    install_user_unit(&target, &sample_opts()).unwrap();
    assert!(target.exists());
    uninstall_user_unit(&target).unwrap();
    assert!(!target.exists());
}

#[test]
fn uninstall_missing_target_is_idempotent() {
    let tmp = tempdir().unwrap();
    // 존재하지 않는 파일에 uninstall 호출 — raise 안 함
    uninstall_user_unit(&tmp.path().join("absent.service")).unwrap();
}
