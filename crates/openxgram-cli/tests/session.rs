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
fn export_roundtrip_via_json() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    use openxgram_core::paths::db_path;
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::{export_session, SessionStore, TextPackage};

    // session + 메시지 2건
    run_session(
        &data_dir,
        SessionAction::New {
            title: "export-test".into(),
        },
    )
    .unwrap();
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let sid = SessionStore::new(&mut db).list().unwrap()[0].id.clone();
    drop(db);

    for body in &["첫 메시지", "두번째"] {
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
    run_session(
        &data_dir,
        SessionAction::Reflect {
            session_id: sid.clone(),
        },
    )
    .unwrap();

    // export → 파일 저장
    let out_path = tmp.path().join("export.json");
    run_session(
        &data_dir,
        SessionAction::Export {
            session_id: sid.clone(),
            out: Some(out_path.clone()),
        },
    )
    .unwrap();

    // JSON parse round-trip
    let json = std::fs::read_to_string(&out_path).unwrap();
    let pkg = TextPackage::from_json(&json).unwrap();
    assert_eq!(pkg.format, "text-package-v1");
    assert_eq!(pkg.session.id, sid);
    assert_eq!(pkg.session.title, "export-test");
    assert_eq!(pkg.messages.len(), 2);
    assert_eq!(pkg.messages[0].body, "첫 메시지");
    assert_eq!(pkg.episodes.len(), 1);
    assert_eq!(pkg.episodes[0].message_count, 2);

    // 같은 결과를 export_session 직접 호출로도 검증
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let pkg2 = export_session(&mut db, &sid, "test-host").unwrap();
    assert_eq!(pkg2.messages.len(), pkg.messages.len());
}

#[test]
fn export_then_import_into_fresh_install_preserves_messages() {
    set_env();
    use openxgram_core::paths::db_path;
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::SessionStore;

    // 머신 A — export
    let tmp_a = tempdir().unwrap();
    let dir_a = tmp_a.path().join("openxgram");
    let mut o_a = init_opts(dir_a.clone());
    o_a.alias = "machine-a".into();
    run_init(&o_a).unwrap();

    run_session(
        &dir_a,
        SessionAction::New {
            title: "transfer-test".into(),
        },
    )
    .unwrap();
    let mut db_a = Db::open(DbConfig {
        path: db_path(&dir_a),
        ..Default::default()
    })
    .unwrap();
    db_a.migrate().unwrap();
    let sid_a = SessionStore::new(&mut db_a).list().unwrap()[0].id.clone();
    drop(db_a);

    for body in &["alpha", "beta", "gamma"] {
        run_session(
            &dir_a,
            SessionAction::Message {
                session_id: sid_a.clone(),
                sender: "0xtest".into(),
                body: body.to_string(),
            },
        )
        .unwrap();
    }
    run_session(
        &dir_a,
        SessionAction::Reflect {
            session_id: sid_a.clone(),
        },
    )
    .unwrap();

    let pkg_path = tmp_a.path().join("pkg.json");
    run_session(
        &dir_a,
        SessionAction::Export {
            session_id: sid_a.clone(),
            out: Some(pkg_path.clone()),
        },
    )
    .unwrap();

    // 머신 B — import (별도 data_dir)
    let tmp_b = tempdir().unwrap();
    let dir_b = tmp_b.path().join("openxgram");
    let mut o_b = init_opts(dir_b.clone());
    o_b.alias = "machine-b".into();
    run_init(&o_b).unwrap();

    run_session(
        &dir_b,
        SessionAction::Import {
            input: Some(pkg_path),
        },
    )
    .unwrap();

    // 검증: 머신 B 에 새 session 이 1개 + 메시지 3개
    let mut db_b = Db::open(DbConfig {
        path: db_path(&dir_b),
        ..Default::default()
    })
    .unwrap();
    db_b.migrate().unwrap();
    let sessions_b = SessionStore::new(&mut db_b).list().unwrap();
    assert_eq!(sessions_b.len(), 1);
    assert_eq!(sessions_b[0].title, "transfer-test");

    use openxgram_memory::{DummyEmbedder, EpisodeStore, MessageStore};
    let sid_b = sessions_b[0].id.clone();
    let embedder = DummyEmbedder;
    let messages_b = MessageStore::new(&mut db_b, &embedder)
        .list_for_session(&sid_b)
        .unwrap();
    assert_eq!(messages_b.len(), 3);
    let bodies: Vec<&str> = messages_b.iter().map(|m| m.body.as_str()).collect();
    assert!(bodies.contains(&"alpha"));
    assert!(bodies.contains(&"beta"));
    assert!(bodies.contains(&"gamma"));

    let episodes_b = EpisodeStore::new(&mut db_b)
        .list_for_session(&sid_b)
        .unwrap();
    assert_eq!(episodes_b.len(), 1);
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
