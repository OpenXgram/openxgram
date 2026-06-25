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
    /// 라이브 상태 캐시 (0064 컬럼): active | idle | disconnected | NULL.
    pub session_status: Option<String>,
    /// 마지막 연결 시각 (RFC3339). gossip 자동흡수 stub 식별·연결성 판정에 사용.
    pub last_seen: Option<String>,
    /// 등록 출처 메모. "auto-registered"/"auto-seed"/"auto-synced" prefix = gossip/테스트 stub.
    pub notes: Option<String>,
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

/// agent_capabilities 1행의 로스터 입력 투영 (rc.361).
/// list_peers 거울 — `messenger_enabled=1 OR alias IN peers` 인 모든 능력 행이
/// (peer 로 이미 표현되지 않았다면) 자기 1행이 된다. sv_aoe_* auto-seed 포함.
#[derive(Debug, Clone)]
pub struct CapInput {
    pub alias: String,
    pub role: Option<String>,
    pub description: Option<String>,
    /// peers 캐시(meta_map)에서 가져온 표시 필드 — list_peers 와 동일 소스.
    pub display_name: Option<String>,
    pub cwd: Option<String>,
    /// peers.session_identifier (있으면) — liveness 판정에 사용.
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
    /// rc.361 — agent_capabilities description (list_peers 거울). caps 소스 행에서 채움.
    pub description: Option<String>,
    pub machine: Option<String>,
    pub cwd: Option<String>,
    pub session_identifier: Option<String>,
    pub aliases: Vec<String>,
    pub is_peer: bool,
    pub has_agent: bool,
    pub has_tmux: bool,
    pub quarantined: bool,
    /// 생명주기 상태 (rc.360): "active" = 라이브 세션 존재 ·
    /// "stopped" = peer/agent 인데 세션 없음 · "dead" = session_identifier 가
    /// 가리키는 tmux/acp 세션이 사라짐. 프론트 현황 그리드 점등 표시용.
    pub status: String,
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

/// gossip/테스트 자동흡수 stub 여부 — notes prefix 로 판정.
/// `auto-registered`(envelope zero-touch) · `auto-seed`(tmux 자동 등록) ·
/// `auto-synced`(세션 동기화) 로 시작하면 사람이 등록한 신원이 아니라 자동 흡수 stub.
/// 이 stub 은 자체로는 라이브 증거가 아니므로(세션/active 상태가 따로 있어야 함),
/// 다른 라이브 신호가 없으면 현황 그리드에서 제외한다.
fn is_auto_stub(notes: Option<&str>) -> bool {
    let n = notes.unwrap_or("").trim().to_lowercase();
    n.starts_with("auto-registered") || n.starts_with("auto-seed") || n.starts_with("auto-synced")
}

/// peers.session_status 가 라이브(연결됨)인지 — `active`/`live`/`connected`.
fn is_live_status(status: Option<&str>) -> bool {
    matches!(
        status.unwrap_or("").trim().to_lowercase().as_str(),
        "active" | "live" | "connected"
    )
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
                "SELECT alias, eth_address, session_identifier, role, display_name,
                        session_status, last_seen, notes
                 FROM peers ORDER BY created_at ASC",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok(PeerInput {
                    alias: r.get(0)?,
                    eth_address: r.get(1)?,
                    session_identifier: r.get(2)?,
                    role: r.get(3)?,
                    display_name: r.get(4)?,
                    session_status: r.get(5)?,
                    last_seen: r.get(6)?,
                    notes: r.get(7)?,
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
        // agent_capabilities (rc.361) — list_peers 거울. 동일 필터
        //   `messenger_enabled=1 OR alias IN peers` 적용. peer/agent_profiles 로 이미
        //   표현된 alias 는 제외(중복 방지) — 그래야 distinct alias 집합이 list_peers 와
        //   일치한다(peers + caps). display_name/cwd/session_identifier 는 peers 캐시
        //   (display_name/cwd/session_identifier) 가 있으면 끌어와 list_peers 와 동일 소스.
        let caps: Vec<CapInput> = {
            let mut stmt = self.db.conn().prepare(
                "SELECT ac.alias, ac.role, ac.description,
                        p.display_name, p.cwd, p.session_identifier
                 FROM agent_capabilities ac
                 LEFT JOIN peers p ON p.alias = ac.alias
                 WHERE (ac.messenger_enabled = 1 OR ac.alias IN (SELECT alias FROM peers))
                   AND ac.alias NOT IN (SELECT alias FROM peers)
                   AND ac.alias NOT IN (SELECT alias FROM agent_profiles)
                 ORDER BY ac.alias ASC",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok(CapInput {
                    alias: r.get(0)?,
                    role: r.get(1)?,
                    description: r.get(2)?,
                    display_name: r.get(3)?,
                    cwd: r.get(4)?,
                    session_identifier: r.get(5)?,
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
            &caps,
            sessions,
            local_machine_alias,
        ))
    }

