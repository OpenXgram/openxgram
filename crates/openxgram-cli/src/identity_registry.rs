//! 정본 신원 — 중앙 등록·중복(이름 유일성) 로직.
//!
//! IDENTITY-MODEL-FINAL-SPEC §A(두 입구 공용 로직)·§C(이름=신원·단일 네임스페이스)·
//! §J 갭#1(split-brain arbiter)·갭#2(죽은세션 이어받기 hijack 방지).
//!
//! 두 입구(MCP `register_subagent`, GUI 그리드 이름 매칭)는 모두 이
//! [`check_name_available`] 를 호출한다. 파괴적 DELETE(기존 동명 행 제거) 대신
//! "충돌=명시 에러, 죽은 동명만 조용히 이어받기" 로 전환한다.

use rusqlite::Connection;

/// 내부 `route_type` 도메인 — IDENTITY-MODEL-FINAL-SPEC §J 갭#5.
///
/// 이것은 **운영/디버깅용 내부 경로 분류**(peers.route_type 컬럼)이며, GUI 가
/// 사용자에게 보여주는 `kind`(acp / tmux) 와는 **다른 차원**이다. 혼동 금지:
///   - UI `kind`     = 사용자에게 보이는 신원 종류(acp / tmux 두 가지).
///   - `route_type`  = 내부 전달 경로(아래 4종). 한 UI kind 가 여러 route_type 에 대응 가능
///     (예: UI=acp 는 route_type=acp-existing 또는 acp-new).
///
/// 지금은 도메인 값만 상수로 고정한다(채우는 로직은 별도). 허용값 외 문자열 사용 금지.
pub mod route_type {
    /// 기존에 살아있는 ACP 세션으로 전달.
    pub const ACP_EXISTING: &str = "acp-existing";
    /// 새로 spawn 한 ACP 세션으로 전달.
    pub const ACP_NEW: &str = "acp-new";
    /// tmux 세션(사람용 관찰 경로)으로 전달.
    pub const TMUX: &str = "tmux";
    /// portal webhook 등 직접 경로로 전달.
    pub const DIRECT_PORTAL: &str = "direct-portal";

    /// 허용되는 모든 route_type 값(검증·문서화용 단일 출처).
    pub const ALL: [&str; 4] = [ACP_EXISTING, ACP_NEW, TMUX, DIRECT_PORTAL];
}

/// 죽음 판정 TTL — last_seen 이 이 시간보다 오래 전이면 "더는 살아있지 않을 수 있음"
/// 후보로 본다(단, live tmux 에 세션이 있으면 무조건 살아있음으로 친다 — §J 갭#2).
///
/// 90초: heartbeat·peer last_seen 갱신 주기보다 넉넉히 길게 잡아, 상태 지연으로
/// 산 세션을 죽었다 오판(→탈취)하는 것을 막는다.
pub const DEAD_TTL_SECS: i64 = 90;

/// 같은 이름을 가진 기존 신원 행 한 개.
#[derive(Debug, Clone)]
pub struct ExistingIdentity {
    pub alias: String,
    pub display_name: Option<String>,
    pub public_key_hex: Option<String>,
    pub session_identifier: Option<String>,
    pub session_status: Option<String>,
    pub last_seen: Option<String>,
    pub origin_machine: Option<String>,
    pub identity_version: Option<i64>,
    /// 신원 갱신/이어받기 시각(unix ms). NULL=0 으로 취급(아직 갱신된 적 없음). arbiter tie-break 보조.
    pub identity_updated_at: Option<i64>,
}

/// 등록 가능 판정 결과(성공 분기).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverDecision {
    /// 같은 이름 보유 행 없음 — 신규 등록.
    New,
    /// 같은 이름이 나 자신(같은 pubkey 또는 같은 session_identifier) — 갱신.
    SelfUpdate,
    /// 같은 이름이 다른 신원이지만 그 세션이 죽음(live 아님 AND TTL 초과) — 이어받기.
    /// arbiter(origin_machine·identity_version) 비교를 통과한 경우에만 반환.
    TakeoverDead { from_alias: String },
}

