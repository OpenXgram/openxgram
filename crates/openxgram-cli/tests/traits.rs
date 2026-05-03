//! xgram traits CLI 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::traits::{run_traits, TraitsAction};
use openxgram_manifest::MachineRole;
use openxgram_memory::TraitSource;
use tempfile::tempdir;

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "traits-cli-test".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", "test-password-12345");
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
#[serial_test::file_serial]
fn set_then_list_and_get() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_traits(
        &data_dir,
        TraitsAction::Set {
            name: "tone".into(),
            value: "concise".into(),
            source: TraitSource::Manual,
            refs: vec!["mem-1".into()],
        },
    )
    .unwrap();

    run_traits(&data_dir, TraitsAction::List).unwrap();
    run_traits(
        &data_dir,
        TraitsAction::Get {
            name: "tone".into(),
        },
    )
    .unwrap();
}

#[test]
#[serial_test::file_serial]
fn set_overwrites_same_name() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_traits(
        &data_dir,
        TraitsAction::Set {
            name: "lang".into(),
            value: "ko".into(),
            source: TraitSource::Manual,
            refs: vec![],
        },
    )
    .unwrap();
    // 같은 name 으로 다시 set — 갱신
    run_traits(
        &data_dir,
        TraitsAction::Set {
            name: "lang".into(),
            value: "en".into(),
            source: TraitSource::Manual,
            refs: vec![],
        },
    )
    .unwrap();
}

#[test]
#[serial_test::file_serial]
fn get_unknown_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_traits(
        &data_dir,
        TraitsAction::Get {
            name: "nonexistent".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("nonexistent"));
}

#[test]
#[serial_test::file_serial]
fn derive_runs_without_patterns() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    // ROUTINE 없음 → 0 도출 OK
    run_traits(&data_dir, TraitsAction::Derive).unwrap();
}

#[test]
#[serial_test::file_serial]
fn requires_init_first() {
    let tmp = tempdir().unwrap();
    let err = run_traits(&tmp.path().join("absent"), TraitsAction::List).unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}
