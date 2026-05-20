//! `GET /v1/gui/sessions` — 머신×세션 통합 detector (UI-MESSENGER-SPEC v1.3 §3.2, M-1).
//!
//! 통합 출처 (M-1):
//! - tmux ls (있을 때만)
//! - ~/.claude/projects 스캔 (Claude Code 세션 history)
//! - xgram session list (SQLite)
//!
//! N6: user-mode. 타 user 프로세스 접근 불가 시 graceful skip.
//! 안티패턴 10: SQL 직접 X — sessions 는 xgram_session_store 거침.

use std::process::Command;
use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct MachineInfo {
    pub hostname: String,
    pub alias: String,                // 기본은 hostname. 사용자 설정 후 alias.
    pub tailscale_ip: Option<String>, // Tailscale CGNAT IP (100.x.y.z)
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Tmux,
    ClaudeProject,
    XgramSession,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Attached,
    Detached,
    Stale,
}

#[derive(Debug, Serialize)]
pub struct DetectedSession {
    pub kind: SessionKind,
    pub identifier: String, // "tmux:starian" / "claude:openxgram" / "xgram:01HN..."
    pub display: String,
    pub status: SessionStatus,
    pub windows: Option<u32>,     // tmux window count
    pub attached: Option<bool>,
    pub created_at: Option<String>,
    pub last_active_at: Option<String>,
    pub agent_id: Option<String>, // ULID — instrumented 면 채움 (Phase 2)
}

#[derive(Debug, Serialize)]
pub struct SessionsDto {
    pub machine: MachineInfo,
    pub sessions: Vec<DetectedSession>,
}

/// `tmux ls -F '#{session_name}|#{session_windows}|#{session_attached}|#{session_created}'`
/// tmux 미설치·없는 세션이면 빈 Vec.
fn detect_tmux() -> Vec<DetectedSession> {
    let out = Command::new("tmux")
        .args([
            "ls",
            "-F",
            "#{session_name}|#{session_windows}|#{session_attached}|#{session_created}",
        ])
        .output();
    let Ok(out) = out else { return vec![] };
    if !out.status.success() {
        return vec![];
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut sessions = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 {
            continue;
        }
        let name = parts[0].to_string();
        let windows: u32 = parts[1].parse().unwrap_or(0);
        let attached: bool = parts[2] != "0";
        // tmux session_created = unix epoch seconds
        let created = parts[3].parse::<i64>().ok().and_then(|ts| {
            chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.to_rfc3339())
        });
        sessions.push(DetectedSession {
            kind: SessionKind::Tmux,
            identifier: format!("tmux:{name}"),
            display: name,
            status: if attached {
                SessionStatus::Attached
            } else {
                SessionStatus::Detached
            },
            windows: Some(windows),
            attached: Some(attached),
            created_at: created,
            last_active_at: None,
            agent_id: None,
        });
    }
    sessions
}

/// `~/.claude/projects/<encoded-path>/*.jsonl` — 각 디렉토리가 Claude Code 프로젝트.
/// 가장 최근 .jsonl 의 mtime 을 last_active 로 사용.
fn detect_claude_projects() -> Vec<DetectedSession> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec![];
    };
    let projects_dir: PathBuf = [home, ".claude/projects".into()].iter().collect();
    let Ok(read) = std::fs::read_dir(&projects_dir) else {
        return vec![];
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // 디렉토리명 = 슬래시를 dash 로 인코딩한 절대경로. "/" 복원.
        let project_path = name.replace('-', "/");
        // 가장 최근 jsonl mtime
        let Ok(files) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        let mut latest: Option<std::time::SystemTime> = None;
        let mut session_count = 0u32;
        for f in files.flatten() {
            if f.path().extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            session_count += 1;
            if let Ok(meta) = f.metadata() {
                if let Ok(m) = meta.modified() {
                    latest = Some(latest.map_or(m, |l| l.max(m)));
                }
            }
        }
        if session_count == 0 {
            continue;
        }
        let last_active_at = latest.and_then(|t| {
            chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339().into()
        });
        let status = match latest {
            Some(t) => {
                let elapsed = std::time::SystemTime::now()
                    .duration_since(t)
                    .map(|d| d.as_secs())
                    .unwrap_or(u64::MAX);
                // 사양 M-4: 15분 idle → 휴면. 1시간 미응답 → stale.
                if elapsed < 900 {
                    SessionStatus::Active
                } else if elapsed < 3600 {
                    SessionStatus::Detached
                } else {
                    SessionStatus::Stale
                }
            }
            None => SessionStatus::Stale,
        };
        let display = std::path::Path::new(&project_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(project_path.clone());
        out.push(DetectedSession {
            kind: SessionKind::ClaudeProject,
            identifier: format!("claude:{}", name),
            display: format!("{} ({} 세션)", display, session_count),
            status,
            windows: Some(session_count),
            attached: None,
            created_at: None,
            last_active_at,
            agent_id: None,
        });
    }
    // 최근 활동 순 정렬
    out.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    out
}

/// Hostname + Tailscale IP. user-mode, 실패 graceful.
pub fn detect_machine() -> MachineInfo {
    let hostname = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());

    // Tailscale IP — `tailscale ip -4`. 없으면 None.
    let tailscale_ip = Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            }
        });

    MachineInfo {
        hostname: hostname.clone(),
        alias: hostname,
        tailscale_ip,
    }
}

/// 통합: 머신 + 세션들. xgram session 은 후속 (DB query needed).
pub fn collect_sessions() -> SessionsDto {
    let machine = detect_machine();
    let mut sessions = detect_tmux();
    sessions.extend(detect_claude_projects());
    SessionsDto { machine, sessions }
}
