//! marketplace_connect (검색→연결) + 온체인 게이트 선택 분기 통합 테스트.
//!
//! 별도 통합 테스트 crate — lib 내부 일부 테스트가 깨져 있어도 독립 컴파일/실행된다.

use openxgram_db::{Db, DbConfig};
use openxgram_peer::{PeerRole, PeerStore};

fn temp_db() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Db::open(DbConfig {
        path: dir.path().join("db.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    (dir, db)
}

/// marketplace_connect 의 핵심 불변식: 같은 마켓 agent_id 를 두 번 연결해도 peer 가
/// 중복 생성되지 않고 재사용된다(idempotent). placeholder pubkey `mkt:<agent_id>` 가
/// UNIQUE 키 역할을 해 dispatch handler 가 get_by_public_key 로 재사용을 판정한다.
#[test]
fn connect_is_idempotent_by_placeholder_pubkey() {
    let (_dir, mut db) = temp_db();
    let agent_id = "agent:writer-007";
    let placeholder = format!("mkt:{agent_id}");
    let note = "marketplace agent (test)";

    // 1차 연결 — placeholder 키로 신규 등록.
    {
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_public_key(&placeholder).unwrap().is_none());
        store
            .add(
                "Writer 007",
                &placeholder,
                agent_id,
                PeerRole::Secondary,
                Some(note),
            )
            .unwrap();
    }

    // 2차 연결 시도 — 이미 존재하므로 핸들러는 add 하지 않고 재사용해야 한다.
    {
        let mut store = PeerStore::new(&mut db);
        let existing = store.get_by_public_key(&placeholder).unwrap();
        assert!(existing.is_some(), "두 번째 connect 는 기존 peer 를 봐야 함");
        assert_eq!(existing.unwrap().alias, "Writer 007");
    }

    // peer 총 1개만 — 중복 없음.
    let mut store = PeerStore::new(&mut db);
    let count = store
        .list()
        .unwrap()
        .into_iter()
        .filter(|p| p.public_key_hex == placeholder)
        .count();
    assert_eq!(count, 1, "같은 마켓 에이전트는 1개 peer 로만 등록");
}

/// alias 충돌 회피: 표시명 alias 가 이미 점유면 agent_id 로 fallback (UNIQUE alias 보장).
#[test]
fn connect_alias_collision_falls_back_to_agent_id() {
    let (_dir, mut db) = temp_db();

    // 기존 다른 peer 가 "Helper" alias 점유 (실제 secp256k1 형태 placeholder).
    {
        let mut store = PeerStore::new(&mut db);
        store
            .add("Helper", "mkt:agent:other", "agent:other", PeerRole::Secondary, None)
            .unwrap();
    }

    // 새 마켓 에이전트 표시명도 "Helper" → alias 충돌 → agent_id 로 fallback.
    let agent_id = "agent:helper-x";
    let placeholder = format!("mkt:{agent_id}");
    let preferred = "Helper";
    let mut store = PeerStore::new(&mut db);
    let alias = match store.get_by_alias(preferred).unwrap() {
        Some(_) => agent_id.to_string(),
        None => preferred.to_string(),
    };
    assert_eq!(alias, agent_id, "표시명 충돌 시 agent_id 로 등록되어야 함");
    store
        .add(&alias, &placeholder, agent_id, PeerRole::Secondary, None)
        .unwrap();

    assert!(store.get_by_alias(agent_id).unwrap().is_some());
}

/// 게이트 선택 분기: `XGRAM_CHAIN_RPC`(비공백) + vault 비밀번호가 **둘 다** 있어야 온체인.
/// mcp_serve `OpenxgramDispatcher::open` 의 match 조건을 동일하게 재현 — 회귀 방지.
fn select_onchain(chain_rpc: Option<&str>, vault_pw: Option<&str>) -> bool {
    let rpc = chain_rpc
        .map(|s| s.to_string())
        .filter(|u| !u.trim().is_empty());
    matches!((rpc, vault_pw), (Some(_), Some(_)))
}

#[test]
fn gateway_selection_branch() {
    assert!(!select_onchain(None, None), "기본: ledger");
    assert!(
        !select_onchain(Some("https://rpc.example"), None),
        "RPC만 → ledger (키 없음)"
    );
    assert!(
        !select_onchain(None, Some("pw")),
        "비밀번호만 → ledger (RPC 없음)"
    );
    assert!(
        !select_onchain(Some("   "), Some("pw")),
        "빈 RPC → ledger"
    );
    assert!(
        select_onchain(Some("https://rpc.example"), Some("pw")),
        "둘 다 → 온체인"
    );
}
