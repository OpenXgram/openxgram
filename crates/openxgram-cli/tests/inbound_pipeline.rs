//! PRD-2.0.1 / 2.0.2 / 2.0.3 통합 테스트 — inbound pipeline.
//!
//! 시나리오:
//!   1. init data_dir (master keypair 생성)
//!   2. master 를 self-peer 로 등록 (eth_address + public_key_hex)
//!   3. master 가 envelope 서명
//!   4. process_inbound 직접 호출
//!   5. 검증: peer last_seen 갱신 / inbox session 자동 생성 / L0 message insert

use openxgram_cli::daemon::process_inbound;
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::peer::{run_peer, PeerAction};
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::MachineRole;
use openxgram_peer::PeerRole;
use openxgram_transport::Envelope;
use std::path::PathBuf;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "inbound-pipeline".into(),
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

fn make_envelope(
    master: &openxgram_keystore::Keypair,
    payload: &[u8],
    to_pubkey: &str,
) -> Envelope {
    let signature = master.sign(payload);
    Envelope {
        from: master.address.to_string(),
        to: to_pubkey.into(),
        payload_hex: hex::encode(payload),
        timestamp: openxgram_core::time::kst_now(),
        signature_hex: hex::encode(signature),
    }
}

#[test]
#[serial_test::file_serial]
fn valid_envelope_round_trip_stores_message_and_touches_peer() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // master keypair 로드
    let ks = FsKeystore::new(keystore_dir(&data_dir));
    let master = ks.load(MASTER_KEY_NAME, TEST_PASSWORD).unwrap();
    let pubkey_hex = hex::encode(master.public_key_bytes());
    let eth_addr = master.address.to_string();

    // self-peer 등록 (eth_address 포함)
    use openxgram_peer::PeerStore;
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    PeerStore::new(&mut db)
        .add_with_eth(
            "self-test",
            &pubkey_hex,
            "http://127.0.0.1:0",
            Some(&eth_addr),
            PeerRole::Worker,
            None,
        )
        .unwrap();
    drop(db);

    // 서명된 envelope
    let body = b"hello from self";
    let env = make_envelope(&master, body, &pubkey_hex);

    // process_inbound 직접 호출
    process_inbound(&data_dir, &[env]).unwrap();

    // 검증 1: peer last_seen 갱신
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let peer = PeerStore::new(&mut db)
        .get_by_alias("self-test")
        .unwrap()
        .unwrap();
    assert!(peer.last_seen.is_some(), "last_seen 갱신");

    // 검증 2: inbox session 자동 생성
    use openxgram_memory::SessionStore;
    let sessions = SessionStore::new(&mut db).list().unwrap();
    let inbox = sessions
        .iter()
        .find(|s| s.title == "inbox-from-self-test")
        .expect("inbox session 자동 생성");

    // 검증 3: L0 message 저장
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            [&inbox.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "L0 message insert");
}

#[test]
#[serial_test::file_serial]
fn invalid_signature_drops_envelope() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let ks = FsKeystore::new(keystore_dir(&data_dir));
    let master = ks.load(MASTER_KEY_NAME, TEST_PASSWORD).unwrap();
    let pubkey_hex = hex::encode(master.public_key_bytes());
    let eth_addr = master.address.to_string();

    run_peer(
        &data_dir,
        PeerAction::Add {
            alias: "self".into(),
            public_key_hex: pubkey_hex.clone(),
            address: "http://x".into(),
            role: PeerRole::Worker,
            notes: None,
        },
    )
    .unwrap();
    // eth_address 등록도 필요 — 직접 update
    {
        let mut db = Db::open(DbConfig {
            path: db_path(&data_dir),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        db.conn()
            .execute(
                "UPDATE peers SET eth_address = ?1 WHERE alias = 'self'",
                [&eth_addr],
            )
            .unwrap();
    }

    // 잘못된 서명 (패딩만 다른 0...0 64 bytes)
    let body = b"forged";
    let bogus_sig = vec![0u8; 64];
    let env = Envelope {
        from: eth_addr.clone(),
        to: pubkey_hex,
        payload_hex: hex::encode(body),
        timestamp: openxgram_core::time::kst_now(),
        signature_hex: hex::encode(&bogus_sig),
    };

    process_inbound(&data_dir, &[env]).unwrap();

    // 검증: message 0건 (drop)
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "위조 서명 envelope 은 drop");
}

#[test]
#[serial_test::file_serial]
fn unknown_peer_drops_envelope() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // peer 등록 안 함
    let env = Envelope {
        from: "0xUnknown000000000000000000000000UnknownX".into(),
        to: "ab".repeat(33),
        payload_hex: hex::encode(b"x"),
        timestamp: openxgram_core::time::kst_now(),
        signature_hex: hex::encode([0u8; 64]),
    };
    process_inbound(&data_dir, &[env]).unwrap();

    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
