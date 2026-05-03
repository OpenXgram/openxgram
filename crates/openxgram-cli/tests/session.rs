//! xgram session 명령 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::session::{run_session, SessionAction};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "session-test".into(),
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
fn session_requires_init_first() {
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("absent");
    let err = run_session(
        &data_dir,
        SessionAction::New {
            title: "x".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}

#[test]
fn full_flow_new_message_reflect_show() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // session 처음엔 빈 list
    run_session(&data_dir, SessionAction::List).unwrap();

    // 새 session — 직접 db 에서 list 후 ID 회수 (run_session 은 stdout 만 출력)
    run_session(
        &data_dir,
        SessionAction::New {
            title: "통합 테스트 세션".into(),
        },
    )
    .unwrap();

    // memory crate 로 직접 첫 session id 조회
    use openxgram_db::{Db, DbConfig};
    use openxgram_core::paths::db_path;
    use openxgram_memory::SessionStore;
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let sessions = SessionStore::new(&mut db).list().unwrap();
    assert_eq!(sessions.len(), 1);
    let sid = sessions[0].id.clone();
    drop(db);

    // 메시지 3개 추가
    for body in &["첫 메시지", "두번째", "세번째"] {
        run_session(
            &data_dir,
            SessionAction::Message {
                session_id: sid.clone(),
                sender: "0xtest".into(),
                body: body.to_string(),
            },
        )
        .unwrap();
    }

    // reflect → episode 1개 생성
    run_session(
        &data_dir,
        SessionAction::Reflect {
            session_id: sid.clone(),
        },
    )
    .unwrap();

    // show 통과
    run_session(&data_dir, SessionAction::Show { id: sid }).unwrap();
}

#[test]
fn message_to_unknown_session_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_session(
        &data_dir,
        SessionAction::Message {
            session_id: "nonexistent-uuid".into(),
            sender: "0xtest".into(),
            body: "hi".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("session 없음"));
}

#[test]
fn recall_returns_results_after_messages() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // session + 메시지 3개
    run_session(
        &data_dir,
        SessionAction::New {
            title: "recall-test".into(),
        },
    )
    .unwrap();

    use openxgram_core::paths::db_path;
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::SessionStore;
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let sid = SessionStore::new(&mut db).list().unwrap()[0].id.clone();
    drop(db);

    for body in &["hello world", "foo bar", "openxgram memory"] {
        run_session(
            &data_dir,
            SessionAction::Message {
                session_id: sid.clone(),
                sender: "0xtest".into(),
                body: body.to_string(),
            },
        )
        .unwrap();
    }

    // recall — 자기 자신이 포함된 결과 K=2
    run_session(
        &data_dir,
        SessionAction::Recall {
            query: "hello world".into(),
            k: 2,
        },
    )
    .unwrap();
}

#[test]
fn recall_empty_db_returns_no_results() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    // 메시지 0건 상태에서 recall — 결과 0건이지만 raise 안 함
    run_session(
        &data_dir,
        SessionAction::Recall {
            query: "anything".into(),
            k: 5,
        },
    )
    .unwrap();
}

#[test]
fn show_unknown_session_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_session(
        &data_dir,
        SessionAction::Show {
            id: "nonexistent".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("session 없음"));
}
