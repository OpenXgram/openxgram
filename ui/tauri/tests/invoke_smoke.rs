//! invoke 핸들러 smoke test — 빈 DB 디렉토리에서도 raise 하지 않고 빈 결과 반환.
//!
//! Tauri `#[tauri::command]` 함수는 `tauri::State` 를 직접 받아 외부에서 호출 어렵다.
//! 본 테스트는 lib.rs 의 `AppState` + `with_db_optional` 동작을 검증해
//! "DB 미존재 시 None / 빈 결과" 정합을 보장한다.
//!
//! 또한 store API + 신규 schedule/chain store 가 빈 DB 에서도 정상 작동
//! (empty Vec) 하는지 sanity-check 한다.

use openxgram_db::{Db, DbConfig};
use openxgram_desktop_lib::state::is_data_initialized;
use openxgram_desktop_lib::AppState;

#[test]
fn appstate_default_data_dir_returns_some_path() {
    if std::env::var("HOME").is_err() && std::env::var("XGRAM_DATA_DIR").is_err() {
        return;
    }
    let p = AppState::default_data_dir().expect("default_data_dir 실패");
    assert!(!p.as_os_str().is_empty());
}

#[test]
fn empty_db_dir_does_not_raise() {
    let tmp = tempfile::tempdir().unwrap();
    let state = AppState::new(tmp.path().to_path_buf());
    let _guard = state.db.lock().expect("mutex poisoned");
    drop(_guard);
    drop(state);
}

#[test]
fn is_data_initialized_returns_false_when_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let state = AppState::new(tmp.path().to_path_buf());
    assert!(!is_data_initialized(&state));
}

#[test]
fn fresh_db_open_yields_empty_stores() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("openxgram.db");
    let mut db = Db::open(DbConfig {
        path: path.clone(),
        ..Default::default()
    })
    .expect("Db open 실패");
    db.migrate().expect("DB migrate 실패");

    {
        let mut store = openxgram_vault::VaultStore::new(&mut db);
        let pending = store.list_pending().expect("list_pending");
        assert!(pending.is_empty(), "fresh DB pending 비어있어야 함");
        let acls = store.list_acl().expect("list_acl");
        assert!(acls.is_empty(), "fresh DB acl 비어있어야 함");
    }
    {
        let mut store = openxgram_peer::PeerStore::new(&mut db);
        let peers = store.list().expect("peer list");
        assert!(peers.is_empty(), "fresh DB peer 비어있어야 함");
    }
    {
        let mut store = openxgram_memory::MemoryStore::new(&mut db);
        let mems = store
            .list_by_kind(openxgram_memory::MemoryKind::Fact)
            .expect("memory list");
        assert!(mems.is_empty(), "fresh DB memory 비어있어야 함");
    }
}

/// 신규 GUI 핸들러가 의존하는 orchestration store 도 빈 DB 에서 raise 하지 않아야 함.
#[test]
fn fresh_db_orchestration_stores_are_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("openxgram.db");
    let mut db = Db::open(DbConfig {
        path: path.clone(),
        ..Default::default()
    })
    .expect("Db open 실패");
    db.migrate().expect("DB migrate 실패");

    let sched = openxgram_orchestration::ScheduledStore::new(db.conn());
    let rows = sched.list(None).expect("schedule list");
    assert!(rows.is_empty(), "fresh DB scheduled 비어있어야 함");

    let chain = openxgram_orchestration::ChainStore::new(db.conn());
    let chains = chain.list().expect("chain list");
    assert!(chains.is_empty(), "fresh DB chain 비어있어야 함");
}

/// notify_status — 저장된 toml 이 없을 때 모두 false 반환해야 한다.
#[test]
fn notify_config_load_empty_yields_no_adapters() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg =
        openxgram_cli::notify_setup::NotifyConfig::load(Some(tmp.path())).expect("load empty");
    assert!(cfg.telegram.is_none());
    assert!(cfg.discord.is_none());
}
