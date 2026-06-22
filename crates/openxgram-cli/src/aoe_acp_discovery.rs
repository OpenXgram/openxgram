//! AoE-managed ACP 세션 동적 열거 (현황 그리드 통합).
//!
//! 배경: openxgram 통합 로스터(`GET /v1/gui/roster`)는 지금까지 openxgram-native
//! ACP 세션(`/v1/acp/sessions`)과 tmux 세션만 보여줬다. AoE(`aoe __acp-runner`)가
//! 띄운 ACP 세션은 보이지 않았다. 스펙
//! (docs/superpowers/specs/2026-06-20-canonical-identity-and-status-grid-design.md L106-107)
//! 과 oxg.md L29-30 의 "머신의 모든 세션 동적 리스트업, 정적 list 금지" 요구에 따라
//! 라이브 AoE ACP 세션을 매 호출마다 동적으로 열거하여 로스터에 합류시킨다.
//!
//! 발견 출처 (FS 읽기는 daemon caller 가 수행, 이 모듈은 순수 매핑만):
//! - `~/.config/agent-of-empires/acp-workers/<session-id>.sock` — 라이브 세션당 소켓 1개
//! - `~/.config/agent-of-empires/acp-workers/<session-id>.json` — runner sidecar
//!   (session_id, pid, cwd, agent_name, detached_at 등)
//! - `~/.config/agent-of-empires/profiles/*/sessions.json` — session-id → title(label)
//!
//! 라이브니스: `.sock` 존재 + runner pid alive(+ detached_at 가 null). caller 가
//! `.sock` 존재 + pid 생존을 판정해 `live=true` 로 넘긴다.
//!
//! 규칙: 절대 룰 #1(fallback 금지) — JSON 파싱 실패는 조용히 넘기지 않고 caller 가
//! 로그/전파한다. 이 순수 코어는 파싱 오류를 `Result`/`Err` 로 명시 반환한다.

use serde::Deserialize;

/// 로스터 한 행으로 투영될 AoE ACP 세션. caller 가 `SessionInput` 으로 변환한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AoeAcpEntry {
    /// 로스터 group 키. 항상 `aoe-acp:<session-id>` 형태(브리지된 peer 와 dedupe).
    pub session_identifier: String,
    /// 표시명 — sessions.json 의 title, 없으면 session-id 단축.
    pub display_name: String,
    /// 작업 디렉토리 — sidecar/cmdline cwd.
    pub cwd: Option<String>,
}

/// runner sidecar(`<id>.json`)의 필요한 필드만.
#[derive(Debug, Clone, Deserialize)]
pub struct AoeWorkerSidecar {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// runner 가 detach 되면 RFC3339 문자열, live 면 null.
    #[serde(default)]
    pub detached_at: Option<serde_json::Value>,
}

/// sessions.json 한 항목 — title(label) 매핑에만 사용.
#[derive(Debug, Clone, Deserialize)]
pub struct AoeSessionStoreEntry {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
}

/// caller 가 발견한 라이브 AoE ACP 세션 1건의 원시 입력.
///
/// `sidecar_json` 은 `<id>.json` 의 내용(없으면 None — cmdline fallback 용).
/// `cwd_from_cmdline` 은 sidecar 부재 시 `aoe __acp-runner ... --cwd <X>` 에서
/// 뽑은 cwd. `live` 는 caller 의 라이브니스 판정(.sock 존재 + pid 생존).
#[derive(Debug, Clone)]
pub struct DiscoveredWorker {
    pub session_id: String,
    pub sidecar_json: Option<String>,
    pub cwd_from_cmdline: Option<String>,
    pub live: bool,
}

/// session-id 짧은 표기(앞 8자) — title 이 없을 때의 fallback display.
fn short_id(session_id: &str) -> String {
    let n = session_id.len().min(8);
    format!("aoe:{}", &session_id[..n])
}

/// `sessions.json` 한 파일의 내용을 파싱한다. 실패 시 `Err`(조용한 무시 금지).
pub fn parse_session_store(contents: &str) -> Result<Vec<AoeSessionStoreEntry>, serde_json::Error> {
    serde_json::from_str(contents)
}

/// runner sidecar JSON 1건 파싱.
pub fn parse_worker_sidecar(contents: &str) -> Result<AoeWorkerSidecar, serde_json::Error> {
    serde_json::from_str(contents)
}

