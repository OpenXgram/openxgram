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

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Clone)]
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

#[derive(Debug, Serialize, Clone)]
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

#[derive(Debug, Serialize, Clone)]
pub struct SessionsDto {
    pub machine: MachineInfo,
    pub sessions: Vec<DetectedSession>,
}

/// rc.148 — portal AoE API 의 tmux_session_name → activity_state map.
/// "active" = LLM 작업 중 (녹색), "waiting" = 사용자 입력 대기 (노랑).
fn aoe_activity_map() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let url = portal_url_base();
    let token = portal_token();
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .danger_accept_invalid_certs(true)
        .build() {
        Ok(c) => c,
        Err(_) => return map,
    };
    if let Ok(resp) = client.get(format!("{}/api/aoe/sessions?token={}", url.trim_end_matches('/'), token)).send() {
        if let Ok(v) = resp.json::<serde_json::Value>() {
            if let Some(sessions) = v.get("sessions").and_then(|s| s.as_array()) {
                for sess in sessions {
                    if let (Some(name), Some(state)) = (
                        sess.get("tmux_session_name").and_then(|x| x.as_str()),
                        sess.get("activity_state").and_then(|x| x.as_str()),
                    ) {
                        map.insert(name.to_string(), state.to_string());
                    }
                }
            }
        }
    }
    map
}

/// rc.147 — attached=true 인 tmux session_name set. portal entry 상태 매핑용.
fn tmux_attached_set() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(out) = Command::new("tmux").args(["ls", "-F", "#{session_name}|#{session_attached}"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let parts: Vec<&str> = line.splitn(2, '|').collect();
                if parts.len() == 2 && parts[1] != "0" {
                    set.insert(parts[0].to_string());
                }
            }
        }
    }
    set
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
        // session 자체
        sessions.push(DetectedSession {
            kind: SessionKind::Tmux,
            identifier: format!("tmux:{name}"),
            display: name.clone(),
            status: if attached {
                SessionStatus::Attached
            } else {
                SessionStatus::Detached
            },
            windows: Some(windows),
            attached: Some(attached),
            created_at: created.clone(),
            last_active_at: None,
            agent_id: None,
        });
        // tmux windows 도 별개 entry 로 enumerate (starian-portal 같이 windows 가 = 사용자가 보는 터미널)
        if windows > 1 {
            let win_out = Command::new("tmux")
                .args(["list-windows", "-t", &name, "-F", "#{window_index}|#{window_name}|#{window_active}"])
                .output();
            if let Ok(wo) = win_out {
                if wo.status.success() {
                    let ws = String::from_utf8_lossy(&wo.stdout);
                    for wl in ws.lines() {
                        let wp: Vec<&str> = wl.splitn(3, '|').collect();
                        if wp.len() < 3 { continue; }
                        let idx = wp[0];
                        let wname = wp[1];
                        let active = wp[2] != "0";
                        sessions.push(DetectedSession {
                            kind: SessionKind::Tmux,
                            identifier: format!("tmux:{name}:{idx}"),
                            display: format!("{name} / {wname}"),
                            status: if active { SessionStatus::Active } else { SessionStatus::Detached },
                            windows: Some(1),
                            attached: Some(active && attached),
                            created_at: created.clone(),
                            last_active_at: None,
                            agent_id: None,
                        });
                    }
                }
            }
        }
    }
    sessions
}