    /// 순수 행 빌더(DB 비의존 — 단위 테스트 가능). rc.360 — **병합 금지**.
    /// 현황 그리드 = `list_peers` 거울 = 전체 에이전트 생명주기 콘솔이므로,
    /// **모든 소스 행이 자기 1행**이 된다(같은 정본/sid/alias 라도 접지 않음).
    /// 정본은 `canonical_address` 필드로만 노출 — 프론트가 그룹 헤더/주석으로 묶는다.
    ///
    /// 행 생성 규칙(소스별 자기 1행):
    /// - peers 테이블 1행 = RosterEntry 1행 (is_peer)
    /// - agent_profiles 1행 = RosterEntry 1행 (has_agent) — peer 와 alias 겹쳐도 별도 행
    /// - 라이브 세션 1개 = RosterEntry 1행 (has_tmux) — 단, 이미 peer/agent 가 같은
    ///   normSid 를 들고 있으면 그 행을 active 로 표시하고 중복 세션 standalone 행은 만들지 않는다
    ///   (세션은 신원이 아니라 peer/agent 의 라이브 증거이기 때문). peer/agent 어느 행도
    ///   해당 세션을 안 들고 있으면 그때만 standalone 세션 행을 만든다.
    ///
    /// status(rc.360): "active" = 라이브 세션과 연결됨 · "dead" = `tmux:` sid 인데
    /// 라이브 집합에 없음 · "stopped" = 그 외(세션 없는 peer/agent, 또는 비-tmux sid 로
    /// 라이브 판정 불가). 라이브 판정은 `sessions` 인자의 normSid 집합.
    ///
    /// rc.361 — `caps`(agent_capabilities) 도 소스로 추가하여 현황 그리드 == `list_peers`.
    /// list_peers 의 dedup 규칙을 미러: peer 로 이미 표현된 alias 의 능력 행은 건너뛰고
    /// (peer 행이 그 신원을 대표), 그 외 능력 행(sv_aoe_* auto-seed 포함)은 각자 1행
    /// (`has_agent=true`). 호출자(`roster()`)가 `messenger_enabled=1 OR alias IN peers`
    /// 필터를 적용해 전달하므로 여기선 peer-alias 중복만 제거한다.
    pub fn roster_from_sources(
        peers: &[PeerInput],
        agents: &[AgentInput],
        caps: &[CapInput],
        sessions: &[SessionInput],
        local_machine_alias: &str,
    ) -> Vec<RosterEntry> {
        use std::collections::HashSet;

        // 라이브 세션 normSid 집합 — status 계산 + standalone 세션 중복 제거에 사용.
        let live_sids: HashSet<String> = sessions
            .iter()
            .map(|s| norm_sid(Some(s.session_identifier.as_str())))
            .filter(|n| !n.is_empty())
            .collect();

        // peer alias 집합 — caps 행 dedup(list_peers `alias NOT IN peers` 미러)에 사용.
        let peer_aliases: HashSet<String> = peers.iter().map(|p| p.alias.clone()).collect();

        // 정본 agent_profiles 등록 alias 집합 — spec #1: 세션이 꺼져 있어도(spawn 가능)
        // 항상 현황 그리드에 남는다. caps(능력 거울) 행과 구분하기 위해 별도 집합으로 유지.
        let profile_aliases: HashSet<String> = agents.iter().map(|a| a.alias.clone()).collect();

        // 라이브/연결된 peer alias 집합 — 현황 그리드 포함 판정에 사용.
        //   포함 조건(peers 행): session_status 가 active/live  OR
        //   라이브 세션 sid 와 매칭  OR  (notes 가 auto-stub 가 아니고 last_seen 존재).
        //   bare gossip/테스트 stub(auto-registered/auto-seed/auto-synced + 세션·active 없음)은 제외.
        let live_peer_aliases: HashSet<String> = peers
            .iter()
            .filter(|p| {
                let live_sid = {
                    let n = norm_sid(p.session_identifier.as_deref());
                    !n.is_empty() && live_sids.contains(&n)
                };
                if is_live_status(p.session_status.as_deref()) || live_sid {
                    return true;
                }
                // 세션 증거가 없으면 — auto-stub 은 제외, 사람이 등록한 행(non-stub)만 유지.
                !is_auto_stub(p.notes.as_deref()) && p.last_seen.is_some()
            })
            .map(|p| p.alias.clone())
            .collect();

        let mut rows: Vec<RosterEntry> = Vec::new();
        // peer/agent 가 들고 있는 normSid 집합 — standalone 세션 행 중복 방지.
        let mut owned_sids: HashSet<String> = HashSet::new();

        // 1) peers — 한 행씩 그대로.
        for p in peers {
            let n = norm_sid(p.session_identifier.as_deref());
            if !n.is_empty() {
                owned_sids.insert(n);
            }
            rows.push(RosterEntry {
                canonical_address: p.eth_address.clone(),
                primary_alias: p.alias.clone(),
                display_name: p.display_name.clone(),
                role: p.role.clone(),
                description: None,
                machine: None,
                cwd: None,
                session_identifier: p.session_identifier.clone(),
                aliases: vec![p.alias.clone()],
                is_peer: true,
                has_agent: false,
                has_tmux: false,
                quarantined: false,
                status: String::new(), // 아래에서 계산
            });
        }

        // 1.5) agent_capabilities (rc.361) — peer 로 이미 표현된 alias 만 건너뛰고
        //      나머지는 각자 1행(has_agent). list_peers 의 "kind=agent" 추가와 동일 집합.
        for c in caps {
            if peer_aliases.contains(&c.alias) {
                continue; // peer 행이 이 신원을 대표(list_peers dedup 미러).
            }
            let n = norm_sid(c.session_identifier.as_deref());
            if !n.is_empty() {
                owned_sids.insert(n);
            }
            rows.push(RosterEntry {
                canonical_address: None,
                primary_alias: c.alias.clone(),
                display_name: c.display_name.clone(),
                role: c.role.clone(),
                description: c.description.clone(),
                machine: None,
                cwd: c.cwd.clone(),
                session_identifier: c.session_identifier.clone(),
                aliases: vec![c.alias.clone()],
                is_peer: false,
                has_agent: true,
                has_tmux: false,
                quarantined: false,
                status: String::new(),
            });
        }

        // 2) agent_profiles — 한 행씩 그대로(peer 와 alias 겹쳐도 별도 행).
        for a in agents {
            let n = norm_sid(a.session_identifier.as_deref());
            if !n.is_empty() {
                owned_sids.insert(n);
            }
            rows.push(RosterEntry {
                canonical_address: None,
                primary_alias: a.alias.clone(),
                display_name: a.display_name.clone(),
                role: a.role.clone(),
                description: None,
                machine: a.machine.clone(),
                cwd: a.cwd.clone(),
                session_identifier: a.session_identifier.clone(),
                aliases: vec![a.alias.clone()],
                is_peer: false,
                has_agent: true,
                has_tmux: false,
                quarantined: false,
                status: String::new(),
            });
        }

        // 3) 세션 — peer/agent 가 이미 들고 있는 세션이면 그 행을 active 로 표시(아래 status
        //    계산이 처리). 그 외(어떤 peer/agent 도 안 들고 있는 라이브 세션)만 standalone 행 생성.
        for se in sessions {
            let n = norm_sid(Some(se.session_identifier.as_str()));
            if !n.is_empty() && owned_sids.contains(&n) {
                continue; // 이미 peer/agent 행이 이 세션을 증거로 들고 있음 → 중복 standalone 금지.
            }
            let alias_guess = if n.is_empty() {
                se.session_identifier.clone()
            } else {
                n
            };
            rows.push(RosterEntry {
                canonical_address: None,
                primary_alias: alias_guess.clone(),
                display_name: se.display_name.clone(),
                role: None,
                description: None,
                machine: None,
                cwd: se.cwd.clone(),
                session_identifier: Some(se.session_identifier.clone()),
                aliases: vec![alias_guess],
                is_peer: false,
                has_agent: false,
                has_tmux: true,
                quarantined: true, // peer/agent 신원 없는 standalone 세션 = 격리 표시.
                status: String::new(),
            });
        }

        // status 계산 + has_tmux 표시 — liveness-hide 대신 status 로 노출(rc.360).
        // `tmux:` sid: normSid 가 라이브 집합에 있으면 active(+has_tmux), 없으면 dead.
        // 비-tmux sid(aoe-acp:/peer:/원격): 라이브 집합과 매칭되면 active, 아니면 stopped
        //   (원격/ACP 의 생존은 여기서 단정 못 하므로 보수적으로 stopped, sid 는 보존).
        // sid 없는 peer/agent: stopped. standalone 세션 행: 항상 active(라이브 증거).
        for r in &mut rows {
            let sid = r.session_identifier.clone();
            let n = norm_sid(sid.as_deref());
            let is_tmux = sid.as_deref().map(|s| s.starts_with("tmux:")).unwrap_or(false);
            let live = !n.is_empty() && live_sids.contains(&n);
            if r.has_tmux && !r.is_peer && !r.has_agent {
                // standalone 세션 행 — 정의상 라이브.
                r.status = "active".to_string();
            } else if sid.is_none() || n.is_empty() {
                r.status = "stopped".to_string();
            } else if live {
                r.status = "active".to_string();
                if is_tmux {
                    r.has_tmux = true;
                }
            } else if is_tmux {
                // tmux sid 인데 라이브 집합에 없음 → 죽은 세션. sid 는 유지(프론트 표시·tooltip).
                r.status = "dead".to_string();
            } else {
                // 비-tmux sid(원격/ACP) — 라이브 판정 불가 → stopped(보존).
                r.status = "stopped".to_string();
            }
        }

        // 머신 정규화 — 모든 행에 적용.
        for r in &mut rows {
            r.machine = Some(normalize_machine(r.machine.as_deref(), local_machine_alias));
        }

        // ── 포함 필터(현황 그리드 source pollution 제거) ──
        // 행은 다음 중 하나 이상이면 EMIT, 아니면 stale gossip/테스트 echo 로 간주해 제외:
        //   1) 라이브 세션 증거: has_tmux == true  또는  status == "active".
        //   2) 정본 agent_profiles 등록: primary_alias ∈ profile_aliases
        //      (세션이 꺼져 있어도 spawn 위해 유지 — spec #1).
        //   3) 연결된 peer: is_peer == true 이고 primary_alias ∈ live_peer_aliases.
        // 제외 대상:
        //   - bare peers 행(profile/세션/active 없음 + auto-stub or last_seen 부재).
        //   - caps echo 행(has_agent=true 지만 profile/세션/active 없음 — 예: 테스트 stub).
        let before = rows.len();
        rows.retain(|r| {
            let live = r.has_tmux || r.status == "active";
            let registered = profile_aliases.contains(&r.primary_alias);
            let connected_peer = r.is_peer && live_peer_aliases.contains(&r.primary_alias);
            live || registered || connected_peer
        });
        let dropped = before - rows.len();
        if dropped > 0 {
            tracing::debug!(
                dropped,
                kept = rows.len(),
                "roster: stale gossip/test echo 행 제외(현황 그리드 정제)"
            );
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
            session_status: None,
            // 사람이 등록한 실제 peer 모델 — last_seen 존재(stub 아님) → 포함 필터 통과.
            last_seen: Some("2026-06-26T00:00:00+09:00".to_string()),
            notes: None,
        }
    }