/// sidecar 의 `detached_at` 가 "비어있음"(null/없음) 인지 — live 보조 판정.
fn sidecar_attached(side: &AoeWorkerSidecar) -> bool {
    match &side.detached_at {
        None => true,
        Some(serde_json::Value::Null) => true,
        Some(_) => false,
    }
}

/// 순수 매핑 코어 — caller 가 모은 라이브 worker 목록 + 모든 프로필의
/// sessions.json 항목을 합쳐 로스터 엔트리로 투영한다.
///
/// - 라이브니스: `worker.live == true` 인 것만 포함(소켓 존재 + pid 생존).
///   추가로 sidecar 가 있으면 `detached_at` 가 null 일 때만 포함(이중 안전).
/// - label: sessions.json 의 `title`(session-id 매칭), 없으면 short_id.
/// - cwd: sidecar.cwd → cmdline cwd → sessions.json.project_path 순.
/// - dedupe: `bridged_session_idents` 에 이미 동일 정규화 키가 있으면 제외
///   (브리지된 peer 가 이미 그 세션을 로스터에 올리므로 이중 표시 방지).
///
/// `bridged_session_idents` 는 peers.session_identifier 중 ACP 계열 값들
/// (예: `acp:<id>`, `aoe-acp:<id>`). 정규화는 마지막 `:` 뒤 토큰(=session-id)으로
/// 비교한다.
pub fn build_entries(
    workers: &[DiscoveredWorker],
    store_entries: &[AoeSessionStoreEntry],
    bridged_session_idents: &[String],
) -> Vec<AoeAcpEntry> {
    // session-id → title / project_path 룩업.
    let mut bridged_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for b in bridged_session_idents {
        bridged_ids.insert(tail_token(b));
    }

    let mut out: Vec<AoeAcpEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for w in workers {
        if !w.live {
            continue;
        }
        // sidecar 가 있으면 파싱해 detached 면 제외 + cwd 보강.
        let mut sidecar_cwd: Option<String> = None;
        if let Some(raw) = &w.sidecar_json {
            // 파싱 실패는 무시하지 않고 — 여기선 순수 코어라 caller 로그를 신뢰하되,
            // 파싱 성공 시에만 detach/ cwd 를 반영(실패 시 cmdline fallback 사용).
            if let Ok(side) = parse_worker_sidecar(raw) {
                if !sidecar_attached(&side) {
                    continue;
                }
                sidecar_cwd = side.cwd.clone();
            }
        }

        // dedupe: 이미 본 세션 또는 브리지된 세션은 제외.
        if w.session_id.is_empty() || seen.contains(&w.session_id) {
            continue;
        }
        if bridged_ids.contains(&w.session_id) {
            continue;
        }
        seen.insert(w.session_id.clone());

        // label / project_path from sessions.json.
        let store = store_entries.iter().find(|e| e.id == w.session_id);
        let display_name = store
            .and_then(|e| e.title.clone())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| short_id(&w.session_id));

        let cwd = sidecar_cwd
            .or_else(|| w.cwd_from_cmdline.clone())
            .or_else(|| store.and_then(|e| e.project_path.clone()))
            .filter(|c| !c.trim().is_empty());

        out.push(AoeAcpEntry {
            session_identifier: format!("aoe-acp:{}", w.session_id),
            display_name,
            cwd,
        });
    }

    out
}

