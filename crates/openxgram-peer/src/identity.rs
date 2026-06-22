use crate::{PeerError, Result};
use openxgram_db::Db;

/// 정본 신원 매핑 저장소. PeerStore 패턴 미러 (`db: &mut Db`).
pub struct IdentityStore<'a> {
    db: &'a mut Db,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalGroup {
    pub canonical_address: String,
    pub primary_alias: Option<String>,
    pub aliases: Vec<String>,
    pub quarantined: bool,
}

// ─── 정본 로스터 (현황 그리드 정석 — peers + agent_profiles + tmux/ACP 세션 통합) ───
//
// 세 소스(peers / agent_profiles / 세션 detector)를 한 논리 신원으로 접는다.
// KakaoShell.tsx 의 unifiedRows(STEP A) + canonMachine(STEP B) 규칙을 Rust 로 포팅.
// 세션 detector 는 openxgram-cli 에 있으므로(peer crate 에서 못 봄), 이 빌더는
// 세션을 `SessionInput` 평탄 구조로 주입받는다(의존 역전). 호출자(daemon_gui)가 변환.

/// peers 테이블 1행의 로스터 입력 투영.
#[derive(Debug, Clone)]
pub struct PeerInput {
    pub alias: String,
    pub eth_address: Option<String>,
    pub session_identifier: Option<String>,
    pub role: Option<String>,
    pub display_name: Option<String>,
}

/// agent_profiles(+agent_capabilities) 1행의 로스터 입력 투영.
#[derive(Debug, Clone)]
pub struct AgentInput {
    pub alias: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub machine: Option<String>,
    /// agent_profiles 에는 cwd 컬럼이 없어 worktree 를 cwd 후보로 쓴다.
    pub cwd: Option<String>,
    pub session_identifier: Option<String>,
}

/// 세션 detector(openxgram-cli `DetectedSession`)의 로스터 입력 투영.
#[derive(Debug, Clone)]
pub struct SessionInput {
    /// detector 의 `identifier` (예: "tmux:aoe_flowsync_x").
    pub session_identifier: String,
    pub display_name: Option<String>,
    pub cwd: Option<String>,
}

/// 한 정본 신원당 1행. 세 소스의 필드를 병합.
#[derive(Debug, Clone, PartialEq)]
pub struct RosterEntry {
    pub canonical_address: Option<String>,
    /// 라우팅 alias — peer 의 full alias 우선(peer_send 가 이걸로 라우팅).
    pub primary_alias: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub machine: Option<String>,
    pub cwd: Option<String>,
    pub session_identifier: Option<String>,
    pub aliases: Vec<String>,
    pub is_peer: bool,
    pub has_agent: bool,
    pub has_tmux: bool,
    pub quarantined: bool,
}

/// session_identifier 정규화 — peer/agent/session 표현을 한 키로 접는다.
/// KakaoShell `normSid` 미러: `peer:<x>:` gossip prefix 제거 → `tmux:` 제거
/// → 양끝 `[ ]` 제거 → trim → lowercase.
pub fn norm_sid(s: Option<&str>) -> String {
    let raw = s.unwrap_or("").trim();
    // peer:<x>: gossip prefix (단일 세그먼트) 제거
    let after_peer = if let Some(rest) = raw.strip_prefix("peer:") {
        match rest.find(':') {
            Some(i) => &rest[i + 1..],
            None => rest,
        }
    } else {
        raw
    };
    let after_tmux = after_peer.strip_prefix("tmux:").unwrap_or(after_peer);
    // bracket 제거 — 프론트 `replace(/^\[|\]$/g,"")`: 선행 '[' 와 후행 ']' 각각 독립 제거.
    let no_lead = after_tmux.strip_prefix('[').unwrap_or(after_tmux);
    let no_trail = no_lead.strip_suffix(']').unwrap_or(no_lead);
    no_trail.trim().to_lowercase()
}

