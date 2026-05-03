//! peer-send 통합 테스트 — peer 등록 → 로컬 transport 띄우기 → send → received 확인.

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::peer::{run_peer, PeerAction};
use openxgram_cli::peer_send::run_peer_send;
use openxgram_manifest::MachineRole;
use openxgram_peer::PeerRole;
use openxgram_transport::spawn_server;
use std::path::PathBuf;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "peer-send-test".into(),
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

#[tokio::test]
#[serial_test::file_serial]
async fn send_to_peer_with_local_server() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // 로컬 서버 띄우기 (port=0)
    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let address = format!("http://{}", server.bound_addr);

    // peer 등록
    run_peer(
        &data_dir,
        PeerAction::Add {
            alias: "local-test".into(),
            public_key_hex: "ab".repeat(33),
            address,
            role: PeerRole::Worker,
            notes: None,
        },
    )
    .unwrap();

    // 메시지 전송
    run_peer_send(&data_dir, "local-test", None, "hello peer", TEST_PASSWORD)
        .await
        .unwrap();

    // 서버 측 received 큐 확인 — 1건 도착
    let received = server.received();
    assert_eq!(received.len(), 1);
    let env = &received[0];
    let payload_bytes = hex::decode(&env.payload_hex).unwrap();
    assert_eq!(std::str::from_utf8(&payload_bytes).unwrap(), "hello peer");

    server.shutdown();
}

#[tokio::test]
#[serial_test::file_serial]
async fn send_to_unknown_peer_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let err = run_peer_send(&data_dir, "nonexistent", None, "test", TEST_PASSWORD)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("nonexistent"));
}

#[tokio::test]
#[serial_test::file_serial]
async fn unsupported_address_scheme_raises() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    run_peer(
        &data_dir,
        PeerAction::Add {
            alias: "xmtp-peer".into(),
            public_key_hex: "cd".repeat(33),
            address: "xmtp://0xRecipient".into(),
            role: PeerRole::Worker,
            notes: None,
        },
    )
    .unwrap();

    let err = run_peer_send(&data_dir, "xmtp-peer", None, "test", TEST_PASSWORD)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("scheme 미지원"));
}
