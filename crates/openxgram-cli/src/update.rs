//! xgram update — self-update wrapper.
//!
//! Windows: PowerShell 의 `install.ps1` 위임 (Task/Service auto stop+restart 자동화 포함).
//! Linux/macOS: `install.sh` 위임 + systemd user service restart.
//!
//! rc.168 — 표준 운영. 사용자가 매번 web install 안 하고 `xgram update` 한 줄.
//! rc.213 — install 전에 모든 xgram daemon process 강제 kill (systemd 외 mechanism 도 cover).

use anyhow::{anyhow, Context, Result};
use std::process::Command;

/// rc.213 — 모든 xgram daemon process 강제 kill (self PID 제외).
/// systemd service 외 mechanism (nohup, init script, cron, ppid=1 orphan 등) 으로
/// boot 된 옛 daemon 도 cover. path 무관 — 'xgram' + 'daemon' 둘 다 cmdline 에 포함된 것만 매칭.
#[cfg(not(target_os = "windows"))]
fn kill_all_xgram_daemons() -> usize {
    let my_pid = std::process::id();
    let mut killed = 0usize;
    let out = match Command::new("ps").args(["-eo", "pid,cmd"]).output() {
        Ok(o) => o,
        Err(_) => return 0,
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let trimmed = line.trim();
        let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
        if parts.len() < 2 {
            continue;
        }
        let pid: u32 = match parts[0].parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if pid == my_pid {
            continue;
        }
        let cmd = parts[1];
        // xgram daemon process 매칭 (path 무관)
        if cmd.contains("xgram") && cmd.contains("daemon") {
            println!("    -> 옛 daemon kill: pid={pid} cmd={cmd}");
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
            killed += 1;
        }
    }
    killed
}

/// rc.213 — Windows path: 모든 xgram.exe process kill (self 제외).
#[cfg(target_os = "windows")]
fn kill_all_xgram_daemons_windows() {
    let my_pid = std::process::id();
    let ps_cmd = format!(
        "Get-Process -Name xgram -EA SilentlyContinue | Where-Object {{ $_.Id -ne {} }} | Stop-Process -Force -EA SilentlyContinue",
        my_pid
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_cmd])
        .status();
}

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
    // rc.213 — install 전 모든 xgram.exe daemon 강제 kill
    println!("    -> 옛 daemon process 강제 kill (rc.213)");
    kill_all_xgram_daemons_windows();

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

    // rc.213 — install 전 모든 xgram daemon process 강제 kill (systemd 외 mechanism 도 cover)
    let killed = kill_all_xgram_daemons();
    if killed > 0 {
        println!("    -> 옛 daemon process 강제 kill (rc.213): {killed} process");
    }

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
