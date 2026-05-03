//! xgram dump CLI 통합 테스트.

use openxgram_cli::dump::run_dump;
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_manifest::MachineRole;
use std::path::PathBuf;
use tempfile::tempdir;

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "dump-cli-test".into(),
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
fn dump_all_kinds_after_init_succeed() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    for kind in [
        "sessions",
        "episodes",
        "memories",
        "patterns",
        "traits",
        "vault",
        "acl",
        "pending",
        "peers",
        "payments",
        "mcp-tokens",
    ] {
        run_dump(&data_dir, kind).unwrap();
    }
}

#[test]
#[serial_test::file_serial]
fn dump_unsupported_kind_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();
    let err = run_dump(&data_dir, "nonexistent").unwrap_err();
    assert!(format!("{err:#}").contains("지원하지 않는 kind"));
}

#[test]
#[serial_test::file_serial]
fn dump_requires_init_first() {
    let tmp = tempdir().unwrap();
    let err = run_dump(&tmp.path().join("absent"), "sessions").unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}