/// 충돌(에러) 분기 — 살아있는 다른 신원이 이름을 점유 중이거나 arbiter 동률.
#[derive(Debug, Clone)]
pub struct NameConflictError {
    pub conflict_name: String,
    pub conflict_alias: String,
    pub conflict_session: Option<String>,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictReason {
    /// 충돌 세션이 살아있음(live tmux 존재 또는 last_seen TTL 이내).
    SessionAlive,
    /// 죽은세션이나 arbiter 동률(origin_machine·identity_version 으로 승자 결정 불가) — 안전하게 거부.
    ArbiterTie,
}

impl NameConflictError {
    /// 사용자/에이전트에게 보여줄 안내 문구(변경 또는 충돌 세션 삭제).
    pub fn user_message(&self) -> String {
        match self.reason {
            ConflictReason::SessionAlive => format!(
                "이름 '{}' 은(는) 이미 살아있는 세션 '{}'(alias={}) 이 사용 중입니다. \
                 다른 이름을 쓰거나, 충돌 세션을 종료/삭제한 뒤 다시 등록하세요.",
                self.conflict_name,
                self.conflict_session.as_deref().unwrap_or("(미상)"),
                self.conflict_alias,
            ),
            ConflictReason::ArbiterTie => format!(
                "이름 '{}' 충돌 — 기존 신원(alias={}) 과 등록 우선순위가 동률이라 \
                 안전상 자동 이어받기를 거부합니다. 다른 이름을 쓰거나 기존 신원을 먼저 삭제하세요.",
                self.conflict_name, self.conflict_alias,
            ),
        }
    }
}

/// 살아있는 tmux 세션 목록(`session_name`). 조회 실패 시 빈 Vec.
///
/// `resolve_tmux_session`(daemon_gui) 과 동일 패턴 — Windows 는 `wsl tmux`.
pub fn live_tmux_sessions() -> Vec<String> {
    let (cmd, base_arg) = if cfg!(windows) {
        ("wsl", Some("tmux"))
    } else {
        ("tmux", None)
    };
    let mut c = std::process::Command::new(cmd);
    if let Some(a) = base_arg {
        c.arg(a);
    }
    match c.args(["list-sessions", "-F", "#{session_name}"]).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// session_identifier 가 live tmux 에 존재하는지(살아있는지) 판정.
///
/// session_identifier 는 `tmux:<name>` 형식(없으면 raw name). live 목록과 정확 일치.
fn session_is_live(session_identifier: Option<&str>, live: &[String]) -> bool {
    let Some(sid) = session_identifier else {
        return false;
    };
    let name = sid.strip_prefix("tmux:").unwrap_or(sid).trim();
    if name.is_empty() {
        return false;
    }
    live.iter().any(|s| s == name)
}

/// last_seen(rfc3339) 이 TTL 이내면 true(아직 살아있음으로 본다).
fn last_seen_within_ttl(last_seen: Option<&str>, ttl_secs: i64, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(ts) = last_seen else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(ts.trim()) {
        Ok(t) => (now - t.with_timezone(&chrono::Utc)).num_seconds() <= ttl_secs,
        Err(_) => false, // 파싱 불가 = 신뢰 못 함 = 살아있다고 단정하지 않음.
    }
}

/// 같은 이름을 가진 다른 신원이 "살아있는지" 판정 — §J 갭#2.
///
/// live tmux 에 세션 존재 **OR** last_seen TTL 이내 → 살아있음(이어받기 금지).
fn other_identity_alive(
    e: &ExistingIdentity,
    live: &[String],
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    session_is_live(e.session_identifier.as_deref(), live)
        || last_seen_within_ttl(e.last_seen.as_deref(), DEAD_TTL_SECS, now)
}

/// §J 갭#1 arbiter — 죽은 동명 신원을 이어받아도 되는지 결정적 비교.
///
/// 더 "최신"인 쪽이 이름을 가진다. tie-break 순서:
///   1) identity_version desc — 큰 쪽이 이김(내가 > 기존 → 이어받기, 작으면 거부).
///   2) version 동률이면 identity_updated_at desc — 더 최근에 갱신된 쪽이 이김
///      (내가 > 기존 → 이어받기, 작으면 거부).
///   3) version·updated_at 모두 동률이면 origin_machine 동일 — 같은 신원 계보로 간주, 이어받기 허용.
///   4) 그래도 동률·불명(다른 머신 또는 정보 부족) → **안전하게 거부**(ArbiterTie).
///
/// NULL 규칙(절대): `identity_version`·`identity_updated_at` 이 NULL 이면 **양쪽 모두 0 으로 취급**해
/// 비교한다(아직 갱신된 적 없는 신원 = 최소값). `my_*` 는 이번 등록자의 값.
fn arbiter_can_take_over(
    existing: &ExistingIdentity,
    my_identity_version: Option<i64>,
    my_identity_updated_at: Option<i64>,
    my_origin_machine: Option<&str>,
) -> bool {
    // NULL 규칙: version/updated_at 의 None 은 0 으로 취급(기존·신규 양쪽 동일).
    let mine_v = my_identity_version.unwrap_or(0);
    let theirs_v = existing.identity_version.unwrap_or(0);
    if mine_v > theirs_v {
        return true;
    }
    if mine_v < theirs_v {
        return false;
    }
    // version 동률 — identity_updated_at desc 로 tie-break (NULL=0).
    let mine_t = my_identity_updated_at.unwrap_or(0);
    let theirs_t = existing.identity_updated_at.unwrap_or(0);
    if mine_t > theirs_t {
        return true;
    }
    if mine_t < theirs_t {
        return false;
    }
    // version·updated_at 모두 동률 — 같은 머신 계보면 이어받기, 아니면 안전 거부.
    match (my_origin_machine, existing.origin_machine.as_deref()) {
        (Some(a), Some(b)) if a == b => true,
        // 양쪽 다 머신 미상이거나 다르면 계보 판별 불가 → 안전 거부.
        _ => false,
    }
}

/// 같은 이름을 가진 모든 기존 신원 행 조회(정확 일치, 파싱·fuzzy 금지).
///
/// 정본 신원 이름 = `peers.display_name`. 대소문자 정책은 기존 동작(정확 일치) 유지.
fn fetch_same_name_rows(
    conn: &Connection,
    name: &str,
) -> rusqlite::Result<Vec<ExistingIdentity>> {
    let mut stmt = conn.prepare(
        "SELECT alias, display_name, public_key_hex, session_identifier, \
                session_status, last_seen, origin_machine, identity_version, \
                identity_updated_at \
         FROM peers WHERE display_name = ?1",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![name], |r| {
            Ok(ExistingIdentity {
                alias: r.get(0)?,
                display_name: r.get(1)?,
                public_key_hex: r.get(2)?,
                session_identifier: r.get(3)?,
                session_status: r.get(4)?,
                last_seen: r.get(5)?,
                origin_machine: r.get(6)?,
                identity_version: r.get(7)?,
                identity_updated_at: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 중앙 이름 유일성 판정 — 두 입구(MCP·GUI) 공용.
///
/// - `name`: 등록하려는 정본 신원 이름(display_name). 정확 일치 비교.
/// - `my_session_identifier`: 이번 등록 세션 식별자(`tmux:<name>` 등). 자기 판별용.
/// - `my_pubkey`: 이번 등록자의 public_key_hex. 자기 판별용(우선).
/// - `my_identity_version`/`my_identity_updated_at`/`my_origin_machine`: arbiter 입력(§J 갭#1).
///   NULL 규칙: version·updated_at 의 None 은 0 으로 취급(기존/신규 양쪽 동일) — arbiter 참조.
///
/// 반환:
/// - `Ok(New)` — 동명 없음 → 신규 등록.
/// - `Ok(SelfUpdate)` — 동명이 나 자신 → 갱신.
/// - `Ok(TakeoverDead{..})` — 동명이 죽은 다른 신원 + arbiter 통과 → 이어받기.
/// - `Err(NameConflictError)` — 살아있는 다른 신원 점유 또는 arbiter 동률.
pub fn check_name_available(
    conn: &Connection,
    name: &str,
    my_session_identifier: Option<&str>,
    my_pubkey: Option<&str>,
    my_identity_version: Option<i64>,
    my_identity_updated_at: Option<i64>,
    my_origin_machine: Option<&str>,
) -> Result<TakeoverDecision, NameConflictError> {
    let name = name.trim();
    // 빈 이름은 유일성 대상 아님 — 호출부에서 거르지만 방어적으로 New 처리.
    if name.is_empty() {
        return Ok(TakeoverDecision::New);
    }

    let rows = match fetch_same_name_rows(conn, name) {
        Ok(r) => r,
        // DB 조회 실패는 조용히 통과시키지 않는다(규칙 #1: fallback 금지).
        // 다만 시그니처상 에러 타입이 NameConflictError 이므로, 보수적으로 충돌로 보고
        // 호출부가 명시 에러를 내게 한다.
        Err(e) => {
            return Err(NameConflictError {
                conflict_name: name.to_string(),
                conflict_alias: String::new(),
                conflict_session: Some(format!("db_error: {e}")),
                reason: ConflictReason::ArbiterTie,
            });
        }
    };

    if rows.is_empty() {
        return Ok(TakeoverDecision::New);
    }

    // (1) 나 자신 판별 — 같은 pubkey 우선, 없으면 같은 session_identifier.
    let is_self = |e: &ExistingIdentity| -> bool {
        if let (Some(mine), Some(theirs)) = (my_pubkey, e.public_key_hex.as_deref()) {
            if !mine.is_empty() && mine.eq_ignore_ascii_case(theirs) {
                return true;
            }
        }
        if let (Some(mine), Some(theirs)) = (my_session_identifier, e.session_identifier.as_deref())
        {
            if !mine.is_empty() && mine == theirs {
                return true;
            }
        }
        false
    };
    if rows.iter().any(is_self) {
        return Ok(TakeoverDecision::SelfUpdate);
    }

    // (2) 동명 중 살아있는 다른 신원이 하나라도 있으면 → 충돌(에러).
    let live = live_tmux_sessions();
    let now = chrono::Utc::now();
    if let Some(alive) = rows.iter().find(|e| other_identity_alive(e, &live, now)) {
        return Err(NameConflictError {
            conflict_name: name.to_string(),
            conflict_alias: alive.alias.clone(),
            conflict_session: alive.session_identifier.clone(),
            reason: ConflictReason::SessionAlive,
        });
    }

    // (3) 모두 죽음 — arbiter 로 이어받기 가능 여부 판정. 하나라도 못 이기면 안전 거부.
    for e in &rows {
        if !arbiter_can_take_over(e, my_identity_version, my_identity_updated_at, my_origin_machine) {
            return Err(NameConflictError {
                conflict_name: name.to_string(),
                conflict_alias: e.alias.clone(),
                conflict_session: e.session_identifier.clone(),
                reason: ConflictReason::ArbiterTie,
            });
        }
    }

    // 모든 죽은 동명 신원을 arbiter 로 이길 수 있음 → 이어받기.
    let from = rows
        .first()
        .map(|e| e.alias.clone())
        .unwrap_or_default();
    Ok(TakeoverDecision::TakeoverDead { from_alias: from })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn last_seen_ttl_boundary() {
        let now = utc("2026-06-24T00:01:30Z");
        // 정확히 90초 전 → TTL 이내(<=)로 살아있음.
        assert!(last_seen_within_ttl(Some("2026-06-24T00:00:00Z"), 90, now));
        // 91초 전 → TTL 초과로 죽음 후보.
        assert!(!last_seen_within_ttl(Some("2026-06-23T23:59:59Z"), 90, now));
        // None / 파싱불가 → 살아있다고 단정 안 함.
        assert!(!last_seen_within_ttl(None, 90, now));
        assert!(!last_seen_within_ttl(Some("garbage"), 90, now));
    }

    #[test]
    fn session_live_matching() {
        let live = vec!["aoe_star_abc".to_string()];
        assert!(session_is_live(Some("tmux:aoe_star_abc"), &live));
        assert!(session_is_live(Some("aoe_star_abc"), &live));
        assert!(!session_is_live(Some("tmux:other"), &live));
        assert!(!session_is_live(None, &live));
        assert!(!session_is_live(Some("tmux:"), &live));
    }

    #[test]
    fn arbiter_version_wins() {
        let mut e = ExistingIdentity {
            alias: "old".into(),
            display_name: Some("Star".into()),
            public_key_hex: None,
            session_identifier: None,
            session_status: None,
            last_seen: None,
            origin_machine: Some("seoul".into()),
            identity_version: Some(3),
            identity_updated_at: Some(1000),
        };
        // 내 version 더 큼 → 이어받기(updated_at 무관).
        assert!(arbiter_can_take_over(&e, Some(4), Some(0), Some("zalman")));
        // 내 version 더 작음 → 거부.
        assert!(!arbiter_can_take_over(&e, Some(2), Some(9999), Some("seoul")));
        // version 동률 + 내 updated_at 더 큼 → 이어받기.
        assert!(arbiter_can_take_over(&e, Some(3), Some(2000), Some("zalman")));
        // version 동률 + 내 updated_at 더 작음 → 거부.
        assert!(!arbiter_can_take_over(&e, Some(3), Some(500), Some("seoul")));
        // version·updated_at 동률 + 같은 머신 → 이어받기.
        assert!(arbiter_can_take_over(&e, Some(3), Some(1000), Some("seoul")));
        // version·updated_at 동률 + 다른 머신 → 거부.
        assert!(!arbiter_can_take_over(&e, Some(3), Some(1000), Some("zalman")));
        // 모두 동률 + 머신 미상 → 거부. (NULL=0 규칙: updated_at None→0, 기존도 0 으로 맞춤)
        e.origin_machine = None;
        e.identity_updated_at = None;
        assert!(!arbiter_can_take_over(&e, Some(3), None, None));
    }
}
