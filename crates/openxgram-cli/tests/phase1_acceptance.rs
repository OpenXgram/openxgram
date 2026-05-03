//! Phase 1 MVP 합격 시나리오 통합 테스트 — PRD §20 의 cli·코드 레이어 검증.
//!
//! 시스템 e2e (다른 머신·실 Tailscale·실 Discord) 는 후속. 본 파일은
//! 단일 머신 안에서 검증 가능한 합격 기준만.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::reset::{run_reset, ResetOpts};
use openxgram_cli::session::{run_session, SessionAction};
use openxgram_cli::uninstall::{run_uninstall, UninstallOpts};
use openxgram_manifest::MachineRole;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";
const DELETE_CONFIRM: &str = "DELETE OPENXGRAM";
const RESET_CONFIRM: &str = "RESET OPENXGRAM";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "phase1-acceptance".into(),
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

fn no_backup(data_dir: PathBuf) -> UninstallOpts {
    UninstallOpts {
        data_dir,
        cold_backup_to: None,
        no_backup: true,
        confirm: Some(DELETE_CONFIRM.into()),
        dry_run: false,
    }
}

/// PRD §20 H: install → uninstall → 흔적 0건 (data_dir 사라짐).
#[test]
fn scenario_h_uninstall_leaves_no_data_dir() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    assert!(data_dir.join("install-manifest.json").exists());
    assert!(data_dir.join("db.sqlite").exists());
    assert!(data_dir.join("keystore").join("master.json").exists());

    run_uninstall(&no_backup(data_dir.clone())).unwrap();

    assert!(!data_dir.exists(), "uninstall 후 흔적 0건");
}

/// PRD §20 I: install → reset --hard → 즉시 재초기화 가능.
#[test]
fn scenario_i_reset_hard_then_reinstall() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_reset(&ResetOpts {
        data_dir: data_dir.clone(),
        hard: true,
        confirm: Some(RESET_CONFIRM.into()),
        dry_run: false,
    })
    .unwrap();
    assert!(!data_dir.exists());

    // 재설치 즉시 가능
    run_init(&init_opts(data_dir.clone())).unwrap();
    assert!(data_dir.join("install-manifest.json").exists());
}

/// PRD §20 J: 마스터 반복 워크플로우 — 3 회 install-uninstall round-trip.
#[test]
fn scenario_j_three_round_trips() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");

    for i in 0..3 {
        run_init(&init_opts(data_dir.clone()))
            .unwrap_or_else(|e| panic!("iter {i} init failed: {e:#}"));
        assert!(data_dir.exists());
        run_uninstall(&no_backup(data_dir.clone()))
            .unwrap_or_else(|e| panic!("iter {i} uninstall failed: {e:#}"));
        assert!(!data_dir.exists(), "iter {i} 후 data_dir 사라짐");
    }
}

/// PRD §20 F: ChatGPT 토론 → 사이드카 import → Claude Code attach (마스터 핵심 요구).
/// 머신 A 에서 session 생성·메시지 → export → 머신 B 에서 import --verify.
#[test]
fn scenario_f_export_import_with_verify() {
    set_env();
    use openxgram_core::paths::db_path;
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::{DummyEmbedder, MessageStore, SessionStore};

    // 머신 A
    let tmp_a = tempdir().unwrap();
    let dir_a = tmp_a.path().join("openxgram");
    let mut o_a = init_opts(dir_a.clone());
    o_a.alias = "machine-a".into();
    run_init(&o_a).unwrap();

    run_session(
        &dir_a,
        SessionAction::New {
            title: "scenario-f".into(),
        },
    )
    .unwrap();
    let sid_a = {
        let mut db = Db::open(DbConfig {
            path: db_path(&dir_a),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        SessionStore::new(&mut db).list().unwrap()[0].id.clone()
    };

    for body in &["메시지 A", "메시지 B", "메시지 C"] {
        run_session(
            &dir_a,
            SessionAction::Message {
                session_id: sid_a.clone(),
                sender: "0xMaster".into(),
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
            session_id: sid_a,
            out: Some(pkg_path.clone()),
        },
    )
    .unwrap();

    // 머신 B
    let tmp_b = tempdir().unwrap();
    let dir_b = tmp_b.path().join("openxgram");
    let mut o_b = init_opts(dir_b.clone());
    o_b.alias = "machine-b".into();
    run_init(&o_b).unwrap();

    run_session(
        &dir_b,
        SessionAction::Import {
            input: Some(pkg_path),
            verify: true,
        },
    )
    .unwrap();

    // 검증 — 머신 B 에 메시지 3 + episode 1
    let mut db_b = Db::open(DbConfig {
        path: db_path(&dir_b),
        ..Default::default()
    })
    .unwrap();
    db_b.migrate().unwrap();
    let sids_b: Vec<String> = SessionStore::new(&mut db_b)
        .list()
        .unwrap()
        .iter()
        .map(|s| s.id.clone())
        .collect();
    assert_eq!(sids_b.len(), 1);
    let embedder = DummyEmbedder;
    let messages = MessageStore::new(&mut db_b, &embedder)
        .list_for_session(&sids_b[0])
        .unwrap();
    assert_eq!(messages.len(), 3);
    let bodies: Vec<&str> = messages.iter().map(|m| m.body.as_str()).collect();
    assert!(bodies.contains(&"메시지 A"));
    assert!(bodies.contains(&"메시지 B"));
    assert!(bodies.contains(&"메시지 C"));
}

/// 합격 시나리오 묶음: cold backup 라운드트립 (uninstall → restore → 데이터 복원).
#[test]
fn scenario_cold_backup_full_round_trip() {
    use openxgram_cli::backup::restore_cold_backup;
    use openxgram_core::paths::manifest_path;

    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    let backup_path = tmp.path().join("snap.enc");
    let restore_dir = tmp.path().join("restored");

    run_init(&init_opts(data_dir.clone())).unwrap();
    let manifest_before = std::fs::read(manifest_path(&data_dir)).unwrap();

    run_uninstall(&UninstallOpts {
        data_dir: data_dir.clone(),
        cold_backup_to: Some(backup_path.clone()),
        no_backup: false,
        confirm: None,
        dry_run: false,
    })
    .unwrap();
    assert!(!data_dir.exists());

    restore_cold_backup(&backup_path, &restore_dir, TEST_PASSWORD).unwrap();
    let manifest_after = std::fs::read(manifest_path(&restore_dir)).unwrap();
    assert_eq!(manifest_before, manifest_after);
}
