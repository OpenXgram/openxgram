//! xgram update — self-update wrapper.
//!
//! Windows: PowerShell 의 `install.ps1` 위임 (Task/Service auto stop+restart 자동화 포함).
//! Linux/macOS: `install.sh` 위임 + systemd user service restart.
//!
//! rc.168 — 표준 운영. 사용자가 매번 web install 안 하고 `xgram update` 한 줄.

use anyhow::{anyhow, Context, Result};
use std::process::Command;

pub fn run_update(version: Option<String>) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("==> xgram update");
    println!("    current : v{current}");
    if let Some(v) = &version {
        println!("    target  : {v}");
    } else {
        println!("    target  : latest");
    }
    println!();

    let target = version.unwrap_or_else(|| "latest".to_string());

    if cfg!(windows) {
        run_windows_update(&target)
    } else {
        run_unix_update(&target)
    }
}

#[cfg(target_os = "windows")]
fn run_windows_update(target: &str) -> Result<()> {
    let env_prefix = if target != "latest" {
        format!("$env:OPENXGRAM_VERSION=\"{target}\"; ")
    } else {
        String::new()
    };
    let cmd = format!("{env_prefix}irm https://openxgram.org/install.ps1 | iex");
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &cmd])
        .status()
        .context("powershell.exe 실행 실패")?;
    if !status.success() {
        return Err(anyhow!("install.ps1 실패: exit code {:?}", status.code()));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn run_windows_update(_target: &str) -> Result<()> {
    Err(anyhow!("Windows 가 아닌 환경에서 Windows update 호출됨"))
}

#[cfg(not(target_os = "windows"))]
fn run_unix_update(target: &str) -> Result<()> {
    let env_prefix = if target != "latest" {
        format!("OPENXGRAM_VERSION={target} ")
    } else {
        String::new()
    };
    let cmd = format!("{env_prefix}curl -sSfL https://openxgram.org/install.sh | sh");

    // systemd user daemon 정지 (있으면)
    let services = ["openxgram-sidecar.service", "openxgram-mcp-serve.service"];
    let mut stopped: Vec<&str> = Vec::new();
    for svc in &services {
        let status = Command::new("systemctl")
            .args(["--user", "is-active", svc])
            .status();
        if let Ok(s) = status {
            if s.success() {
                let _ = Command::new("systemctl")
                    .args(["--user", "stop", svc])
                    .status();
                stopped.push(svc);
                println!("    -> stopped {svc}");
            }
        }
    }

    // install
    let status = Command::new("sh")
        .args(["-c", &cmd])
        .status()
        .context("install.sh 실행 실패")?;
    if !status.success() {
        return Err(anyhow!("install.sh 실패: exit code {:?}", status.code()));
    }

    // 재시작
    for svc in &stopped {
        let _ = Command::new("systemctl")
            .args(["--user", "start", svc])
            .status();
        println!("    -> started {svc}");
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn run_unix_update(_target: &str) -> Result<()> {
    Err(anyhow!("Unix 가 아닌 환경에서 Unix update 호출됨"))
}
