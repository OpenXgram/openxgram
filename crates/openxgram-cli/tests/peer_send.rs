//! peer-send 통합 테스트 — peer 등록 → 로컬 transport 띄우기 → send → received 확인.

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::peer::{run_peer, PeerAction};
use openxgram_cli::peer_send::{run_peer_broadcast, run_peer_send};
use openxgram_manifest::MachineRole;
use openxgram_peer::PeerRole;
use openxgram_transport::spawn_server;
use std::path::PathBuf;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

// 유효한 secp256k1 compressed pubkey 테스트 벡터 (k=1..4 의 G·2G·3G·4G).
// PR #138 이후 CLI 의 peer add 가 sec1 검증 + eth_address 도출하므로 실 점이 필요.
const PK_G1: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const PK_G2: &str = "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
const PK_G3: &str = "02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";
const PK_G4: &str = "02e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13";

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
            public_key_hex: PK_G1.into(),
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
async fn broadcast_to_multiple_peers_partial_failure() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // peer A — 정상
    let server_a = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr_a = format!("http://{}", server_a.bound_addr);
    run_peer(
        &data_dir,
        PeerAction::Add {
            alias: "alpha".into(),
            public_key_hex: PK_G2.into(),
            address: addr_a,
            role: PeerRole::Worker,
            notes: None,
        },
    )
    .unwrap();

    // peer B — 잘못된 주소 (closed port)
    run_peer(
        &data_dir,
        PeerAction::Add {
            alias: "beta".into(),
            public_key_hex: PK_G3.into(),
            address: "http://127.0.0.1:1".into(), // 거의 확실히 닫힘
            role: PeerRole::Worker,
            notes: None,
        },
    )
    .unwrap();

    let aliases = vec!["alpha".to_string(), "beta".to_string()];
    let results = run_peer_broadcast(&data_dir, &aliases, "broadcast test", TEST_PASSWORD)
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    let alpha_result = results.iter().find(|(a, _)| a == "alpha").unwrap();
    let beta_result = results.iter().find(|(a, _)| a == "beta").unwrap();
    assert!(alpha_result.1.is_ok(), "alpha 성공 기대");
    assert!(beta_result.1.is_err(), "beta 실패 (port 닫힘) 기대");

    // alpha 만 received
    assert_eq!(server_a.received().len(), 1);
    server_a.shutdown();
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
            public_key_hex: PK_G4.into(),
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
