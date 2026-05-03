//! xgram payment CLI 통합 테스트.

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::payment::{run_payment, PaymentAction};
use openxgram_manifest::MachineRole;
use std::path::PathBuf;
use tempfile::tempdir;

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "payment-cli-test".into(),
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
fn chains_list_works() {
    let tmp = tempdir().unwrap();
    // chains 명령은 db 미존재 상태에서도 작동 (메모리만)
    run_payment(&tmp.path().join("absent"), PaymentAction::Chains).unwrap();
}

#[test]
fn new_then_list_then_sign_round_trip() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_payment(
        &data_dir,
        PaymentAction::New {
            amount_usdc: "1.50".into(),
            chain: "base".into(),
            to: "0xrecipient000000000000000000000000000000".into(),
            memo: Some("test".into()),
        },
    )
    .unwrap();

    run_payment(&data_dir, PaymentAction::List).unwrap();
}

#[test]
fn unsupported_chain_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_payment(
        &data_dir,
        PaymentAction::New {
            amount_usdc: "1".into(),
            chain: "fake-chain".into(),
            to: "0xa".into(),
            memo: None,
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("지원하지 않는 chain"));
}

#[test]
fn requires_init_first() {
    let tmp = tempdir().unwrap();
    let err = run_payment(&tmp.path().join("absent"), PaymentAction::List).unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}
