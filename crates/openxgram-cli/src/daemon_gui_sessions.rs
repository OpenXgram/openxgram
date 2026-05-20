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

/// UI-MESSENGER-SPEC v1.3 M-1 §3.5 — ps -ef 로 LLM 관련 프로세스 감지.
/// claude / codex / ollama / xgram agent / mcp-serve 등.
/// N6: user-mode. 다른 user 의 environ 접근 불가 시 skip.
fn detect_processes() -> Vec<DetectedSession> {
    let out = Command::new("ps")
        .args(["-eo", "pid,user,etime,cmd", "--sort=-etime"])
        .output();
    let Ok(out) = out else { return vec![] };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let me = std::env::var("USER").unwrap_or_default();
    let mut sessions = Vec::new();
    for line in stdout.lines().skip(1) {
        // Parse: <pid> <user> <etime> <cmd...>
        let mut parts = line.splitn(4, |c: char| c == ' ' || c == '\t').filter(|s| !s.is_empty());
        let pid_str = parts.next().unwrap_or("");
        let user = parts.next().unwrap_or("");
        let etime = parts.next().unwrap_or("");
        let cmd = parts.next().unwrap_or("");
        // 자기 user 만 (N6).
        if !me.is_empty() && user != me {
            continue;
        }
        // 관심 패턴
        let kind_label = if cmd.starts_with("claude") || cmd.contains("/claude ") {
            Some("claude")
        } else if cmd.starts_with("codex") || cmd.contains("/codex ") {
            Some("codex")
        } else if cmd.starts_with("ollama") || cmd.contains("ollama serve") {
            Some("ollama")
        } else if cmd.contains("xgram agent") || cmd.contains("xgram mcp-serve") {
            Some("xgram-proc")
        } else if cmd.contains("aider") {
            Some("aider")
        } else {
            None
        };
        let Some(label) = kind_label else { continue };
        let Ok(pid) = pid_str.parse::<u32>() else { continue };
        sessions.push(DetectedSession {
            kind: SessionKind::XgramSession, // 통합 표시 (proc는 별 enum 추가 가능)
            identifier: format!("proc:{label}:{pid}"),
            display: format!("{label} (pid {pid})"),
            status: SessionStatus::Active,
            windows: None,
            attached: None,
            created_at: None,
            last_active_at: Some(format!("etime {etime}")),
            agent_id: None,
        });
    }
    sessions
}

/// 통합: 머신 + 세션들. xgram session 은 후속 (DB query needed).
pub fn collect_sessions() -> SessionsDto {
    let machine = detect_machine();
    let mut sessions = detect_tmux();
    sessions.extend(detect_claude_projects());
    sessions.extend(detect_processes());
    SessionsDto { machine, sessions }
}

/// UI-MESSENGER-SPEC v1.3 §4.3 — 세션 클릭 시 중앙 패널 라이브 터미널 (S5 xterm.js).
/// identifier 예: "tmux:starian" / "claude:-home-llm-projects-wgolf"
#[derive(Debug, Serialize)]
pub struct SessionScreenDto {
    pub identifier: String,
    pub kind: SessionKind,
    pub display: String,
    pub content: String,   // ANSI escape 포함 (xterm.js writeUtf8)
    pub lines: u32,
    pub source_note: String, // "tmux capture-pane -e" 또는 "Claude Code .jsonl tail" 등
    pub fetched_at: String,
}

