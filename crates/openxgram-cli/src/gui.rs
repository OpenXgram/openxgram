//! `xgram gui` — GUI(Tauri) 데스크톱 앱 진입점.
//!
//! 별 바이너리 `xgram-desktop` 을 같이 ship 하고, 이 명령은 그걸 exec 한다.
//! 검색 순서:
//!   1. 같은 디렉토리 (`<xgram-binary-dir>/xgram-desktop`) — install.sh 가 둘을 같이 둠
//!   2. PATH 의 `xgram-desktop`
//!   3. dev — 워크스페이스 빌드 산출물 (`target/release/openxgram-desktop`)
//!
//! 셋 다 못 찾으면 빌드 안내 명시 — silent fallback 금지.

use std::env;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use openxgram_core::paths::default_data_dir;

/// `xgram-desktop` 바이너리 이름 (Windows 는 .exe).
fn desktop_binary_name() -> &'static str {
    if cfg!(windows) {
        "xgram-desktop.exe"
    } else {
        "xgram-desktop"
    }
}

fn try_alongside_xgram() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    let cand = dir.join(desktop_binary_name());
    if cand.is_file() {
        Some(cand)
    } else {
        None
    }
}

fn try_path_lookup() -> Option<PathBuf> {
    let name = desktop_binary_name();
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn try_dev_build() -> Option<PathBuf> {
    // `xgram` 자체가 target/release 안이면 거기에 openxgram-desktop 도 있을 가능성.
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    let cand = dir.join("openxgram-desktop");
    if cand.is_file() {
        Some(cand)
    } else {
        None
    }
}

pub fn run_gui(args: &[String]) -> Result<()> {
    let bin = try_alongside_xgram()
        .or_else(try_path_lookup)
        .or_else(try_dev_build)
        .ok_or_else(|| {
            anyhow!(
                "xgram-desktop (GUI 바이너리) 를 찾을 수 없습니다.\n\
                 \n\
                 검색 위치:\n\
                 - xgram 옆 디렉토리\n\
                 - PATH\n\
                 - target/release (dev 빌드)\n\
                 \n\
                 v0.2.0-rc.3 이후 release tarball 에는 GUI 가 포함됩니다 — install.sh 재실행 또는 \
                 GitHub Releases 에서 직접 다운로드.\n\
                 dev 환경: cd ui/tauri && cargo tauri build"
            )
        })?;

    // 백그라운드 detach — GUI 는 자체 윈도우 루프를 가지므로 부모 CLI 가 기다릴 필요 없음.
    // stdin 은 닫고 stdout/stderr 는 data_dir/desktop.log 로 redirect (디버깅용).
    let log_path = default_data_dir()
        .map(|d| d.join("desktop.log"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/xgram-desktop.log"));
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("desktop log 파일 생성 실패: {}", log_path.display()))?;
    let log_err = log_file
        .try_clone()
        .context("desktop log 파일 핸들 복제 실패")?;

    let child = Command::new(&bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .spawn()
        .with_context(|| format!("xgram-desktop 실행 실패: {}", bin.display()))?;

    let pid = child.id();
    drop(child); // wait 안 함 — 백그라운드 운영. 부모 종료해도 OS 가 reparent.

    println!("✓ xgram-desktop 백그라운드 가동 (PID {pid})");
    println!("  바이너리 : {}", bin.display());
    println!("  로그     : {}", log_path.display());
    println!("  종료     : kill {pid}    또는 GUI 창 닫기");
    Ok(())
}