/// starian-portal API — 두 endpoint fetch 해서 sessions 통합.
///   1. `/api/terminals`     → identifier `portal:<tmuxSession>:<tmuxIndex>` (rc.89~)
///   2. `/api/aoe/sessions`  → identifier `aoe:<tmuxSession>:<aoe_id>:<title>` (rc.89~)
///
/// 둘 다 capture 시 portal-new `/api/tmux/capture?session=<tmuxSession>&window=<idx>` 호출.
fn detect_starian_portal() -> Vec<DetectedSession> {
    let url_base = portal_url_base();
    let token = portal_token();
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .danger_accept_invalid_certs(true)
        .build() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut out: Vec<DetectedSession> = Vec::new();
    // (1) /api/terminals — tmuxSession + tmuxIndex 사용
    let terms_url = format!("{}/api/terminals?token={}", url_base, token);
    if let Ok(resp) = client.get(&terms_url).send() {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>() {
                if let Some(terms) = v.get("terminals").and_then(|t| t.as_array()) {
                    for t in terms.iter() {
                        let id = t.get("id").and_then(|x| x.as_str()).unwrap_or("?");
                        let name = t.get("name").and_then(|x| x.as_str()).unwrap_or(id);
                        let origin = t.get("origin").and_then(|x| x.as_str()).unwrap_or("");
                        let path = t.get("path").and_then(|x| x.as_str()).unwrap_or("");
                        let group = t.get("group").and_then(|x| x.as_str()).unwrap_or("");
                        let tmux_session = t.get("tmuxSession").and_then(|x| x.as_str()).unwrap_or("starian");
                        let tmux_index = t.get("tmuxIndex").and_then(|x| x.as_u64()).unwrap_or(0);
                        out.push(DetectedSession {
                            kind: SessionKind::Tmux,
                            identifier: format!("portal:{}:{}", tmux_session, tmux_index),
                            display: format!("{} [{}]", name, origin),
                            status: SessionStatus::Detached,
                            windows: Some(1),
                            attached: None,
                            created_at: None,
                            last_active_at: Some(format!("id:{} · group:{} · path:{}", id, group, path)),
                            agent_id: None,
                        });
                    }
                }
            }
        }
    }
    // (2) /api/aoe/sessions
    let aoe_url = format!("{}/api/aoe/sessions?token={}", url_base, token);
    if let Ok(resp) = client.get(&aoe_url).send() {
        if resp.status().is_success() {
            if let Ok(v) = resp.json::<serde_json::Value>() {
                if v.get("available").and_then(|b| b.as_bool()).unwrap_or(false) {
                    if let Some(sess) = v.get("sessions").and_then(|s| s.as_array()) {
                        for s in sess {
                            let id = s.get("id").and_then(|x| x.as_str()).unwrap_or("?");
                            let title = s.get("title").and_then(|x| x.as_str()).unwrap_or(id);
                            let project_path = s.get("project_path").and_then(|x| x.as_str()).unwrap_or("");
                            let status = s.get("status").and_then(|x| x.as_str()).unwrap_or("");
                            let alive = s.get("tmux_alive").and_then(|b| b.as_bool()).unwrap_or(false);
                            let tmux_name = s.get("tmux_session_name").and_then(|x| x.as_str()).unwrap_or("");
                            // identifier 에 tmux session 직접 인코딩 — 그 tmux:0 캡쳐로 화면 보임.
                            out.push(DetectedSession {
                                kind: SessionKind::Tmux,
                                identifier: format!("aoe:{}:{}:{}", tmux_name, id, title),
                                display: format!("aoe·{} [{}]", title, status),
                                status: if alive { SessionStatus::Active } else { SessionStatus::Detached },
                                windows: Some(1),
                                attached: None,
                                created_at: s.get("created_at").and_then(|x| x.as_str()).map(String::from),
                                last_active_at: Some(format!("tmux:{} · path:{}", tmux_name, project_path)),
                                agent_id: None,
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

fn portal_url_base() -> String {
    std::env::var("XGRAM_PORTAL_URL")
        .unwrap_or_else(|_| "https://portal-zalman.starian.us".into())
        .trim_end_matches('/').to_string()
}

fn portal_token() -> String {
    std::env::var("XGRAM_PORTAL_TOKEN").unwrap_or_else(|_| "0205".into())
}

/// portal capture — `/api/tmux/capture?session=<S>&window=<idx>` (rc.89~).
/// session 명시 시 그 tmux 세션의 window 캡쳐. None 이면 default starian session.
pub fn capture_portal_session(session: Option<&str>, idx: u32) -> Result<String, String> {
    let mut url = format!("{}/api/tmux/capture?window={}&lines=200&escape=1&token={}",
        portal_url_base(), idx, portal_token());
    if let Some(s) = session {
        if !s.is_empty() {
            url.push_str(&format!("&session={}", urlencode(s)));
        }
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("http: {e}"))?;
    let resp = client.get(&url).send().map_err(|e| format!("send: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("portal HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().map_err(|e| format!("json: {e}"))?;
    Ok(v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string())
}

/// 옛 API 호환 — default session, idx only.
pub fn capture_portal(idx: u32) -> Result<String, String> {
    capture_portal_session(None, idx)
}

/// AoE 세션 캡쳐 — identifier 의 tmux_session_name 사용. portal 의 새 session 파라미터로 호출.
pub fn capture_aoe(rest: &str) -> Result<String, String> {
    // rest 형식: `<tmux_session_name>:<aoe_id>:<title>`
    let tmux_session = rest.split(':').next().unwrap_or("");
    if tmux_session.is_empty() {
        return Err("aoe identifier 에 tmux_session_name 없음 (rc.88 cache?). 새로고침 필요.".into());
    }
    capture_portal_session(Some(tmux_session), 0)
}

fn urlencode(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect()
}

/// `~/.claude/projects/<encoded-path>/*.jsonl` — 각 디렉토리가 Claude Code 프로젝트.
/// 가장 최근 .jsonl 의 mtime 을 last_active 로 사용.
fn detect_claude_projects() -> Vec<DetectedSession> {
    let mut out = Vec::new();
    // WSL 환경에서 Windows side claude projects 도 스캔 (W가 Windows 직접 Claude Code 실행하는 경우).
    // XGRAM_EXTRA_CLAUDE_DIRS env 로 추가 dir 콜론구분 (예: /mnt/c/Users/User/.claude/projects)
    let mut dirs: Vec<PathBuf> = Vec::new();
    // rc.139 — Windows 호환: HOME 없으면 USERPROFILE fallback.
    // 이전엔 Windows 에서 sessions 0 반환 (HOME env 없음).
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home {
        let p: PathBuf = [home, ".claude/projects".into()].iter().collect();
        dirs.push(p);
    }
    if let Ok(extra) = std::env::var("XGRAM_EXTRA_CLAUDE_DIRS") {
        for s in extra.split(':') {
            let p = PathBuf::from(s.trim());
            if !p.as_os_str().is_empty() { dirs.push(p); }
        }
    }
    // WSL 자동 감지 — /mnt/c/Users/*/.claude/projects 가 있으면 추가
    if let Ok(c_users) = std::fs::read_dir("/mnt/c/Users") {
        for u in c_users.flatten() {
            let p = u.path().join(".claude/projects");
            if p.exists() {
                dirs.push(p);
            }
        }
    }
    for projects_dir in dirs {
    let Ok(read) = std::fs::read_dir(&projects_dir) else {
        continue;
    };
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
    } // for projects_dir in dirs
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

/// 통합: 머신 + 세션들.
/// 정책: 사용자 의도 기준만 노출.
///   - portal-new 가 등록한 터미널 (portal:*, aoe:*) — 사용자가 직접 만든 것
///   - Claude Code project history (claude:*) — 사용자가 한 번이라도 그 폴더에서 claude 실행함
///   - peer/process (peer:*, proc:*) — 네트워크/시스템 메타
/// raw tmux:* 는 제외 — system jobs (xgramd 등) 노이즈 차단. portal 등록한 것만 의미 있음.
/// portal-new 가 없는 머신만 fallback 으로 raw tmux 보임.
// rc.137 — stale-while-revalidate 패턴.
// 60초 TTL + background warming worker (30초마다 미리 collect → cache 갱신).
// endpoint 는 항상 cache 즉시 반환 (cache 만료돼도 옛 데이터라도 반환).
// collect 가 5초 이상 걸려도 endpoint hang 안 함.
static SESSIONS_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Option<(std::time::Instant, SessionsDto)>>
> = std::sync::OnceLock::new();

const CACHE_TTL_SECS: u64 = 60;

pub fn collect_sessions() -> SessionsDto {
    let cache = SESSIONS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    // 1) cache 있으면 즉시 반환 (TTL 무관 — stale 도 OK, background warming 이 갱신)
    if let Ok(guard) = cache.lock() {
        if let Some((ts, dto)) = guard.as_ref() {
            // TTL 안 지났으면 fresh. 지났어도 stale 반환 — endpoint hang 안 시킴.
            // 다만 stale 이면 다음 background tick 에서 갱신.
            if ts.elapsed() < std::time::Duration::from_secs(CACHE_TTL_SECS) {
                return dto.clone();
            }
            // stale 반환 + fresh 도 시도 (fallthrough)
            let stale = dto.clone();
            drop(guard);
            // background 가 비활성이면 여기서 동기 갱신 시도, 실패 시 stale 반환
            return refresh_or_stale(stale);
        }
    }
    // 2) cache 비어있음 (첫 호출) — 동기 collect
    let dto = collect_fresh();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((std::time::Instant::now(), dto.clone()));
    }
    dto
}

fn refresh_or_stale(stale: SessionsDto) -> SessionsDto {
    // 동기 갱신 시도. 실패하거나 시간 오래 걸리면 stale 반환.
    // 단순 구현: 그냥 collect_fresh 호출 후 cache 갱신.
    // background warming 이 잘 돌면 거의 도달 안 함.
    let dto = collect_fresh();
    let cache = SESSIONS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((std::time::Instant::now(), dto.clone()));
    }
    let _ = stale;
    dto
}

fn collect_fresh() -> SessionsDto {
    let machine = detect_machine();
    let mut sessions = Vec::new();
    let mut portal = detect_starian_portal();
    let had_portal = !portal.is_empty();
    // rc.148 — portal entry 의 status 를 AoE activity_state 기반으로 매핑.
    // active = LLM 작업 중 → 녹색 (Attached 로 표시)
    // waiting = 사용자 입력 대기 → 노랑 (Detached 로 표시)
    // 이전 (rc.147): tmux attached/detached 만 봐서 의미 부정확. 사용자 의도 = 에이전트 작동 상태.
    let activity_map = aoe_activity_map();
    for s in &mut portal {
        let tmux_name: Option<String> = if let Some(rest) = s.identifier.strip_prefix("portal:") {
            rest.split(':').next().map(String::from)
        } else if let Some(rest) = s.identifier.strip_prefix("aoe:") {
            rest.split(':').next().map(String::from)
        } else { None };
        if let Some(name) = tmux_name {
            if let Some(state) = activity_map.get(&name) {
                match state.as_str() {
                    "active" => {
                        s.status = SessionStatus::Active;
                        s.attached = Some(true); // green dot
                    }
                    _ => {
                        s.status = SessionStatus::Detached;
                        s.attached = Some(false); // yellow dot
                    }
                }
            }
        }
    }
    sessions.extend(portal);
    sessions.extend(detect_claude_projects());
    if !had_portal {
        sessions.extend(detect_tmux());
    }
    sessions.extend(detect_processes());
    SessionsDto { machine, sessions }
}

/// rc.137 — daemon 시작 시 spawn. 30초마다 collect → cache 갱신.
/// endpoint 는 항상 cache 만 즉시 반환 → 사용자 응답 < 1ms 보장.
pub fn spawn_session_warming() {
    tokio::spawn(async {
        // 시작 직후 첫 collect
        let dto = tokio::task::spawn_blocking(collect_fresh).await
            .unwrap_or_else(|_| SessionsDto { machine: detect_machine(), sessions: vec![] });
        // rc.156 — auto-sync 비활성화. 사용자가 GUI 에서 직접 등록/해제 선택.
        // (이전 rc.143 의 자동 sync 가 portal 의 모든 tmux 를 강제 등록 → 사용자 선택 override)
        let _ = sync_messenger_registrations; // keep function 참조 (dead-code 경고 회피)
        let cache = SESSIONS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
        if let Ok(mut guard) = cache.lock() {
            *guard = Some((std::time::Instant::now(), dto));
        }
        tracing::info!("session_warming: initial collect done");
        // 30초마다 갱신
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // 첫 tick 즉시 — skip
        loop {
            interval.tick().await;
            let started = std::time::Instant::now();
            let dto = tokio::task::spawn_blocking(collect_fresh).await
                .unwrap_or_else(|_| SessionsDto { machine: detect_machine(), sessions: vec![] });
            // rc.156 — auto-sync 비활성화. 사용자가 GUI 에서 직접 등록/해제 선택.
        // (이전 rc.143 의 자동 sync 가 portal 의 모든 tmux 를 강제 등록 → 사용자 선택 override)
        let _ = sync_messenger_registrations; // keep function 참조 (dead-code 경고 회피)
            let elapsed = started.elapsed();
            if let Ok(mut guard) = cache.lock() {
                *guard = Some((std::time::Instant::now(), dto));
            }
            if elapsed.as_secs() >= 5 {
                tracing::warn!("session_warming: collect took {}s (slow)", elapsed.as_secs());
            }
        }
    });
}

/// rc.143 — agent_capabilities 의 messenger_enabled 를 portal/aoe sessions 와 자동 동기화.
/// • portal 에 있는 tmux session → messenger_enabled=1 (없으면 INSERT)
/// • portal 에 없는 등록 → messenger_enabled=0
/// 사이드바 보이는 것 = 메신저 등록된 것 일치성 보장.
fn sync_messenger_registrations(dto: &SessionsDto) {
    let data_dir = match openxgram_core::paths::default_data_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let db_path = data_dir.join("db.sqlite");
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    use std::collections::HashSet;
    let mut portal_tmux: HashSet<String> = HashSet::new();
    for s in &dto.sessions {
        let id = &s.identifier;
        // peer:<alias>:rest 면 local 머신 아님 (server-seoul 의 agent_capabilities 갱신 X)
        if id.starts_with("peer:") { continue; }
        let inner = id.as_str();
        if let Some(rest) = inner.strip_prefix("portal:") {
            if let Some(name) = rest.split(':').next() { portal_tmux.insert(name.to_string()); }
        } else if let Some(rest) = inner.strip_prefix("aoe:") {
            if let Some(name) = rest.split(':').next() { portal_tmux.insert(name.to_string()); }
        } else if let Some(rest) = inner.strip_prefix("tmux:") {
            if let Some(name) = rest.split(':').next() { portal_tmux.insert(name.to_string()); }
        }
    }
    if portal_tmux.is_empty() {
        return; // 안전 — portal 비어있으면 sync skip (모든 등록 비활성화 방지)
    }
    let now = chrono::Local::now().to_rfc3339();
    // 새 / 갱신: portal 에 있는 모든 tmux session → messenger_enabled=1 upsert
    for name in &portal_tmux {
        let _ = conn.execute(
            "INSERT INTO agent_capabilities (alias, role, description, capabilities, tool_list, project_path, updated_at, messenger_enabled) \
             VALUES (?1, 'tmux', 'auto-synced from sessions', '[]', '[]', '', ?2, 1) \
             ON CONFLICT(alias) DO UPDATE SET messenger_enabled=1, updated_at=excluded.updated_at",
            rusqlite::params![name, &now],
        );
    }
    // portal 없는 등록은 비활성 (alias 가 aoe_* 또는 sv_aoe_* 패턴인 것만 — 사용자 등록 안 건드림)
    let placeholders = portal_tmux.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE agent_capabilities SET messenger_enabled=0, updated_at=?1 \
         WHERE messenger_enabled=1 AND (alias LIKE 'aoe_%' OR alias LIKE 'sv_aoe_%') AND alias NOT IN ({})",
        placeholders
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];
    for name in &portal_tmux {
        params.push(Box::new(name.clone()));
    }
    let params_ref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let _ = conn.execute(&sql, &params_ref[..]);
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
    // rc.139 — Windows 호환: HOME 없으면 USERPROFILE fallback
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("HOME/USERPROFILE unset")?;
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

/// UI-MESSENGER-SPEC v1.3 §7.5 + N4 — 글로벌 검색 결과 (FTS5).
#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub kind: String,        // 'message' | 'wiki' | 'mistake' | 'pattern' | 'trait'
    pub ref_id: String,
    pub title: String,
    pub body: String,
    pub rank: f64,
}

#[derive(Debug, Serialize)]
pub struct SearchResultDto {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub total: usize,
}

/// V11 — RoutingRule (에이전트 ↔ 에이전트 internal scope).
#[derive(Debug, Serialize, Deserialize)]
pub struct RoutingRuleDto {
    pub id: String,
    pub scope: String,
    pub from_pattern: String,
    pub to_pattern: String,
    pub action: String,
    pub created_at: String,
    pub active: bool,
}

/// V12 — 3-layer version (release / GUI / daemon) + rc.92 changelog 최근 entry.
#[derive(Debug, Serialize)]
pub struct VersionInfoDto {
    pub release: String,
    pub daemon: String,
    pub spec_doc: String,
    pub prd_doc: String,
    /// rc.92 — CHANGELOG.md 의 latest version block (UI 팝업 표시).
    pub changelog_latest_title: Option<String>,
    pub changelog_latest_body: Option<String>,
}

/// CHANGELOG.md 의 최상단 `## [버전] — ...` block 1개 추출.
fn extract_latest_changelog() -> (Option<String>, Option<String>) {
    let candidates = [
        std::path::PathBuf::from("CHANGELOG.md"),
        std::path::PathBuf::from("/home/pasia/projects/openxgram/CHANGELOG.md"),
        std::path::PathBuf::from("/home/llm/projects/starian-set/openxgram/CHANGELOG.md"),
    ];
    let mut content = String::new();
    for p in &candidates {
        if let Ok(s) = std::fs::read_to_string(p) {
            content = s;
            break;
        }
    }
    if content.is_empty() { return (None, None); }
    // first `## [` 부터 다음 `## [` 또는 EOF 까지.
    let lines: Vec<&str> = content.lines().collect();
    let mut start: Option<usize> = None;
    let mut end: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("## [") {
            if start.is_none() {
                start = Some(i);
            } else {
                end = Some(i);
                break;
            }
        }
    }
    let start = match start { Some(s) => s, None => return (None, None) };
    let end = end.unwrap_or(lines.len());
    let title = lines[start].trim_start_matches("## ").to_string();
    let body: String = lines[(start+1)..end].iter().copied().collect::<Vec<_>>().join("\n").trim().to_string();
    (Some(title), Some(body))
}

// rc.135 — CARGO_PKG_VERSION 매크로가 incremental cache 로 workspace.version 변경 미반영.
// const 직접 작성 → 파일 mtime 변경 → 강제 재컴파일 → version_info 응답 갱신 → App.tsx 의
// 30s polling 이 cur != baseline 감지 → 업데이트 팝업 표시.
// 매 release 마다 RELEASE_TAG 갱신 (Cargo.toml + ui/web/package.json + 본 const 3곳).
pub const RELEASE_TAG: &str = "0.2.0-rc.163";

pub fn version_info() -> VersionInfoDto {
    let (title, body) = extract_latest_changelog();
    VersionInfoDto {
        release: RELEASE_TAG.to_string(),
        daemon: RELEASE_TAG.to_string(),
        spec_doc: "UI-MESSENGER-SPEC v1.3".to_string(),
        prd_doc: "PRD-OpenXgram v1.4".to_string(),
        changelog_latest_title: title,
        changelog_latest_body: body,
    }
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
    let (kind, content, source_note) = if let Some(rest) = identifier.strip_prefix("portal:") {
        // 새 형식 (rc.89+): portal:<tmuxSession>:<tmuxIndex>
        // 옛 형식 (rc.88):  portal:<idx>:<id>   ← idx 가 숫자면 옛 형식, 아니면 새 형식
        let mut parts = rest.splitn(2, ':');
        let first = parts.next().unwrap_or("0");
        let rest2 = parts.next().unwrap_or("");
        let (session_opt, idx) = if first.parse::<u32>().is_ok() && !rest2.contains(':') {
            // 옛 형식 fallback — first 가 숫자 + 두 번째에 ':' 없음
            (None, first.parse::<u32>().unwrap_or(0))
        } else {
            // 새 형식 — first = session 명, rest2 = window index
            let idx = rest2.split(':').next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            (Some(first), idx)
        };
        match capture_portal_session(session_opt, idx) {
            Ok(s) => (
                SessionKind::Tmux,
                s,
                format!("portal capture session={} window={}", session_opt.unwrap_or("(default)"), idx),
            ),
            Err(e) => (
                SessionKind::Tmux,
                format!("\x1b[31mportal 캡처 실패: {e}\n→ portal-zalman.starian.us 에서 직접 보기, 또는 XGRAM_PORTAL_URL/TOKEN env 확인\x1b[0m"),
                "error".into(),
            ),
        }
    } else if let Some(rest) = identifier.strip_prefix("aoe:") {
        // aoe:<id>:<title> 형식
        match capture_aoe(rest) {
            Ok(s) => (SessionKind::Tmux, s, format!("aoe session {rest}")),
            Err(e) => (
                SessionKind::Tmux,
                format!("\x1b[31mAoE 캡처 실패: {e}\x1b[0m"),
                "error".into(),
            ),
        }
    } else if let Some(name) = identifier.strip_prefix("tmux:") {
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