/// `tmux capture-pane -t <session> -p -e -E -` (escape 포함, 전체 history 까지).
fn capture_tmux(session_name: &str) -> Result<String, String> {
    let out = Command::new("tmux")
        .args([
            "capture-pane",
            "-t",
            session_name,
            "-p",
            "-e",
            "-S",
            "-200", // 마지막 200 줄
        ])
        .output()
        .map_err(|e| format!("tmux capture-pane 실패: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        return Err(format!("tmux: {}", err.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Claude Code 프로젝트 디렉토리의 가장 최근 .jsonl 의 마지막 N 줄 (50줄).
/// 각 줄 = 한 메시지 (system/user/assistant + content).
fn tail_claude_jsonl(project_dir_name: &str) -> Result<String, String> {
    let home = std::env::var_os("HOME").ok_or("HOME unset")?;
    let dir: PathBuf = [home, ".claude/projects".into(), project_dir_name.into()]
        .iter()
        .collect();
    let read = std::fs::read_dir(&dir).map_err(|e| format!("read_dir {dir:?}: {e}"))?;
    // 가장 최근 .jsonl 선택
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    for f in read.flatten() {
        if f.path().extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(meta) = f.metadata() {
            if let Ok(m) = meta.modified() {
                let p = f.path();
                latest = Some(match latest {
                    Some((t, _)) if t > m => (t, latest.unwrap().1),
                    _ => (m, p),
                });
            }
        }
    }
    let Some((_, path)) = latest else {
        return Ok("(빈 프로젝트)".into());
    };
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
    let lines: Vec<&str> = content.lines().collect();
    let tail = &lines[lines.len().saturating_sub(50)..];
    // 각 줄 = JSON. type/role/content 만 추출해서 사람 친화 포맷.
    let mut out = String::new();
    for line in tail {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let kind = v
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("?");
        let role = v
            .get("message")
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            .unwrap_or(kind);
        let body = v
            .get("message")
            .and_then(|m| m.get("content"))
            .map(|c| match c {
                serde_json::Value::String(s) => s.clone(),
                _ => c.to_string(),
            })
            .unwrap_or_default();
        // 256줄 cap per message
        let trimmed: String = body.chars().take(2000).collect();
        out.push_str(&format!(
            "\x1b[36m[{ts}]\x1b[0m \x1b[1m{role}\x1b[0m\n{trimmed}\n\n"
        ));
    }
    if out.is_empty() {
        out = format!("(파싱 가능한 메시지 없음 — {} bytes)", content.len());
    }
    Ok(out)
}

/// UI-MESSENGER-SPEC v1.3 §7.1 + §7.3 — 헤더 🔔 통합 승인 큐 (L6 차등 만료 + V4).
///
/// 큐 유형: payment / pending_session / risky_action / external_call / channel_moderation.
/// 만료: payment 24h / pending_session 7d / risky_action 1h / external_call 24h / channel_moderation 7d.
/// 만료 시 자동 거절. 화이트리스트 매칭 시 만료 전 자동 승인 (V4).
///
/// MVP: 기존 단일 큐 (`gui_vault_pending`) 를 통합 형태로 노출. 다른 큐 종류는 placeholder.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Payment,
    PendingSession,
    RiskyAction,
    ExternalCall,
    ChannelModeration,
}

impl ApprovalKind {
    pub fn ttl_hours(&self) -> u32 {
        match self {
            ApprovalKind::Payment => 24,
            ApprovalKind::PendingSession => 24 * 7,
            ApprovalKind::RiskyAction => 1,
            ApprovalKind::ExternalCall => 24,
            ApprovalKind::ChannelModeration => 24 * 7,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ApprovalItem {
    pub id: String,
    pub kind: ApprovalKind,
    pub title: String,
    pub detail: String,
    pub created_at: String,
    pub expires_at: String,
    pub source_card: String, // "vault" / "messenger" / "external" 등 cross-link 용
}

#[derive(Debug, Serialize)]
pub struct ApprovalQueueDto {
    pub items: Vec<ApprovalItem>,
    pub policy: ApprovalPolicy,
}

#[derive(Debug, Serialize)]
pub struct ApprovalPolicy {
    pub payment_ttl_hours: u32,
    pub pending_session_ttl_hours: u32,
    pub risky_action_ttl_hours: u32,
    pub external_call_ttl_hours: u32,
    pub channel_moderation_ttl_hours: u32,
    pub auto_approve_on_whitelist_match: bool, // V4
    pub never_auto_approve: Vec<String>,       // ["payment", "risky_action"] — V4
}

/// UI-MESSENGER-SPEC v1.3 L3 + V1 — 역할별 auto_respond 기본 정책.
/// 마스터 = ⏰ 자율 행동 카드. 메신저 탭 2 는 view·override.
/// V1: 향후 RolePolicy struct 형태 (max_concurrent 등 추가 예정).
#[derive(Debug, Serialize)]
pub struct RolePolicyItem {
    pub role: String,
    pub auto_respond_default: bool,
    pub max_concurrent: u32,
}

#[derive(Debug, Serialize)]
pub struct RolePolicyDto {
    pub master_card: String,
    pub roles: Vec<RolePolicyItem>,
}

/// UI-MESSENGER-SPEC v1.3 §3.6 M-5 + N1 + N3 — 화이트리스트 패턴.
#[derive(Debug, Serialize)]
pub struct WhitelistPatternItem {
    pub priority: u32,
    pub pattern_type: String, // "command" | "tmux" | "cwd"
    pub pattern: String,
    pub default_role: String,
    pub auto_register: bool,
    pub auto_approve_pending: bool, // N3 + V4
}

#[derive(Debug, Serialize)]
pub struct WhitelistDto {
    pub patterns: Vec<WhitelistPatternItem>,
    pub priority_order: Vec<String>, // N1
    pub never_auto_approve: Vec<String>, // V4
}

/// UI-MESSENGER-SPEC v1.3 S8 + V6 — cross-machine 큐 영구화 status.
#[derive(Debug, Serialize)]
pub struct CrossMachineQueueDto {
    pub backend: String,        // "Tailscale P2P"
    pub queue_path: String,     // "~/.openxgram/outbound_queue.db"
    pub max_retention_days: u32, // 30 (V6)
    pub retry_backoff: String,  // "1s -> 2s -> 4s ... max 5min"
    pub dedup_strategy: String, // "message ULID"
    pub pending: u32,           // 향후 실시간; 현재 0
    pub last_sent_at: Option<String>,
}

pub fn default_cross_machine_queue() -> CrossMachineQueueDto {
    CrossMachineQueueDto {
        backend: "Tailscale P2P".into(),
        queue_path: "~/.openxgram/outbound_queue.db".into(),
        max_retention_days: 30,
        retry_backoff: "exponential 1s -> 5min".into(),
        dedup_strategy: "message ULID".into(),
        pending: 0,
        last_sent_at: None,
    }
}

pub fn default_whitelist() -> WhitelistDto {
    WhitelistDto {
        patterns: vec![
            WhitelistPatternItem {
                priority: 1,
                pattern_type: "command".into(),
                pattern: "claude *".into(),
                default_role: "llm-attached".into(),
                auto_register: true,
                auto_approve_pending: true,
            },
            WhitelistPatternItem {
                priority: 2,
                pattern_type: "tmux".into(),
                pattern: "xgram-*".into(),
                default_role: "researcher".into(),
                auto_register: true,
                auto_approve_pending: false,
            },
            WhitelistPatternItem {
                priority: 3,
                pattern_type: "cwd".into(),
                pattern: "~/projects/*/".into(),
                default_role: "coder".into(),
                auto_register: false, // confirm
                auto_approve_pending: false,
            },
        ],
        priority_order: vec!["command".into(), "tmux".into(), "cwd".into()],
        never_auto_approve: vec!["payment".into(), "risky_action".into()],
    }
}

pub fn default_role_policies() -> RolePolicyDto {
    RolePolicyDto {
        master_card: "⏰ 자율 행동".into(),
        roles: vec![
            RolePolicyItem { role: "researcher".into(), auto_respond_default: true, max_concurrent: 3 },
            RolePolicyItem { role: "reviewer".into(), auto_respond_default: false, max_concurrent: 2 },
            RolePolicyItem { role: "coder".into(), auto_respond_default: true, max_concurrent: 2 },
            RolePolicyItem { role: "orchestrator".into(), auto_respond_default: true, max_concurrent: 5 },
            RolePolicyItem { role: "scribe".into(), auto_respond_default: true, max_concurrent: 1 },
            RolePolicyItem { role: "analyst".into(), auto_respond_default: true, max_concurrent: 2 },
            RolePolicyItem { role: "tester".into(), auto_respond_default: false, max_concurrent: 2 },
            RolePolicyItem { role: "ops".into(), auto_respond_default: false, max_concurrent: 1 },
        ],
    }
}

pub fn default_approval_policy() -> ApprovalPolicy {
    ApprovalPolicy {
        payment_ttl_hours: ApprovalKind::Payment.ttl_hours(),
        pending_session_ttl_hours: ApprovalKind::PendingSession.ttl_hours(),
        risky_action_ttl_hours: ApprovalKind::RiskyAction.ttl_hours(),
        external_call_ttl_hours: ApprovalKind::ExternalCall.ttl_hours(),
        channel_moderation_ttl_hours: ApprovalKind::ChannelModeration.ttl_hours(),
        auto_approve_on_whitelist_match: true,
        never_auto_approve: vec!["payment".into(), "risky_action".into()],
    }
}

/// `GET /v1/gui/sessions/{identifier}/screen` 의 핵심 로직.
pub fn capture_session(identifier: &str) -> SessionScreenDto {
    let (kind, content, source_note) = if let Some(name) = identifier.strip_prefix("tmux:") {
        match capture_tmux(name) {
            Ok(s) => (
                SessionKind::Tmux,
                s,
                format!("tmux capture-pane -t {name} -e -S -200"),
            ),
            Err(e) => (
                SessionKind::Tmux,
                format!("\x1b[31m캡처 실패: {e}\x1b[0m"),
                "error".into(),
            ),
        }
    } else if let Some(proj) = identifier.strip_prefix("claude:") {
        match tail_claude_jsonl(proj) {
            Ok(s) => (
                SessionKind::ClaudeProject,
                s,
                format!("~/.claude/projects/{proj}/*.jsonl tail 50"),
            ),
            Err(e) => (
                SessionKind::ClaudeProject,
                format!("\x1b[31m읽기 실패: {e}\x1b[0m"),
                "error".into(),
            ),
        }
    } else {
        (
            SessionKind::XgramSession,
            format!("\x1b[33m(unsupported identifier: {identifier})\x1b[0m"),
            "unsupported".into(),
        )
    };
    let lines = content.lines().count() as u32;
    SessionScreenDto {
        identifier: identifier.to_string(),
        kind,
        display: identifier.split(':').nth(1).unwrap_or(identifier).into(),
        content,
        lines,
        source_note,
        fetched_at: chrono::Utc::now().to_rfc3339(),
    }
}

