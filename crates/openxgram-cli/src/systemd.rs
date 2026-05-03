//! systemd user unit 생성기 — xgram daemon 을 백그라운드로 띄우기 위한
//! `~/.config/systemd/user/openxgram-sidecar.service` 작성.
//!
//! Phase 1: install / uninstall 만. ExecStart 의 binary 경로는 인자로 받음
//! (기본 `which xgram` 결과). 환경변수(XGRAM_KEYSTORE_PASSWORD)는 사용자가
//! 별도 systemd-creds 또는 EnvironmentFile 로 주입.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

const UNIT_FILENAME: &str = "openxgram-sidecar.service";

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
        std::fs::create_dir_all(parent).with_context(|| {
            format!("부모 디렉토리 생성 실패: {}", parent.display())
        })?;
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