/// 머신 라벨 정규화 — KakaoShell `canonMachine` 미러.
/// lowercase+trim → seoul/서울 ⇒ "seoul" · zalman/잘만 ⇒ "zalman" ·
/// FQDN(점 포함)은 첫 세그먼트만 취하고 선행 "server-" 제거 후 재판정 ·
/// 빈값/None ⇒ `local_alias` · 그 외 ⇒ 정리된 첫 세그먼트.
pub fn normalize_machine(label: Option<&str>, local_alias: &str) -> String {
    let s = label.unwrap_or("").trim().to_lowercase();
    if s.is_empty() {
        // local_alias 자체도 FQDN/서버- 일 수 있으니 한 번 정규화(무한재귀 방지: 빈값 가드).
        if local_alias.trim().is_empty() {
            return String::new();
        }
        return normalize_machine(Some(local_alias), "");
    }
    let apply = |v: &str| -> Option<&'static str> {
        if v.contains("seoul") || v.contains("서울") {
            Some("seoul")
        } else if v.contains("zalman") || v.contains("잘만") {
            Some("zalman")
        } else {
            None
        }
    };
    if let Some(direct) = apply(&s) {
        return direct.to_string();
    }
    if s.contains('.') {
        let first = s.split('.').next().unwrap_or(&s);
        let first = first.strip_prefix("server-").unwrap_or(first);
        if let Some(seg) = apply(first) {
            return seg.to_string();
        }
        return if first.is_empty() { s.clone() } else { first.to_string() };
    }
    s.strip_prefix("server-").unwrap_or(&s).to_string()
}

