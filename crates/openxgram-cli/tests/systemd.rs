//! systemd unit 생성기 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::systemd::{
    install_backup_units, install_user_unit, render_backup_service, render_backup_timer,
    render_unit, uninstall_backup_units, uninstall_user_unit, BackupUnitOpts,
    DEFAULT_BACKUP_ON_CALENDAR, UnitOpts,
};
use tempfile::tempdir;

fn sample_backup_opts() -> BackupUnitOpts {
    BackupUnitOpts {
        binary: PathBuf::from("/usr/local/bin/xgram"),
        data_dir: PathBuf::from("/home/user/.openxgram"),
        backup_dir: PathBuf::from("/home/user/.openxgram/backups"),
        on_calendar: DEFAULT_BACKUP_ON_CALENDAR.into(),
    }
}

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

#[test]
fn render_backup_service_invokes_xgram_backup() {
    let s = render_backup_service(&sample_backup_opts());
    assert!(s.contains("Type=oneshot"));
    assert!(s.contains("/usr/local/bin/xgram backup"));
    assert!(s.contains("--data-dir /home/user/.openxgram"));
    assert!(s.contains("--to /home/user/.openxgram/backups"));
}

#[test]
fn render_backup_timer_uses_oncalendar_and_persistent() {
    let t = render_backup_timer(&sample_backup_opts());
    assert!(t.contains("OnCalendar=Sun 03:00:00"));
    assert!(t.contains("Persistent=true"));
    assert!(t.contains("Unit=openxgram-backup.service"));
    assert!(t.contains("WantedBy=timers.target"));
}

#[test]
fn install_backup_units_creates_both_files() {
    let tmp = tempdir().unwrap();
    let svc = tmp.path().join("openxgram-backup.service");
    let tim = tmp.path().join("openxgram-backup.timer");
    install_backup_units(&svc, &tim, &sample_backup_opts()).unwrap();
    assert!(svc.exists() && tim.exists());
    assert!(std::fs::read_to_string(&svc).unwrap().contains("Type=oneshot"));
    assert!(std::fs::read_to_string(&tim).unwrap().contains("OnCalendar="));
}

#[test]
fn install_backup_units_raises_when_either_exists() {
    let tmp = tempdir().unwrap();
    let svc = tmp.path().join("openxgram-backup.service");
    let tim = tmp.path().join("openxgram-backup.timer");
    std::fs::write(&svc, "stale").unwrap();
    let err = install_backup_units(&svc, &tim, &sample_backup_opts()).unwrap_err();
    assert!(format!("{err:#}").contains("이미 존재"));
    // timer 는 작성되지 않아야 함
    assert!(!tim.exists());
}

#[test]
fn uninstall_backup_units_removes_both_idempotent() {
    let tmp = tempdir().unwrap();
    let svc = tmp.path().join("openxgram-backup.service");
    let tim = tmp.path().join("openxgram-backup.timer");
    install_backup_units(&svc, &tim, &sample_backup_opts()).unwrap();
    uninstall_backup_units(&svc, &tim).unwrap();
    assert!(!svc.exists() && !tim.exists());
    // 재호출 idempotent
    uninstall_backup_units(&svc, &tim).unwrap();
}
