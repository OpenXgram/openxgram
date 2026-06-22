use openxgram_db::{Db, DbConfig};
use openxgram_peer::IdentityStore;
use tempfile::TempDir;

fn fresh_db(tmp: &TempDir) -> Db {
    let cfg = DbConfig {
        path: tmp.path().join("db.sqlite"),
        ..Default::default()
    };
    let mut db = Db::open(cfg).unwrap();
    db.migrate().unwrap();
    db
}

#[test]
fn test_upsert_and_resolve() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);
    let mut store = IdentityStore::new(&mut db);
    assert_eq!(store.resolve("star").unwrap(), None);
    store
        .upsert_alias("star", "0xDADA", true, "active", "2026-06-20T00:00:00+09:00")
        .unwrap();
    store
        .upsert_alias("starian", "0xDADA", false, "active", "2026-06-20T00:00:00+09:00")
        .unwrap();
    assert_eq!(store.resolve("star").unwrap(), Some("0xDADA".to_string()));
    assert_eq!(store.resolve("starian").unwrap(), Some("0xDADA".to_string()));
    store
        .upsert_alias("star", "0xBEEF", false, "active", "2026-06-20T00:00:00+09:00")
        .unwrap();
    assert_eq!(store.resolve("star").unwrap(), Some("0xBEEF".to_string()));
}

fn insert_peer(
    db: &mut openxgram_db::Db,
    alias: &str,
    eth: Option<&str>,
    sid: Option<&str>,
    role: &str,
) {
    // public_key_hex has a UNIQUE constraint, so we use the alias as a stand-in key.
    db.conn()
        .execute(
            "INSERT INTO peers (id, alias, public_key_hex, address, role, created_at, eth_address, session_identifier)
             VALUES (?1, ?2, ?2, 'http://x', ?3, '2026-06-20T00:00:00+09:00', ?4, ?5)",
            rusqlite::params![alias, alias, role, eth, sid],
        )
        .unwrap();
}

#[test]
fn test_reconcile_groups_by_session_then_address() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);
    insert_peer(&mut db, "star", Some("0xAAA"), Some("aoe_star_549029"), "primary");
    insert_peer(&mut db, "starian", Some("0xBBB"), Some("aoe_star_549029"), "worker");
    insert_peer(&mut db, "akashic", Some("0xCCC"), Some("aoe_akashic_1"), "worker");
    insert_peer(&mut db, "orphan", None, None, "worker");

    let mut store = IdentityStore::new(&mut db);
    store.reconcile("2026-06-20T00:00:00+09:00").unwrap();

    assert_eq!(store.resolve("star").unwrap(), Some("0xAAA".to_string()));
    assert_eq!(store.resolve("starian").unwrap(), Some("0xAAA".to_string()));
    assert_eq!(store.resolve("akashic").unwrap(), Some("0xCCC".to_string()));

    let status: String = db
        .conn()
        .query_row(
            "SELECT status FROM identity_aliases WHERE alias='orphan'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "quarantined");
}

#[test]
fn test_groups_and_set_primary() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);
    insert_peer(&mut db, "star", Some("0xAAA"), Some("aoe_star_549029"), "primary");
    insert_peer(&mut db, "starian", Some("0xBBB"), Some("aoe_star_549029"), "worker");

    let mut store = IdentityStore::new(&mut db);
    store.reconcile("2026-06-20T00:00:00+09:00").unwrap();

    let groups = store.groups().unwrap();
    let g = groups.iter().find(|g| g.canonical_address == "0xAAA").unwrap();
    assert_eq!(g.primary_alias.as_deref(), Some("star"));
    assert!(g.aliases.contains(&"star".to_string()));
    assert!(g.aliases.contains(&"starian".to_string()));
    assert!(!g.quarantined);

    store.set_primary_alias("0xAAA", "starian").unwrap();
    let groups2 = store.groups().unwrap();
    let g2 = groups2.iter().find(|g| g.canonical_address == "0xAAA").unwrap();
    assert_eq!(g2.primary_alias.as_deref(), Some("starian"));
}

#[test]
fn test_set_primary_missing_alias_preserves_existing() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);
    insert_peer(&mut db, "star", Some("0xAAA"), Some("aoe_star_549029"), "primary");
    insert_peer(&mut db, "starian", Some("0xBBB"), Some("aoe_star_549029"), "worker");

    let mut store = IdentityStore::new(&mut db);
    store.reconcile("2026-06-20T00:00:00+09:00").unwrap();

    // 존재하지 않는 alias 로 호출 → NotFound, 기존 primary(star) 보존 (그룹이 primary 없는 상태가 되면 안 됨)
    let err = store.set_primary_alias("0xAAA", "ghost").unwrap_err();
    assert!(matches!(err, openxgram_peer::PeerError::NotFound(_)));

    let groups = store.groups().unwrap();
    let g = groups.iter().find(|g| g.canonical_address == "0xAAA").unwrap();
    assert_eq!(g.primary_alias.as_deref(), Some("star"));
}

// ─── 정본 로스터 (현황 그리드 정석) ───────────────────────────────────────

use openxgram_peer::{normalize_machine, AgentInput, PeerInput, SessionInput};

