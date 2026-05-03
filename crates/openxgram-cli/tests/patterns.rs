//! xgram patterns CLI 통합 테스트.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::patterns::{run_patterns, PatternsAction};
use openxgram_manifest::MachineRole;
use openxgram_memory::Classification;
use tempfile::tempdir;

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "patterns-cli-test".into(),
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
        std::env::remove_var("XGRAM_SEED");
    }
}

#[test]
fn observe_then_list_classifies_by_frequency() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // 1회 → NEW
    run_patterns(
        &data_dir,
        PatternsAction::Observe {
            text: "wake-up greeting".into(),
        },
    )
    .unwrap();
    run_patterns(
        &data_dir,
        PatternsAction::List {
            classification: Classification::New,
        },
    )
    .unwrap();

    // 2회 더 observe → RECURRING (총 3회)
    for _ in 0..2 {
        run_patterns(
            &data_dir,
            PatternsAction::Observe {
                text: "wake-up greeting".into(),
            },
        )
        .unwrap();
    }
    run_patterns(
        &data_dir,
        PatternsAction::List {
            classification: Classification::Recurring,
        },
    )
    .unwrap();

    // 2회 더 observe → ROUTINE (총 5회)
    for _ in 0..2 {
        run_patterns(
            &data_dir,
            PatternsAction::Observe {
                text: "wake-up greeting".into(),
            },
        )
        .unwrap();
    }
    run_patterns(
        &data_dir,
        PatternsAction::List {
            classification: Classification::Routine,
        },
    )
    .unwrap();
}

#[test]
fn list_empty_classification_ok() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_patterns(
        &data_dir,
        PatternsAction::List {
            classification: Classification::Routine,
        },
    )
    .unwrap();
}

#[test]
fn requires_init_first() {
    let tmp = tempdir().unwrap();
    let err = run_patterns(
        &tmp.path().join("absent"),
        PatternsAction::List {
            classification: Classification::New,
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}
