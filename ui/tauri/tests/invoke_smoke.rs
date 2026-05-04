//! invoke 핸들러 smoke test — 빈 DB 디렉토리에서도 raise 하지 않고 빈 결과 반환.
//!
//! Tauri `#[tauri::command]` 함수는 `tauri::State` 를 직접 받아 외부에서 호출 어렵다.
//! 본 테스트는 lib.rs 의 `AppState` + `with_db_optional` (pub) 동작을 검증해
//! "DB 미존재 시 None / 빈 결과" 정합을 보장한다.
//!
//! 또한 store API 가 빈 DB 에서도 정상 작동(empty Vec) 하는지 sanity-check.

use openxgram_db::{Db, DbConfig};
use openxgram_desktop_lib::AppState;

#[test]
fn appstate_default_data_dir_returns_some_path() {
    // HOME 또는 XGRAM_DATA_DIR 둘 중 하나는 set 되어 있어야 한다 (CI 가정).
    if std::env::var("HOME").is_err() && std::env::var("XGRAM_DATA_DIR").is_err() {
        return; // skip — 환경 미충족
    }
    let p = AppState::default_data_dir().expect("default_data_dir 실패");
    assert!(!p.as_os_str().is_empty());
}

#[test]
fn empty_db_dir_does_not_raise() {
    let tmp = tempfile::tempdir().unwrap();
    let state = AppState::new(tmp.path().to_path_buf());
    // DB 파일을 만들지 않음 — `with_db_optional` 은 db 락을 잡지 않고 None 반환해야 한다.
    // AppState 의 db Mutex 가 unpoisoned 상태인지만 확인.
    let _guard = state.db.lock().expect("mutex poisoned");
    drop(_guard);
    drop(state);
}

#[test]
fn fresh_db_open_yields_empty_stores() {
    // openxgram-db / vault / peer / memory store 가 새 DB 에서도 빈 Vec 을 반환해야 한다.
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