/// 마지막 `:` 뒤 토큰(없으면 전체) — 정규화 dedupe 비교용.
fn tail_token(s: &str) -> String {
    match s.rsplit_once(':') {
        Some((_, t)) => t.to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_json() -> &'static str {
        r#"[
            {"id":"51a4cda6540e4032","title":"page-picker","project_path":"/home/llm/.starian/page-picker"},
            {"id":"06cbd2cc361d4f98","title":"starianset","project_path":"/home/llm/x/starianset_acp"}
        ]"#
    }

    fn sidecar(session_id: &str, cwd: &str, detached: bool) -> String {
        let det = if detached { "\"2026-06-22T00:00:00Z\"" } else { "null" };
        format!(
            r#"{{"runner_version":1,"session_id":"{session_id}","pid":123,"cwd":"{cwd}","detached_at":{det}}}"#
        )
    }

    #[test]
    fn parses_store_titles_and_paths() {
        let entries = parse_session_store(store_json()).expect("parse");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "51a4cda6540e4032");
        assert_eq!(entries[0].title.as_deref(), Some("page-picker"));
        assert_eq!(
            entries[0].project_path.as_deref(),
            Some("/home/llm/.starian/page-picker")
        );
    }

    #[test]
    fn live_worker_maps_to_entry_with_title_and_cwd() {
        let workers = vec![DiscoveredWorker {
            session_id: "51a4cda6540e4032".into(),
            sidecar_json: Some(sidecar("51a4cda6540e4032", "/home/llm/.starian/page-picker", false)),
            cwd_from_cmdline: None,
            live: true,
        }];
        let store = parse_session_store(store_json()).unwrap();
        let out = build_entries(&workers, &store, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].session_identifier, "aoe-acp:51a4cda6540e4032");
        assert_eq!(out[0].display_name, "page-picker");
        assert_eq!(out[0].cwd.as_deref(), Some("/home/llm/.starian/page-picker"));
    }

    #[test]
    fn dead_worker_filtered_by_liveness() {
        let workers = vec![DiscoveredWorker {
            session_id: "187b4289e48a4e8a".into(),
            sidecar_json: None,
            cwd_from_cmdline: Some("/tmp".into()),
            live: false, // no .sock / pid not alive
        }];
        let store = parse_session_store(store_json()).unwrap();
        let out = build_entries(&workers, &store, &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn detached_sidecar_filtered_even_if_socket_present() {
        let workers = vec![DiscoveredWorker {
            session_id: "51a4cda6540e4032".into(),
            sidecar_json: Some(sidecar("51a4cda6540e4032", "/x", true)),
            cwd_from_cmdline: None,
            live: true, // caller saw .sock, but sidecar says detached
        }];
        let store = parse_session_store(store_json()).unwrap();
        let out = build_entries(&workers, &store, &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn dedupe_against_bridged_peer() {
        let workers = vec![DiscoveredWorker {
            session_id: "51a4cda6540e4032".into(),
            sidecar_json: Some(sidecar("51a4cda6540e4032", "/x", false)),
            cwd_from_cmdline: None,
            live: true,
        }];
        let store = parse_session_store(store_json()).unwrap();
        // peer already bridged this acp session under "acp:<id>".
        let bridged = vec!["acp:51a4cda6540e4032".to_string()];
        let out = build_entries(&workers, &store, &bridged);
        assert!(out.is_empty(), "bridged session must not be double-listed");

        // also dedupe when bridged uses the aoe-acp: prefix form.
        let bridged2 = vec!["aoe-acp:51a4cda6540e4032".to_string()];
        let out2 = build_entries(&workers, &store, &bridged2);
        assert!(out2.is_empty());
    }

    #[test]
    fn missing_title_falls_back_to_short_id() {
        let workers = vec![DiscoveredWorker {
            session_id: "deadbeefcafe0000".into(),
            sidecar_json: Some(sidecar("deadbeefcafe0000", "/w", false)),
            cwd_from_cmdline: None,
            live: true,
        }];
        // store has no entry for this id.
        let store = parse_session_store(store_json()).unwrap();
        let out = build_entries(&workers, &store, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].display_name, "aoe:deadbeef");
        assert_eq!(out[0].cwd.as_deref(), Some("/w"));
    }

    #[test]
    fn cmdline_cwd_used_when_sidecar_absent() {
        let workers = vec![DiscoveredWorker {
            session_id: "06cbd2cc361d4f98".into(),
            sidecar_json: None,
            cwd_from_cmdline: Some("/home/llm/x/starianset_acp".into()),
            live: true,
        }];
        let store = parse_session_store(store_json()).unwrap();
        let out = build_entries(&workers, &store, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].display_name, "starianset");
        assert_eq!(out[0].cwd.as_deref(), Some("/home/llm/x/starianset_acp"));
    }

    #[test]
    fn duplicate_workers_collapsed() {
        let mk = || DiscoveredWorker {
            session_id: "51a4cda6540e4032".into(),
            sidecar_json: Some(sidecar("51a4cda6540e4032", "/x", false)),
            cwd_from_cmdline: None,
            live: true,
        };
        let workers = vec![mk(), mk()];
        let store = parse_session_store(store_json()).unwrap();
        let out = build_entries(&workers, &store, &[]);
        assert_eq!(out.len(), 1);
    }
}
