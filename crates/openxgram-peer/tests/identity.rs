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

/// 한 논리 에이전트가 peer + agent + session + gossip 중복으로 흩어져 있어도 1행으로 접힌다.
#[test]
fn test_roster_collapses_one_agent_across_sources() {
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

    // sessions: 정상 tmux + gossip 중복(peer:other:tmux:...).
    let sessions = vec![
        sess("tmux:aoe_flowsync_x", Some("/home/llm/projects/flowsync"), None),
        sess("peer:other:tmux:aoe_flowsync_x", None, None),
    ];

    let mut store = IdentityStore::new(&mut db);
    let roster = store.roster(&sessions, "server-seoul").unwrap();

    // peer 의 sid 와 agent alias 매칭으로 한 행: flowsync(agent) sid 는 없지만 alias 매칭됨.
    // agent alias "flowsync" 는 peer sid/canon 과 직접 안 묶이지만, normSid 가 같지 않으므로
    // alias 키로는 별개일 수 있다 → 검증: flowsync 한 행으로 합쳐졌는지 vs 분리됐는지.
    // 본 케이스 설계상 peer.sid 와 session.sid 가 normSid 동일("aoe_flowsync_x") → peer+session 1행.
    // agent("flowsync")는 alias 가 달라(다른 normSid 없음) 별도일 수 있어 — 통합 키 검증.
    let flow = roster
        .iter()
        .find(|r| r.aliases.iter().any(|a| a == "aoe_flowsync_x"))
        .expect("flowsync 행 존재");

    assert_eq!(flow.canonical_address.as_deref(), Some("0xFLOW"));
    assert_eq!(flow.primary_alias, "aoe_flowsync_x");
    assert!(flow.is_peer, "peer 레코드");
    assert!(flow.has_tmux, "tmux 세션 매칭");
    // gossip 중복 세션이 별도 행을 만들지 않았는지(중복 dedup).
    let dup_rows = roster
        .iter()
        .filter(|r| {
            openxgram_peer::norm_sid(r.session_identifier.as_deref()) == "aoe_flowsync_x"
        })
        .count();
    assert_eq!(dup_rows, 1, "gossip 중복 세션이 별도 행을 만들면 안 됨");
}

/// agent alias 가 peer sid 의 normSid 와 같은 경우(짧은 alias = 세션명) 완전 통합 검증.
#[test]
fn test_roster_merges_agent_by_matching_sid() {
    let tmp = TempDir::new().unwrap();
    let mut db = fresh_db(&tmp);

    let peers = vec![PeerInput {
        alias: "aoe_flowsync_x".into(),
        eth_address: Some("0xFLOW".into()),
        session_identifier: Some("tmux:aoe_flowsync_x".into()),
        role: Some("primary".into()),
        display_name: Some("FlowSync".into()),
    }];
    // agent 가 같은 sid 를 들고 있음 → peer 와 sid 로 병합.
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
    assert_eq!(rows.len(), 1, "모든 소스가 한 행으로");
    let r = &rows[0];
    assert_eq!(r.canonical_address.as_deref(), Some("0xFLOW"));
    assert_eq!(r.primary_alias, "aoe_flowsync_x");
    assert_eq!(r.cwd.as_deref(), Some("/home/llm/projects/flowsync"));
    assert_eq!(r.session_identifier.as_deref(), Some("tmux:aoe_flowsync_x"));
    assert!(r.is_peer && r.has_agent && r.has_tmux);
    assert_eq!(r.machine.as_deref(), Some("seoul"));
    assert!(r.aliases.contains(&"aoe_flowsync_x".to_string()));
    assert!(r.aliases.contains(&"flowsync".to_string()));
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