    fn peer_eth(alias: &str, eth: &str) -> PeerInput {
        PeerInput {
            alias: alias.to_string(),
            eth_address: Some(eth.to_string()),
            session_identifier: None,
            role: None,
            display_name: None,
            session_status: None,
            last_seen: Some("2026-06-26T00:00:00+09:00".to_string()),
            notes: None,
        }
    }

    fn agent(alias: &str) -> AgentInput {
        AgentInput {
            alias: alias.to_string(),
            display_name: None,
            role: None,
            machine: None,
            cwd: None,
            session_identifier: None,
        }
    }

    fn sess(id: &str) -> SessionInput {
        SessionInput {
            session_identifier: id.to_string(),
            display_name: None,
            cwd: None,
        }
    }

    fn cap(alias: &str) -> CapInput {
        CapInput {
            alias: alias.to_string(),
            role: Some("agent".to_string()),
            description: None,
            display_name: None,
            cwd: None,
            session_identifier: None,
        }
    }

    /// 라이브 세션 sid 를 든 caps 행 — 포함 필터 통과(현황 그리드 유지) 검증용.
    fn cap_sid(alias: &str, sid: &str) -> CapInput {
        CapInput {
            session_identifier: Some(sid.to_string()),
            ..cap(alias)
        }
    }