fn insert_peer_full(
    db: &mut openxgram_db::Db,
    alias: &str,
    eth: Option<&str>,
    sid: Option<&str>,
    role: &str,
    display_name: Option<&str>,
) {
    db.conn()
        .execute(
            "INSERT INTO peers (id, alias, public_key_hex, address, role, created_at, eth_address, session_identifier, display_name)
             VALUES (?1, ?2, ?2, 'http://x', ?3, '2026-06-20T00:00:00+09:00', ?4, ?5, ?6)",
            rusqlite::params![alias, alias, role, eth, sid, display_name],
        )
        .unwrap();
}

fn insert_agent(
    db: &mut openxgram_db::Db,
    alias: &str,
    display_name: Option<&str>,
    machine: Option<&str>,
    worktree: Option<&str>,
    role: Option<&str>,
) {
    db.conn()
        .execute(
            "INSERT INTO agent_profiles (alias, machine, worktree, display_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, '2026-06-20T00:00:00+09:00', '2026-06-20T00:00:00+09:00')",
            rusqlite::params![alias, machine, worktree, display_name],
        )
        .unwrap();
    if let Some(r) = role {
        db.conn()
            .execute(
                "INSERT INTO agent_capabilities (alias, role, updated_at)
                 VALUES (?1, ?2, '2026-06-20T00:00:00+09:00')",
                rusqlite::params![alias, r],
            )
            .unwrap();
    }
}

fn sess(id: &str, cwd: Option<&str>, name: Option<&str>) -> SessionInput {
    SessionInput {
        session_identifier: id.to_string(),
        display_name: name.map(|s| s.to_string()),
        cwd: cwd.map(|s| s.to_string()),
    }
}

/// rc.360 — **병합 금지**. 같은 논리 에이전트가 peer + agent 로 흩어져 있으면
/// 각자 자기 행(접지 않음). 단, peer 가 들고 있는 세션과 그 gossip 중복은
/// peer 행의 라이브 증거로만 흡수돼 별도 standalone 세션 행을 만들지 않는다.
#[test]
fn test_roster_no_collapse_peer_and_agent_kept_separate() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);

    // peer: full alias + eth(정본 주소) + tmux sid.
    insert_peer_full(
        &mut db,
        "aoe_flowsync_x",
        Some("0xFLOW"),
        Some("tmux:aoe_flowsync_x"),
        "primary",
        Some("FlowSync"),
    );
    // agent_profiles: 짧은 alias "flowsync" + cwd(worktree) + machine.
    insert_agent(
        &mut db,
        "flowsync",
        Some("FlowSync"),
        Some("서울"),
        Some("/home/llm/projects/flowsync"),
        Some("builder"),
    );

    // sessions: 정상 tmux + gossip 중복(peer:other:tmux:...). 둘 다 normSid="aoe_flowsync_x".
    let sessions = vec![
        sess("tmux:aoe_flowsync_x", Some("/home/llm/projects/flowsync"), None),
        sess("peer:other:tmux:aoe_flowsync_x", None, None),
    ];

    let mut store = IdentityStore::new(&mut db);
    let roster = store.roster(&sessions, "server-seoul").unwrap();

    // peer 행 — 자기 행으로 존재. tmux 세션이 owned_sid 라 active.
    let peer_row = roster
        .iter()
        .find(|r| r.primary_alias == "aoe_flowsync_x")
        .expect("peer 행 존재");
    assert_eq!(peer_row.canonical_address.as_deref(), Some("0xFLOW"));
    assert!(peer_row.is_peer && !peer_row.has_agent, "peer 전용 행(병합 X)");
    assert!(peer_row.has_tmux, "owned tmux 세션 → active");
    assert_eq!(peer_row.status, "active");

    // agent 행 — 별도 행(접히지 않음).
    let agent_row = roster
        .iter()
        .find(|r| r.primary_alias == "flowsync")
        .expect("agent 행 별도 존재");
    assert!(agent_row.has_agent && !agent_row.is_peer, "agent 전용 행(병합 X)");
    assert_eq!(agent_row.machine.as_deref(), Some("seoul"));

    // gossip 중복 세션(peer 의 owned sid)이 별도 standalone 행을 만들지 않았는지.
    let standalone_sess = roster
        .iter()
        .filter(|r| {
            r.has_tmux
                && !r.is_peer
                && !r.has_agent
                && openxgram_peer::norm_sid(r.session_identifier.as_deref()) == "aoe_flowsync_x"
        })
        .count();
    assert_eq!(standalone_sess, 0, "owned 세션은 standalone 행 금지(dedup)");
}

