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
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
#[serial_test::file_serial]
fn dispatcher_lists_db_and_vault_tools_when_password_present() {
    set_env(); // sets XGRAM_KEYSTORE_PASSWORD
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    let names: Vec<String> = dispatcher.tools().iter().map(|t| t.name.clone()).collect();
    assert!(names.contains(&"list_sessions".to_string()));
    assert!(names.contains(&"recall_messages".to_string()));
    assert!(names.contains(&"list_memories_by_kind".to_string()));
    assert!(names.contains(&"vault_list".to_string()));
    assert!(names.contains(&"vault_get".to_string()));
    assert!(names.contains(&"vault_set".to_string()));
}

#[test]
#[serial_test::file_serial]
fn dispatcher_omits_vault_tools_when_no_password() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // open 후 환경에서 password 제거 — open 시점에 이미 캐시됨
    // 따라서 password 없는 상태로 새로 open
    unsafe {
        std::env::remove_var("XGRAM_KEYSTORE_PASSWORD");
    }
    let dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    let names: Vec<String> = dispatcher.tools().iter().map(|t| t.name.clone()).collect();
    assert_eq!(names.len(), 3, "vault 미노출 → db tools 3개만");
    assert!(!names.iter().any(|n| n.starts_with("vault_")));
    set_env(); // 다른 테스트에 영향 차단
}

#[test]
#[serial_test::file_serial]
fn dispatcher_vault_set_get_round_trip() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let mut dispatcher = OpenxgramDispatcher::open(&data_dir).unwrap();
    dispatcher
        .dispatch(
            "vault_set",
            &json!({"key": "discord/bot", "value": "TOKEN", "tags": ["discord"]}),
        )
        .unwrap();
    let got = dispatcher
        .dispatch("vault_get", &json!({"key": "discord/bot"}))
        .unwrap();
    assert_eq!(got["value"], "TOKEN");

    let listed = dispatcher.dispatch("vault_list", &json!({})).unwrap();
    assert_eq!(listed["count"], 1);
}

#[test]
#[serial_test::file_serial]
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
#[serial_test::file_serial]
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
#[serial_test::file_serial]
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
#[serial_test::file_serial]
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
#[serial_test::file_serial]
fn open_without_init_raises() {
    let tmp = tempdir().unwrap();
    match OpenxgramDispatcher::open(&tmp.path().join("absent")) {
        Ok(_) => panic!("expected error for missing data_dir"),
        Err(e) => assert!(format!("{e:#}").contains("미존재")),
    }
}
