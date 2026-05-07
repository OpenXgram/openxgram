//! xgram peer CLI 통합 테스트.

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::peer::{run_peer, PeerAction};
use openxgram_manifest::MachineRole;
use openxgram_peer::PeerRole;
use std::path::PathBuf;
use tempfile::tempdir;

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "peer-cli-test".into(),
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
fn add_list_show_touch_delete_round_trip() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_peer(
        &data_dir,
        PeerAction::Add {
            alias: "mac-mini".into(),
            // secp256k1 generator G (compressed) — 유효한 sec1 인코딩.
            // PR #138 이후 CLI 의 peer add 가 pubkey 를 sec1 로 파싱해 eth_address 를 도출하므로 실 점이 필요.
            public_key_hex: "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .into(),
            address: "http://192.168.1.10:7300".into(),
            role: PeerRole::Secondary,
            notes: Some("home server".into()),
        },
    )
    .unwrap();

    run_peer(&data_dir, PeerAction::List).unwrap();
    run_peer(
        &data_dir,
        PeerAction::Show {
            alias: "mac-mini".into(),
        },
    )
    .unwrap();
    run_peer(
        &data_dir,
        PeerAction::Touch {
            alias: "mac-mini".into(),
        },
    )
    .unwrap();
    run_peer(
        &data_dir,
        PeerAction::Delete {
            alias: "mac-mini".into(),
        },
    )
    .unwrap();
}

#[test]
fn show_unknown_peer_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_peer(
        &data_dir,
        PeerAction::Show {
            alias: "nope".into(),
        },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("nope"));
}

#[test]
fn requires_init_first() {
    let tmp = tempdir().unwrap();
    let err = run_peer(&tmp.path().join("absent"), PeerAction::List).unwrap_err();
    assert!(format!("{err:#}").contains("미존재"));
}
