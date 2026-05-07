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
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

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

    println!("→ launching {}", bin.display());
    let status = Command::new(&bin)
        .args(args)
        .status()
        .with_context(|| format!("xgram-desktop 실행 실패: {}", bin.display()))?;

    if !status.success() {
        bail!(
            "xgram-desktop 종료 코드 {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into())
        );
    }
    Ok(())
}
