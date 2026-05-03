//! systemd user unit 생성기 — xgram daemon 을 백그라운드로 띄우기 위한
//! `~/.config/systemd/user/openxgram-sidecar.service` 작성.
//!
//! Phase 1: install / uninstall 만. ExecStart 의 binary 경로는 인자로 받음
//! (기본 `which xgram` 결과). 환경변수(XGRAM_KEYSTORE_PASSWORD)는 사용자가
//! 별도 systemd-creds 또는 EnvironmentFile 로 주입.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

const UNIT_FILENAME: &str = "openxgram-sidecar.service";
const BACKUP_SERVICE_FILENAME: &str = "openxgram-backup.service";
const BACKUP_TIMER_FILENAME: &str = "openxgram-backup.timer";

/// 기본 OnCalendar — 매주 일요일 03:00 KST. systemd 가 로컬 timezone 기준 처리.
pub const DEFAULT_BACKUP_ON_CALENDAR: &str = "Sun 03:00:00";

#[derive(Debug, Clone)]
pub struct UnitOpts {
    /// xgram binary 절대 경로
    pub binary: PathBuf,
    /// daemon 데이터 디렉토리 (--data-dir 인자)
    pub data_dir: PathBuf,
    /// transport bind 주소
    pub bind: String,
}

pub fn default_user_unit_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME 환경변수 누락"))?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join(UNIT_FILENAME))
}

pub fn render_unit(opts: &UnitOpts) -> String {
    format!(
        "# OpenXgram systemd user unit\n\
[Unit]\n\
Description=OpenXgram sidecar daemon\n\
After=network.target\n\
\n\
[Service]\n\
Type=simple\n\
ExecStart={binary} daemon --data-dir {data_dir} --bind {bind}\n\
Restart=on-failure\n\
RestartSec=5\n\
\n\
[Install]\n\
WantedBy=default.target\n",
        binary = opts.binary.display(),
        data_dir = opts.data_dir.display(),
        bind = opts.bind,
    )
}

pub fn install_user_unit(target: &Path, opts: &UnitOpts) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("부모 디렉토리 생성 실패: {}", parent.display()))?;
    }
    if target.exists() {
        bail!(
            "unit 파일 이미 존재: {} — 먼저 uninstall 실행하거나 다른 경로 지정",
            target.display()
        );
    }
    std::fs::write(target, render_unit(opts))
        .with_context(|| format!("unit 파일 저장 실패: {}", target.display()))?;
    Ok(())
}

pub fn uninstall_user_unit(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    std::fs::remove_file(target)
        .with_context(|| format!("unit 파일 제거 실패: {}", target.display()))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BackupUnitOpts {
    /// xgram binary 절대 경로
    pub binary: PathBuf,
    /// 데이터 디렉토리
    pub data_dir: PathBuf,
    /// cold backup 출력 디렉토리 (timestamped 파일 생성됨)
    pub backup_dir: PathBuf,
    /// systemd OnCalendar 표현식 (기본 DEFAULT_BACKUP_ON_CALENDAR)
    pub on_calendar: String,
}

pub fn default_backup_service_path() -> Result<PathBuf> {
    Ok(default_user_unit_path()?
        .parent()
        .ok_or_else(|| anyhow!("user unit 부모 경로 누락"))?
        .join(BACKUP_SERVICE_FILENAME))
}

pub fn default_backup_timer_path() -> Result<PathBuf> {
    Ok(default_user_unit_path()?
        .parent()
        .ok_or_else(|| anyhow!("user unit 부모 경로 누락"))?
        .join(BACKUP_TIMER_FILENAME))
}

pub fn render_backup_service(opts: &BackupUnitOpts) -> String {
    format!(
        "# OpenXgram cold backup oneshot — invoked by openxgram-backup.timer\n\
[Unit]\n\
Description=OpenXgram cold backup\n\
\n\
[Service]\n\
Type=oneshot\n\
ExecStart={binary} backup --data-dir {data_dir} --to {backup_dir}\n",
        binary = opts.binary.display(),
        data_dir = opts.data_dir.display(),
        backup_dir = opts.backup_dir.display(),
    )
}

pub fn render_backup_timer(opts: &BackupUnitOpts) -> String {
    format!(
        "# OpenXgram cold backup timer (KST 기준 OnCalendar — systemd 가 로컬 tz 사용)\n\
[Unit]\n\
Description=OpenXgram cold backup timer\n\
\n\
[Timer]\n\
OnCalendar={on_calendar}\n\
Persistent=true\n\
Unit=openxgram-backup.service\n\
\n\
[Install]\n\
WantedBy=timers.target\n",
        on_calendar = opts.on_calendar,
    )
}

/// service + timer 두 파일을 동시 작성. 둘 중 하나라도 이미 있으면 raise.
pub fn install_backup_units(
    service_path: &Path,
    timer_path: &Path,
    opts: &BackupUnitOpts,
) -> Result<()> {
    if let Some(parent) = service_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("부모 디렉토리 생성 실패: {}", parent.display()))?;
    }
    for p in [service_path, timer_path] {
        if p.exists() {
            bail!(
                "unit 파일 이미 존재: {} — 먼저 backup-uninstall 실행",
                p.display()
            );
        }
    }
    std::fs::write(service_path, render_backup_service(opts))
        .with_context(|| format!("service 저장 실패: {}", service_path.display()))?;
    std::fs::write(timer_path, render_backup_timer(opts))
        .with_context(|| format!("timer 저장 실패: {}", timer_path.display()))?;
    Ok(())
}

pub fn uninstall_backup_units(service_path: &Path, timer_path: &Path) -> Result<()> {
    for p in [service_path, timer_path] {
        if p.exists() {
            std::fs::remove_file(p)
                .with_context(|| format!("unit 파일 제거 실패: {}", p.display()))?;
        }
    }
    Ok(())
}