    fn row_for<'a>(rows: &'a [RosterEntry], alias: &str) -> &'a RosterEntry {
        rows.iter()
            .find(|r| r.primary_alias == alias || r.aliases.iter().any(|a| a == alias))
            .expect("행 존재")
    }

    /// rc.360 핵심: 같은 eth(정본) 두 alias → 접지 않고 **두 행**. canonical_address 는 보존.
    #[test]
    fn test_no_collapse_two_aliases_same_eth() {
        let peers = vec![
            peer_eth("alias_a", "0xABC"),
            peer_eth("alias_b", "0xABC"),
        ];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &[], &[], "server-seoul");
        let n: usize = rows.iter().filter(|r| r.is_peer).count();
        assert_eq!(n, 2, "같은 eth 라도 두 peer = 두 행 (접지 않음)");
        for a in ["alias_a", "alias_b"] {
            let r = row_for(&rows, a);
            assert_eq!(r.canonical_address.as_deref(), Some("0xABC"), "정본 보존");
        }
    }

    /// peer + 같은 alias 의 agent_profiles → **두 행**(peer 행 1 + agent 행 1).
    #[test]
    fn test_no_collapse_peer_and_agent_same_alias() {
        let peers = vec![peer("dup", None)];
        let agents = vec![agent("dup")];
        let rows = IdentityStore::roster_from_sources(&peers, &agents, &[], &[], "server-seoul");
        assert_eq!(rows.len(), 2, "peer 행 + agent 행 별도");
        assert!(rows.iter().any(|r| r.is_peer && !r.has_agent));
        assert!(rows.iter().any(|r| r.has_agent && !r.is_peer));
    }

    /// 프로필 없는 peer 도 행으로 나온다(peers 테이블 직접 투영).
    #[test]
    fn test_profileless_peer_appears() {
        let peers = vec![peer("lonely_peer", None)];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &[], &[], "server-seoul");
        let r = row_for(&rows, "lonely_peer");
        assert!(r.is_peer && !r.has_agent);
        assert_eq!(r.status, "stopped", "세션 없는 peer = stopped");
    }

    /// status: 라이브 tmux 세션과 연결된 peer → active(+has_tmux), 행은 그대로 유지.
    #[test]
    fn test_status_live_tmux_active() {
        let peers = vec![peer("aoe_live_x", Some("tmux:aoe_live_x"))];
        let sessions = vec![sess("tmux:aoe_live_x")];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &[], &sessions, "server-seoul");
        let r = row_for(&rows, "aoe_live_x");
        assert_eq!(r.session_identifier.as_deref(), Some("tmux:aoe_live_x"), "sid 유지");
        assert_eq!(r.status, "active");
        assert!(r.has_tmux, "라이브 tmux 표시");
        // owned_sids dedupe — standalone 세션 행이 추가로 생기지 않아야 함.
        assert_eq!(rows.iter().filter(|r| r.has_tmux).count(), 1, "세션 중복 행 금지");
    }

    /// status: tmux sid 인데 라이브 집합에 없음 → dead, sid 는 유지(숨기지 않음).
    #[test]
    fn test_status_dead_tmux() {
        let peers = vec![peer("aoe_dead_x", Some("tmux:aoe_dead_x"))];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &[], &[], "server-seoul");
        let r = row_for(&rows, "aoe_dead_x");
        assert_eq!(r.status, "dead", "죽은 tmux 는 dead");
        assert_eq!(
            r.session_identifier.as_deref(),
            Some("tmux:aoe_dead_x"),
            "rc.360 — 죽어도 sid 유지(숨기지 않음)"
        );
    }

    /// status: 비-tmux(aoe-acp/원격) sid → stopped, sid 유지.
    #[test]
    fn test_status_non_tmux_sid_stopped() {
        let peers = vec![
            peer("acp_agent", Some("aoe-acp:abc123")),
            peer("remote_agent", Some("peer:zalman:remote_agent")),
        ];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &[], &[], "server-seoul");
        let acp = row_for(&rows, "acp_agent");
        assert_eq!(acp.session_identifier.as_deref(), Some("aoe-acp:abc123"));
        assert_eq!(acp.status, "stopped");
        let rem = row_for(&rows, "remote_agent");
        assert_eq!(rem.session_identifier.as_deref(), Some("peer:zalman:remote_agent"));
        assert_eq!(rem.status, "stopped");
    }

    /// standalone 라이브 세션(peer/agent 없음) → 자기 행, active.
    #[test]
    fn test_standalone_session_active() {
        let sessions = vec![sess("tmux:lonely_session")];
        let rows = IdentityStore::roster_from_sources(&[], &[], &[], &sessions, "server-seoul");
        let r = rows.iter().find(|r| r.has_tmux).expect("세션 행 존재");
        assert_eq!(r.session_identifier.as_deref(), Some("tmux:lonely_session"));
        assert_eq!(r.status, "active");
    }

    /// rc.361 — 현황 그리드(roster) ≡ MCP list_peers 의 alias 집합 패리티.
    /// list_peers = peers ∪ (agent_capabilities[messenger=1 OR in peers] − peers).
    /// 포함 필터(현황 그리드 정제) 이후에도, **라이브 증거가 있는** 행은 모두 살아남고
    /// distinct primary_alias 집합이 list_peers 의 alias 집합과 같아야 한다.
    /// (caps 행은 라이브 세션을 들고 있을 때 유지 — bare caps echo 는 별도 테스트에서 제외 확인.)
    #[test]
    fn test_roster_parity_with_list_peers_alias_set() {
        use std::collections::HashSet;

        // peers 3개(그 중 하나는 caps 와 alias 겹침 → list_peers 는 peer 로만 1행).
        let peers = vec![
            peer("aoe_akashic_x", Some("tmux:aoe_akashic_x")),
            peer("seoul", None),
            peer("zalman", None),
        ];
        // caps: 라이브 세션을 들고 있는 능력 행(현황 그리드 포함 대상).
        let caps = vec![
            cap_sid("sv_aoe_flow_1", "tmux:sv_aoe_flow_1"),
            cap_sid("sv_aoe_flow_2", "tmux:sv_aoe_flow_2"),
            cap_sid("res", "tmux:res"),
        ];
        // 위 caps 세션이 라이브로 보이도록 세션 detector 입력에 동일 sid 주입.
        let sessions = vec![
            sess("tmux:sv_aoe_flow_1"),
            sess("tmux:sv_aoe_flow_2"),
            sess("tmux:res"),
        ];

        // list_peers 기대 alias 집합 = peers ∪ caps(peer 제거 후).
        let mut expected: HashSet<String> = peers.iter().map(|p| p.alias.clone()).collect();
        for c in &caps {
            expected.insert(c.alias.clone());
        }

        let rows =
            IdentityStore::roster_from_sources(&peers, &[], &caps, &sessions, "server-seoul");
        let got: HashSet<String> = rows.iter().map(|r| r.primary_alias.clone()).collect();

        assert_eq!(got, expected, "roster alias 집합 == list_peers alias 집합");
        assert_eq!(rows.len(), 6, "peers(3) + 라이브 caps(3) = 6 행, 중복 없음");
        // caps 행은 has_agent, peer 행은 is_peer.
        assert!(rows.iter().filter(|r| r.has_agent && !r.is_peer).count() == 3);
        assert!(rows.iter().filter(|r| r.is_peer).count() == 3);
    }

    /// 포함 필터: 라이브 증거(세션/active/profile/연결 peer) 없는 **bare caps echo**
    /// (예: 테스트 stub eno-test/pip-test)는 현황 그리드에서 제외된다.
    #[test]
    fn test_bare_caps_echo_excluded() {
        let caps = vec![cap("eno-test"), cap("pip-test")];
        let rows = IdentityStore::roster_from_sources(&[], &[], &caps, &[], "server-seoul");
        assert!(
            rows.is_empty(),
            "라이브 증거 없는 caps echo 는 현황 그리드에서 제외(stale gossip/test)"
        );
    }

    /// 포함 필터: bare gossip/test peer(profile·세션·active 없음 + auto-stub notes)는 제외.
    #[test]
    fn test_bare_gossip_peer_excluded() {
        let peers = vec![PeerInput {
            alias: "mptest".into(),
            eth_address: None,
            session_identifier: None,
            role: Some("primary".into()),
            display_name: None,
            session_status: None,
            last_seen: Some("2026-06-26T00:00:00+09:00".into()),
            notes: Some("auto-registered via envelope (zero-touch rc.244)".into()),
        }];
        let rows = IdentityStore::roster_from_sources(&peers, &[], &[], &[], "server-seoul");
        assert!(
            rows.is_empty(),
            "auto-registered stub + 세션/active 없음 = 제외(현황 그리드 정제)"
        );
    }

    /// rc.361 — peer alias 와 겹치는 caps 행은 roster_from_sources 가 직접 제거한다
    /// (list_peers 의 `alias NOT IN peers` dedup 미러). 호출자 누락 시에도 중복 방지.
    #[test]
    fn test_caps_dedup_against_peer_alias() {
        let peers = vec![peer("seoul", None)];
        // res 는 라이브 세션을 들고 있어야 포함 필터를 통과(bare echo 제외 규칙).
        let caps = vec![cap("seoul"), cap_sid("res", "tmux:res")];
        let sessions = vec![sess("tmux:res")];
        let rows =
            IdentityStore::roster_from_sources(&peers, &[], &caps, &sessions, "server-seoul");
        // seoul = peer 1행만(caps 의 seoul 은 흡수). res = caps 1행. 총 2.
        assert_eq!(rows.len(), 2, "peer 와 겹치는 caps alias 는 제거");
        assert_eq!(rows.iter().filter(|r| r.primary_alias == "seoul").count(), 1);
    }
}