impl<'a> IdentityStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// 변형 alias -> 정본 주소. 매핑 없으면 None.
    pub fn resolve(&mut self, alias: &str) -> Result<Option<String>> {
        let r = self.db.conn().query_row(
            "SELECT canonical_address FROM identity_aliases WHERE alias = ?1",
            [alias],
            |row| row.get::<_, String>(0),
        );
        match r {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// peers 를 스캔해 정본 신원으로 분류·매핑한다.
    /// 그룹핑 키: session_identifier(있으면) -> eth_address -> 둘 다 없으면 격리.
    /// 정본 주소: 그룹 내 role='primary' 의 eth_address (없으면 첫 행의 eth_address, 그것도 없으면 sid:<session>).
    pub fn reconcile(&mut self, now_rfc3339: &str) -> Result<()> {
        struct Row {
            alias: String,
            eth: Option<String>,
            sid: Option<String>,
            role: String,
        }
        let rows: Vec<Row> = {
            let mut stmt = self.db.conn().prepare(
                "SELECT alias, eth_address, session_identifier, role FROM peers ORDER BY created_at ASC",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok(Row {
                    alias: r.get(0)?,
                    eth: r.get(1)?,
                    sid: r.get(2)?,
                    role: r.get(3)?,
                })
            })?;
            let mut out = Vec::new();
            for r in mapped {
                out.push(r?);
            }
            out
        };

        use std::collections::BTreeMap;
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut quarantine: Vec<usize> = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let key = if let Some(sid) = &row.sid {
                Some(format!("sid:{sid}"))
            } else {
                row.eth.clone()
            };
            match key {
                Some(k) => groups.entry(k).or_default().push(i),
                None => quarantine.push(i),
            }
        }

        for (_key, idxs) in &groups {
            let primary_idx = idxs
                .iter()
                .copied()
                .find(|&i| rows[i].role == "primary")
                .unwrap_or(idxs[0]);
            let canonical_address = rows[primary_idx]
                .eth
                .clone()
                .or_else(|| idxs.iter().filter_map(|&i| rows[i].eth.clone()).next())
                .unwrap_or_else(|| {
                    rows[primary_idx]
                        .sid
                        .clone()
                        .map(|s| format!("sid:{s}"))
                        .unwrap_or_else(|| format!("alias:{}", rows[primary_idx].alias))
                });
            for &i in idxs {
                let is_primary = i == primary_idx;
                self.upsert_alias(&rows[i].alias, &canonical_address, is_primary, "active", now_rfc3339)?;
            }
        }

        for &i in &quarantine {
            let canon = format!("alias:{}", rows[i].alias);
            self.upsert_alias(&rows[i].alias, &canon, false, "quarantined", now_rfc3339)?;
        }

        Ok(())
    }

    /// 정본 주소별 그룹 목록 (현황 그리드 P2 용).
    pub fn groups(&mut self) -> Result<Vec<CanonicalGroup>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT canonical_address, alias, is_primary_alias, status
             FROM identity_aliases ORDER BY canonical_address, alias",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? != 0,
                r.get::<_, String>(3)?,
            ))
        })?;
        use std::collections::BTreeMap;
        let mut map: BTreeMap<String, CanonicalGroup> = BTreeMap::new();
        for row in rows {
            let (canon, alias, is_primary, status) = row?;
            let g = map.entry(canon.clone()).or_insert_with(|| CanonicalGroup {
                canonical_address: canon.clone(),
                primary_alias: None,
                aliases: Vec::new(),
                quarantined: false,
            });
            if is_primary {
                g.primary_alias = Some(alias.clone());
            }
            if status == "quarantined" {
                g.quarantined = true;
            }
            g.aliases.push(alias);
        }
        Ok(map.into_values().collect())
    }

    /// 세 소스를 DB 에서 로드해 정본 로스터를 만든다. 세션은 detector(cli)에서 주입.
    /// peers / agent_profiles(+agent_capabilities role) 는 여기서 읽고, 세션만 호출자가 변환해 전달.
    pub fn roster(
        &mut self,
        sessions: &[SessionInput],
        local_machine_alias: &str,
    ) -> Result<Vec<RosterEntry>> {
        // peers — eth_address(정본 주소)·session_identifier·role·display_name.
        let peers: Vec<PeerInput> = {
            let mut stmt = self.db.conn().prepare(
                "SELECT alias, eth_address, session_identifier, role, display_name
                 FROM peers ORDER BY created_at ASC",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok(PeerInput {
                    alias: r.get(0)?,
                    eth_address: r.get(1)?,
                    session_identifier: r.get(2)?,
                    role: r.get(3)?,
                    display_name: r.get(4)?,
                })
            })?;
            let mut out = Vec::new();
            for r in mapped {
                out.push(r?);
            }
            out
        };
        // agent_profiles + agent_capabilities(role). cwd = worktree(없으면 None).
        let agents: Vec<AgentInput> = {
            let mut stmt = self.db.conn().prepare(
                "SELECT p.alias, p.display_name, ac.role, p.machine,
                        COALESCE(ac.project_path, p.worktree)
                 FROM agent_profiles p
                 LEFT JOIN agent_capabilities ac ON ac.alias = p.alias
                 ORDER BY p.alias ASC",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok(AgentInput {
                    alias: r.get(0)?,
                    display_name: r.get(1)?,
                    role: r.get(2)?,
                    machine: r.get(3)?,
                    cwd: r.get(4)?,
                    session_identifier: None,
                })
            })?;
            let mut out = Vec::new();
            for r in mapped {
                out.push(r?);
            }
            out
        };
        Ok(Self::roster_from_sources(
            &peers,
            &agents,
            sessions,
            local_machine_alias,
        ))
    }

    /// 순수 그룹핑 로직(DB 비의존 — 단위 테스트 가능). KakaoShell unifiedRows STEP A 미러.
    /// 그룹핑 키 우선순위: canonical_address(eth) → normSid(session_identifier) → alias(lowercase).
    /// 소스 순서: peer(1차·정본+라우팅 alias) → agent → session. 나중 소스가 기존 행과
    /// 매칭되면 새 행 대신 병합(cwd/세션/머신/플래그 채움).
    pub fn roster_from_sources(
        peers: &[PeerInput],
        agents: &[AgentInput],
        sessions: &[SessionInput],
        local_machine_alias: &str,
    ) -> Vec<RosterEntry> {
        use std::collections::HashMap;

        // 행 저장소 + 3개 인덱스(canonical/sid/alias → 행 슬롯 인덱스).
        let mut rows: Vec<RosterEntry> = Vec::new();
        let mut by_canon: HashMap<String, usize> = HashMap::new();
        let mut by_sid: HashMap<String, usize> = HashMap::new();
        let mut by_alias: HashMap<String, usize> = HashMap::new();

        let index_row = |rows: &[RosterEntry],
                         by_canon: &mut HashMap<String, usize>,
                         by_sid: &mut HashMap<String, usize>,
                         by_alias: &mut HashMap<String, usize>,
                         slot: usize| {
            let r = &rows[slot];
            if let Some(c) = &r.canonical_address {
                by_canon.insert(c.to_lowercase(), slot);
            }
            let n = norm_sid(r.session_identifier.as_deref());
            if !n.is_empty() {
                by_sid.insert(n, slot);
            }
            if !r.primary_alias.is_empty() {
                by_alias.insert(r.primary_alias.to_lowercase(), slot);
            }
        };

        let find_slot = |by_canon: &HashMap<String, usize>,
                         by_sid: &HashMap<String, usize>,
                         by_alias: &HashMap<String, usize>,
                         canonical: Option<&str>,
                         sid: Option<&str>,
                         alias: Option<&str>|
         -> Option<usize> {
            if let Some(c) = canonical {
                if let Some(&s) = by_canon.get(&c.to_lowercase()) {
                    return Some(s);
                }
            }
            let n = norm_sid(sid);
            if !n.is_empty() {
                if let Some(&s) = by_sid.get(&n) {
                    return Some(s);
                }
            }
            if let Some(a) = alias {
                if let Some(&s) = by_alias.get(&a.to_lowercase()) {
                    return Some(s);
                }
            }
            None
        };

        // 1) peer 행 — 항상 1차. canonical_address + A2A 라우팅 alias.
        for p in peers {
            let canonical = p.eth_address.clone();
            let slot = find_slot(
                &by_canon,
                &by_sid,
                &by_alias,
                canonical.as_deref(),
                p.session_identifier.as_deref(),
                Some(&p.alias),
            );
            match slot {
                Some(s) => {
                    let r = &mut rows[s];
                    r.is_peer = true;
                    if r.canonical_address.is_none() {
                        r.canonical_address = canonical;
                    }
                    if r.display_name.is_none() {
                        r.display_name = p.display_name.clone();
                    }
                    if r.role.is_none() {
                        r.role = p.role.clone();
                    }
                    if r.session_identifier.is_none() {
                        r.session_identifier = p.session_identifier.clone();
                    }
                    if !r.aliases.contains(&p.alias) {
                        r.aliases.push(p.alias.clone());
                    }
                }
                None => {
                    rows.push(RosterEntry {
                        canonical_address: canonical,
                        primary_alias: p.alias.clone(),
                        display_name: p.display_name.clone(),
                        role: p.role.clone(),
                        machine: None,
                        cwd: None,
                        session_identifier: p.session_identifier.clone(),
                        aliases: vec![p.alias.clone()],
                        is_peer: true,
                        has_agent: false,
                        has_tmux: false,
                        quarantined: false,
                    });
                    let slot = rows.len() - 1;
                    index_row(&rows, &mut by_canon, &mut by_sid, &mut by_alias, slot);
                }
            }
        }

        // 2) agent 행 — peer 와 매칭되면 병합(cwd/machine/세션/role 채움), 아니면 신규.
        for a in agents {
            let slot = find_slot(
                &by_canon,
                &by_sid,
                &by_alias,
                None,
                a.session_identifier.as_deref(),
                Some(&a.alias),
            );
            match slot {
                Some(s) => {
                    let r = &mut rows[s];
                    r.has_agent = true;
                    if r.display_name.is_none() {
                        r.display_name = a.display_name.clone();
                    }
                    if r.role.is_none() {
                        r.role = a.role.clone();
                    }
                    if r.machine.is_none() {
                        r.machine = a.machine.clone();
                    }
                    if r.cwd.is_none() {
                        r.cwd = a.cwd.clone();
                    }
                    if r.session_identifier.is_none() {
                        r.session_identifier = a.session_identifier.clone();
                        let n = norm_sid(r.session_identifier.as_deref());
                        if !n.is_empty() {
                            by_sid.insert(n, s);
                        }
                    }
                    if !r.aliases.contains(&a.alias) {
                        r.aliases.push(a.alias.clone());
                    }
                }
                None => {
                    rows.push(RosterEntry {
                        canonical_address: None,
                        primary_alias: a.alias.clone(),
                        display_name: a.display_name.clone(),
                        role: a.role.clone(),
                        machine: a.machine.clone(),
                        cwd: a.cwd.clone(),
                        session_identifier: a.session_identifier.clone(),
                        aliases: vec![a.alias.clone()],
                        is_peer: false,
                        has_agent: true,
                        has_tmux: false,
                        quarantined: false,
                    });
                    let slot = rows.len() - 1;
                    index_row(&rows, &mut by_canon, &mut by_sid, &mut by_alias, slot);
                }
            }
        }

        // 3) session 행 — sid/alias 로 매칭되면 has_tmux + cwd/세션 채움, 아니면 standalone.
        for se in sessions {
            let sid = Some(se.session_identifier.as_str());
            // 세션은 alias 후보로 normSid 값을 쓴다(예: "tmux:aoe_X" → "aoe_x").
            let alias_guess = norm_sid(sid);
            let slot = find_slot(
                &by_canon,
                &by_sid,
                &by_alias,
                None,
                sid,
                if alias_guess.is_empty() {
                    None
                } else {
                    Some(alias_guess.as_str())
                },
            );
            match slot {
                Some(s) => {
                    let r = &mut rows[s];
                    r.has_tmux = true;
                    if r.session_identifier.is_none() {
                        r.session_identifier = Some(se.session_identifier.clone());
                    }
                    if r.cwd.is_none() {
                        r.cwd = se.cwd.clone();
                    }
                    if r.display_name.is_none() {
                        r.display_name = se.display_name.clone();
                    }
                }
                None => {
                    rows.push(RosterEntry {
                        canonical_address: None,
                        primary_alias: if alias_guess.is_empty() {
                            se.session_identifier.clone()
                        } else {
                            alias_guess.clone()
                        },
                        display_name: se.display_name.clone(),
                        role: None,
                        machine: None,
                        cwd: se.cwd.clone(),
                        session_identifier: Some(se.session_identifier.clone()),
                        aliases: vec![if alias_guess.is_empty() {
                            se.session_identifier.clone()
                        } else {
                            alias_guess.clone()
                        }],
                        is_peer: false,
                        has_agent: false,
                        has_tmux: true,
                        quarantined: true, // peer/agent 신원 없는 standalone 세션 = 격리 표시.
                    });
                    let slot = rows.len() - 1;
                    index_row(&rows, &mut by_canon, &mut by_sid, &mut by_alias, slot);
                }
            }
        }

        // tmux liveness 필터 — "유령 tmux" 행 제거(rc.358).
        // sessions 파라미터가 실제 살아있는 세션 집합(daemon_gui collect_sessions).
        // session_identifier 가 `tmux:` 로 시작하는데 그 normSid 가 live 집합에 없으면
        // session_identifier 를 None 으로 비운다 → 프론트에서 "active 아님"·종료 불가로 표시.
        // 주의: `tmux:` 접두만 대상. `aoe-acp:`·`peer:`·원격/크로스머신 sid 는 건드리지 않는다.
        let live_sids: std::collections::HashSet<String> = sessions
            .iter()
            .map(|s| norm_sid(Some(s.session_identifier.as_str())))
            .filter(|n| !n.is_empty())
            .collect();
        for r in &mut rows {
            let is_tmux = r
                .session_identifier
                .as_deref()
                .map(|s| s.starts_with("tmux:"))
                .unwrap_or(false);
            if is_tmux {
                let n = norm_sid(r.session_identifier.as_deref());
                if !live_sids.contains(&n) {
                    r.session_identifier = None;
                }
            }
        }

        // 머신 정규화 — 모든 행에 적용.
        for r in &mut rows {
            r.machine = Some(normalize_machine(r.machine.as_deref(), local_machine_alias));
        }

        rows
    }

    /// 정본 alias 재지정: 그룹 내 다른 행의 primary 해제 후 지정 alias 를 primary 로.
    /// 대상 alias 가 그룹에 없으면 아무것도 변경하지 않고 NotFound 반환(기존 primary 보존).
    pub fn set_primary_alias(&mut self, canonical_address: &str, alias: &str) -> Result<()> {
        // 존재 확인을 먼저 — 그래야 없는 alias 호출 시 기존 primary 를 날리지 않는다.
        let exists: bool = self.db.conn().query_row(
            "SELECT COUNT(*) FROM identity_aliases WHERE canonical_address = ?1 AND alias = ?2",
            rusqlite::params![canonical_address, alias],
            |r| r.get::<_, i64>(0),
        )? > 0;
        if !exists {
            return Err(PeerError::NotFound(format!(
                "alias '{alias}' 가 정본 '{canonical_address}' 그룹에 없음"
            )));
        }
        self.db.conn().execute(
            "UPDATE identity_aliases SET is_primary_alias = 0 WHERE canonical_address = ?1",
            [canonical_address],
        )?;
        self.db.conn().execute(
            "UPDATE identity_aliases SET is_primary_alias = 1 WHERE canonical_address = ?1 AND alias = ?2",
            rusqlite::params![canonical_address, alias],
        )?;
        Ok(())
    }

    /// 별칭 upsert. created_at 은 RFC3339 호출자 주입.
    pub fn upsert_alias(
        &mut self,
        alias: &str,
        canonical_address: &str,
        is_primary: bool,
        status: &str,
        created_at: &str,
    ) -> Result<()> {
        self.db.conn().execute(
            "INSERT INTO identity_aliases (alias, canonical_address, is_primary_alias, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(alias) DO UPDATE SET
               canonical_address = excluded.canonical_address,
               is_primary_alias  = excluded.is_primary_alias,
               status            = excluded.status",
            rusqlite::params![alias, canonical_address, is_primary as i64, status, created_at],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(alias: &str, sid: Option<&str>) -> PeerInput {
        PeerInput {
            alias: alias.to_string(),
            eth_address: None,
            session_identifier: sid.map(|s| s.to_string()),
            role: None,
            display_name: None,
        }
    }

    fn sess(id: &str) -> SessionInput {
        SessionInput {
            session_identifier: id.to_string(),
            display_name: None,
            cwd: None,
        }
    }

    fn row_for<'a>(rows: &'a [RosterEntry], alias: &str) -> &'a RosterEntry {
        rows.iter()
            .find(|r| r.primary_alias == alias || r.aliases.iter().any(|a| a == alias))
            .expect("행 존재")
    }

    /// tmux:live + 해당 세션이 sessions 에 있음 → sid 유지.
    #[test]
    fn test_liveness_keeps_live_tmux_sid() {
        let peers = vec![peer("aoe_live_x", Some("tmux:aoe_live_x"))];
        let sessions = vec![sess("tmux:aoe_live_x")];
        let rows =
            IdentityStore::roster_from_sources(&peers, &[], &sessions, "server-seoul");
        let r = row_for(&rows, "aoe_live_x");
        assert_eq!(r.session_identifier.as_deref(), Some("tmux:aoe_live_x"));
        assert!(r.has_tmux, "live tmux 세션과 병합");
    }

    /// tmux:dead + sessions 에 없음 → sid 비워짐(None).
    #[test]
    fn test_liveness_clears_dead_tmux_sid() {
        let peers = vec![peer("aoe_dead_x", Some("tmux:aoe_dead_x"))];
        let sessions: Vec<SessionInput> = vec![];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &sessions, "server-seoul");
        let r = row_for(&rows, "aoe_dead_x");
        assert_eq!(
            r.session_identifier, None,
            "죽은 tmux sid 는 None 으로 비워져야 함"
        );
    }

    /// aoe-acp:<id> 가 tmux 집합에 없어도 → sid 유지(tmux 접두 아님).
    #[test]
    fn test_liveness_keeps_aoe_acp_sid() {
        let peers = vec![peer("acp_agent", Some("aoe-acp:abc123"))];
        let sessions: Vec<SessionInput> = vec![];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &sessions, "server-seoul");
        let r = row_for(&rows, "acp_agent");
        assert_eq!(
            r.session_identifier.as_deref(),
            Some("aoe-acp:abc123"),
            "aoe-acp sid 는 tmux 접두가 아니므로 비우면 안 됨"
        );
    }

    /// peer:remote/크로스머신 sid → 유지(tmux 접두 아님).
    #[test]
    fn test_liveness_keeps_remote_peer_sid() {
        let peers = vec![peer("remote_agent", Some("peer:zalman:remote_agent"))];
        let sessions: Vec<SessionInput> = vec![];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &sessions, "server-seoul");
        let r = row_for(&rows, "remote_agent");
        assert_eq!(
            r.session_identifier.as_deref(),
            Some("peer:zalman:remote_agent"),
            "원격 peer sid 는 비우면 안 됨"
        );
    }

    /// live tmux 세션 단독 행(peer 없음) → 영향 없음(유지).
    #[test]
    fn test_liveness_session_only_row_unaffected() {
        let peers: Vec<PeerInput> = vec![];
        let sessions = vec![sess("tmux:lonely_session")];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &sessions, "server-seoul");
        let r = rows.iter().find(|r| r.has_tmux).expect("세션 행 존재");
        assert_eq!(
            r.session_identifier.as_deref(),
            Some("tmux:lonely_session"),
            "live 세션 단독 행 sid 유지"
        );
    }
}
