//! xgram memory CLI 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::memory::{run_memory, MemoryAction};
use openxgram_manifest::MachineRole;
use openxgram_memory::MemoryKind;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "memory-cli-test".into(),
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
fn add_then_list() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_memory(
        &data_dir,
        MemoryAction::Add {
            kind: MemoryKind::Fact,
            content: "Phase 1 마감 5월".into(),
            session_id: None,
        },
    )
    .unwrap();
    run_memory(
        &data_dir,
        MemoryAction::Add {
            kind: MemoryKind::Decision,
            content: "ChaCha20 통일".into(),
            session_id: None,
        },
    )
    .unwrap();

    // list each kind — raise 없으면 OK (빈 list 도 정상)
    run_memory(
        &data_dir,
        MemoryAction::List {
            kind: MemoryKind::Fact,
        },
    )
    .unwrap();
    run_memory(
        &data_dir,
        MemoryAction::List {
            kind: MemoryKind::Reference,
        },
    )
    .unwrap();
}

#[test]
fn pin_unknown_id_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_memory(
        &data_dir,
        MemoryAction::Pin {
            id: "nonexistent".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("affected rows"));
}

#[test]
fn requires_init_first() {
    let tmp = tempdir().unwrap();
    let err = run_memory(
        &tmp.path().join("absent"),
        MemoryAction::List {
            kind: MemoryKind::Fact,
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}