/// rc.360 — peer + 같은 sid 를 든 agent → **두 행**(병합 안 함). 세션·gossip 중복은
/// owned_sid 로 흡수돼 standalone 행을 만들지 않는다.
#[test]
fn test_roster_peer_and_agent_same_sid_two_rows() {
    let peers = vec![PeerInput {
        alias: "aoe_flowsync_x".into(),
        eth_address: Some("0xFLOW".into()),
        session_identifier: Some("tmux:aoe_flowsync_x".into()),
        role: Some("primary".into()),
        display_name: Some("FlowSync".into()),
    }];
    // agent 가 같은 sid 를 들고 있어도 rc.360 에선 병합 안 함 — 별도 행.
    let agents = vec![AgentInput {
        alias: "flowsync".into(),
        display_name: Some("FlowSync".into()),
        role: Some("builder".into()),
        machine: Some("서울".into()),
        cwd: Some("/home/llm/projects/flowsync".into()),
        session_identifier: Some("tmux:aoe_flowsync_x".into()),
    }];
    let sessions = vec![
        sess("tmux:aoe_flowsync_x", None, None),
        sess("peer:remote:tmux:aoe_flowsync_x", None, None),
    ];

    let rows = IdentityStore::roster_from_sources(&peers, &agents, &sessions, "server-seoul");
    // peer 행 1 + agent 행 1 = 2 (owned 세션/gossip 은 standalone 행 안 만듦).
    assert_eq!(rows.len(), 2, "peer 행 + agent 행 별도(병합 금지, 세션 dedup)");
    let peer_row = rows.iter().find(|r| r.is_peer).expect("peer 행");
    assert_eq!(peer_row.canonical_address.as_deref(), Some("0xFLOW"));
    assert_eq!(peer_row.primary_alias, "aoe_flowsync_x");
    assert_eq!(peer_row.session_identifier.as_deref(), Some("tmux:aoe_flowsync_x"));
    assert!(peer_row.has_tmux);
    assert_eq!(peer_row.status, "active");
    let agent_row = rows.iter().find(|r| r.has_agent && !r.is_peer).expect("agent 행");
    assert_eq!(agent_row.primary_alias, "flowsync");
    assert_eq!(agent_row.cwd.as_deref(), Some("/home/llm/projects/flowsync"));
    assert_eq!(agent_row.machine.as_deref(), Some("seoul"));
    // agent 도 같은 sid(owned)라 active.
    assert_eq!(agent_row.status, "active");
}

/// agent-only(peer 없음) 와 session-only(peer/agent 없음)는 각자 자기 행.
#[test]
fn test_roster_agent_only_and_session_only() {
    let peers: Vec<PeerInput> = vec![];
    let agents = vec![AgentInput {
        alias: "soloagent".into(),
        display_name: Some("Solo".into()),
        role: Some("special".into()),
        machine: Some("zalman".into()),
        cwd: Some("/tmp/solo".into()),
        session_identifier: None,
    }];
    let sessions = vec![sess("tmux:lonely_session", Some("/tmp/lonely"), None)];

    let rows = IdentityStore::roster_from_sources(&peers, &agents, &sessions, "server-seoul");
    assert_eq!(rows.len(), 2);

    let agent = rows.iter().find(|r| r.primary_alias == "soloagent").unwrap();
    assert!(agent.has_agent && !agent.is_peer && !agent.has_tmux);
    assert_eq!(agent.machine.as_deref(), Some("zalman"));

    let session = rows.iter().find(|r| r.has_tmux).unwrap();
    assert!(!session.is_peer && !session.has_agent);
    assert_eq!(session.session_identifier.as_deref(), Some("tmux:lonely_session"));
    assert!(session.quarantined, "신원 없는 standalone 세션 = 격리");
    // session-only 머신은 라벨 없음 → local_alias 폴백.
    assert_eq!(session.machine.as_deref(), Some("seoul"));
}

#[test]
fn test_normalize_machine_cases() {
    assert_eq!(normalize_machine(Some("서울"), "server-seoul"), "seoul");
    assert_eq!(normalize_machine(Some("seoul"), "server-seoul"), "seoul");
    assert_eq!(normalize_machine(Some("Seoul"), "server-seoul"), "seoul");
    assert_eq!(
        normalize_machine(Some("server-seoul.c.teeup-492907.internal"), "x"),
        "seoul"
    );
    // null → local_alias 폴백(자체도 정규화).
    assert_eq!(normalize_machine(None, "server-seoul"), "seoul");
    assert_eq!(normalize_machine(Some(""), "잘만"), "zalman");
    assert_eq!(normalize_machine(Some("잘만"), "x"), "zalman");
    assert_eq!(normalize_machine(Some("zalman-wsl"), "x"), "zalman");
    // 미래 3번째 머신 — 정리된 첫 세그먼트(크래시 X).
    assert_eq!(normalize_machine(Some("server-macmini.local"), "x"), "macmini");
}

#[test]
fn test_norm_sid_strips_prefixes() {
    use openxgram_peer::norm_sid;
    assert_eq!(norm_sid(Some("tmux:aoe_flowsync_x")), "aoe_flowsync_x");
    assert_eq!(norm_sid(Some("peer:other:tmux:aoe_flowsync_x")), "aoe_flowsync_x");
    assert_eq!(norm_sid(Some("aoe_flowsync_x")), "aoe_flowsync_x");
    // 프론트 규칙 미러: bracket 제거는 tmux 제거 *후* 단계라, "[aoe_X]" → "aoe_x".
    assert_eq!(norm_sid(Some("[aoe_X]")), "aoe_x");
    assert_eq!(norm_sid(None), "");
}
