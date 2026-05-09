//! Phase 2.3.4 — cross-node conversation_id 동기.
//!
//! 시나리오:
//! - 노드 A (master keypair) 가 envelope 에 conversation_id 를 실어 보냄
//! - 노드 B (process_inbound) 가 받아서 inbox-from-A 세션의 메시지로 같은 conversation_id 보존
//! - 두 번째 envelope 도 같은 conversation_id → 한 conversation 으로 묶임
//! - conversation_id None envelope 은 별도 conversation

use openxgram_cli::daemon::process_inbound;
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::peer::{run_peer, PeerAction};
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::MachineRole;
use openxgram_memory::{default_embedder, MessageStore};
use openxgram_peer::PeerRole;
use openxgram_transport::Envelope;
use std::path::PathBuf;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "cross-node-conv".into(),
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

#[test]
#[serial_test::file_serial]
fn envelope_carries_conversation_id_to_inbox_message() {
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
    // process_inbound 은 envelope.from = eth_address 로 peer 매칭하므로 직접 update.
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

    let conv_a = "conv-aaaa-1111-2222-3333".to_string();

    // 첫 envelope — conversation_id 동봉
    let body1 = b"first message in thread";
    let env1 = Envelope {
        from: eth_addr.clone(),
        to: pubkey_hex.clone(),
        payload_hex: hex::encode(body1),
        timestamp: openxgram_core::time::kst_now(),
        signature_hex: hex::encode(master.sign(body1)),
        nonce: Some(uuid::Uuid::new_v4().to_string()),
        conversation_id: Some(conv_a.clone()),
    };

    // 두 번째 envelope — 같은 conversation_id 로 thread 이어가기
    let body2 = b"second message in same thread";
    let env2 = Envelope {
        from: eth_addr.clone(),
        to: pubkey_hex.clone(),
        payload_hex: hex::encode(body2),
        timestamp: openxgram_core::time::kst_now(),
        signature_hex: hex::encode(master.sign(body2)),
        nonce: Some(uuid::Uuid::new_v4().to_string()),
        conversation_id: Some(conv_a.clone()),
    };

    process_inbound(&data_dir, &[env1, env2]).unwrap();

    // 검증 — 두 envelope 모두 같은 conversation 에 묶임
    let mut db = Db::open(DbConfig {
        path: db_path(&data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let embedder = default_embedder().unwrap();
    let thread = MessageStore::new(&mut db, embedder.as_ref())
        .list_for_conversation(&conv_a)
        .unwrap();
    assert_eq!(thread.len(), 2, "두 envelope 모두 같은 conversation 에 묶임");
    assert!(thread.iter().all(|m| m.conversation_id == conv_a));
    assert_eq!(thread[0].body, "first message in thread");
    assert_eq!(thread[1].body, "second message in same thread");
    // 1.9.1.3: process_inbound 가 sender 를 `peer:{alias}` 형식으로 저장
    assert!(
        thread.iter().all(|m| m.sender == "peer:self"),
        "sender 는 peer:alias 형식 (라우팅용)"
    );

    // conv_id None envelope 은 별도 conversation
    let body3 = b"orphan envelope no conv id";
    let env3 = Envelope {
        from: eth_addr,
        to: pubkey_hex,
        payload_hex: hex::encode(body3),
        timestamp: openxgram_core::time::kst_now(),
        signature_hex: hex::encode(master.sign(body3)),
        nonce: Some(uuid::Uuid::new_v4().to_string()),
        conversation_id: None,
    };
    process_inbound(&data_dir, &[env3]).unwrap();
    let still_two = MessageStore::new(&mut db, embedder.as_ref())
        .list_for_conversation(&conv_a)
        .unwrap();
    assert_eq!(
        still_two.len(),
        2,
        "conv_id None envelope 은 conv_a thread 에 들어가지 않음"
    );
}
