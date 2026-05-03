//! mcp_serve dispatcher 통합 테스트 — OpenxgramDispatcher 가 db/memory tools
//! 를 노출·실행하는지. stdio loop 자체는 e2e 환경에서 검증.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::mcp_serve::OpenxgramDispatcher;
use openxgram_manifest::MachineRole;
use openxgram_mcp::ToolDispatcher;
use serde_json::json;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "mcp-test".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", TEST_PASSWORD);
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
fn dispatcher_lists_three_tools() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    let tools = dispatcher.tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"list_sessions"));
    assert!(names.contains(&"recall_messages"));
    assert!(names.contains(&"list_memories_by_kind"));
}

#[test]
fn dispatcher_list_sessions_returns_empty_then_one() {
    set_env();
    use openxgram_core::paths::db_path;
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::SessionStore;

    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    let result = dispatcher.dispatch("list_sessions", &json!({})).unwrap();
    assert_eq!(result["count"], 0);

    // session 1개 추가
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    SessionStore::new(&mut db).create("test", "host").unwrap();
    drop(db);

    // dispatcher 새로 (db 변경 후 재open 권장)
    let mut dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    let result = dispatcher.dispatch("list_sessions", &json!({})).unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["sessions"][0]["title"], "test");
}

#[test]
fn dispatcher_list_memories_validates_kind() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();

    // 빈 fact list
    let result = dispatcher
        .dispatch("list_memories_by_kind", &json!({"kind": "fact"}))
        .unwrap();
    assert_eq!(result["count"], 0);

    // invalid kind → InvalidParams
    let err = dispatcher
        .dispatch("list_memories_by_kind", &json!({"kind": "bogus"}))
        .unwrap_err();
    assert_eq!(err.code, openxgram_mcp::ERR_INVALID_PARAMS);
}

#[test]
fn dispatcher_recall_messages_validates_query() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    let err = dispatcher
        .dispatch("recall_messages", &json!({}))
        .unwrap_err();
    assert_eq!(err.code, openxgram_mcp::ERR_INVALID_PARAMS);

    // 빈 결과
    let result = dispatcher
        .dispatch("recall_messages", &json!({"query": "anything", "k": 3}))
        .unwrap();
    assert_eq!(result["count"], 0);
}

#[test]
fn dispatcher_unknown_tool_returns_method_not_found() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    let err = dispatcher.dispatch("nonexistent", &json!({})).unwrap_err();
    assert_eq!(err.code, openxgram_mcp::ERR_METHOD_NOT_FOUND);
}

#[test]
fn open_without_init_raises() {
    let tmp = tempdir().unwrap();
    match OpenxgramDispatcher::open(&tmp.path().join("absent")) {
        Ok(_) => panic!("expected error for missing data_dir"),
        Err(e) => assert!(format!("{e:#}").contains("미존재")),
    }
}
