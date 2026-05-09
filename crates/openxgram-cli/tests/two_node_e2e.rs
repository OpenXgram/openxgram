//! Phase 2 multi-instance e2e — 같은 프로세스 안에서 두 데이터 디렉터리 + 두 transport 서버를 띄워
//! 실 노드 A↔B 라운드트립을 시뮬. master deploy 가 가능한 환경 없이도 회로 검증.
//!
//! 커버 leaf:
//! - 2.1.3 양쪽 daemon health (transport server 양쪽 alive)
//! - 2.2.3 양쪽 peer list 확인
//! - 2.3.1 A → B 한 줄 메시지
//! - 2.3.2 B 받음 + 메모리 기록
//! - 2.3.3 B 응답 → A
//! - 2.3.4 conversation_id 동기 — 이미 cross_node_conversation 에서 검증, 본 테스트는 라운드트립 흐름

use openxgram_cli::daemon::process_inbound;
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::peer::{run_peer, PeerAction};
use openxgram_cli::peer_send::run_peer_send_with_conv;
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::MachineRole;
use openxgram_memory::{default_embedder, MessageStore, SessionStore};
use openxgram_peer::{PeerRole, PeerStore};
use openxgram_transport::spawn_server;
use std::path::PathBuf;
use tempfile::tempdir;

const PW_A: &str = "alice-pass-1234567";
const PW_B: &str = "bob-pass-7654321";

fn init_opts(data_dir: PathBuf, alias: &str) -> InitOpts {
    InitOpts {
        alias: alias.into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

#[tokio::test]
#[serial_test::file_serial]
async fn two_node_round_trip_with_conversation_thread() {
    unsafe {
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
        std::env::remove_var("XGRAM_SEED");
    }
    let tmp_a = tempdir().unwrap();
    let tmp_b = tempdir().unwrap();
    let dir_a = tmp_a.path().join("a");
    let dir_b = tmp_b.path().join("b");

    // 2.1.3 — 양 노드 init (master keypair 생성)
    unsafe { std::env::set_var("XGRAM_KEYSTORE_PASSWORD", PW_A); }
    run_init(&init_opts(dir_a.clone(), "alice")).unwrap();
    unsafe { std::env::set_var("XGRAM_KEYSTORE_PASSWORD", PW_B); }
    run_init(&init_opts(dir_b.clone(), "bob")).unwrap();

    // master keypair load
    let ks_a = FsKeystore::new(keystore_dir(&dir_a));
    let master_a = ks_a.load(MASTER_KEY_NAME, PW_A).unwrap();
    let pubkey_a = hex::encode(master_a.public_key_bytes());
    let eth_a = master_a.address.to_string();

    let ks_b = FsKeystore::new(keystore_dir(&dir_b));
    let master_b = ks_b.load(MASTER_KEY_NAME, PW_B).unwrap();
    let pubkey_b = hex::encode(master_b.public_key_bytes());
    let eth_b = master_b.address.to_string();

    // 2.1.3 — 두 transport 서버 띄움 (양쪽 daemon health)
    let server_a = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let server_b = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let url_a = format!("http://{}", server_a.bound_addr);
    let url_b = format!("http://{}", server_b.bound_addr);

    // 2.2.1 / 2.2.2 — 양쪽 peer add
    unsafe { std::env::set_var("XGRAM_KEYSTORE_PASSWORD", PW_A); }
    run_peer(
        &dir_a,
        PeerAction::Add {
            alias: "bob".into(),
            public_key_hex: pubkey_b.clone(),
            address: url_b.clone(),
            role: PeerRole::Worker,
            notes: None,
        },
    )
    .unwrap();
    // eth_address 직접 update — process_inbound 매칭용
    {
        let mut db = Db::open(DbConfig {
            path: db_path(&dir_a),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        db.conn()
            .execute(
                "UPDATE peers SET eth_address = ?1 WHERE alias = 'bob'",
                [&eth_b],
            )
            .unwrap();
    }

    unsafe { std::env::set_var("XGRAM_KEYSTORE_PASSWORD", PW_B); }
    run_peer(
        &dir_b,
        PeerAction::Add {
            alias: "alice".into(),
            public_key_hex: pubkey_a.clone(),
            address: url_a.clone(),
            role: PeerRole::Worker,
            notes: None,
        },
    )
    .unwrap();
    {
        let mut db = Db::open(DbConfig {
            path: db_path(&dir_b),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        db.conn()
            .execute(
                "UPDATE peers SET eth_address = ?1 WHERE alias = 'alice'",
                [&eth_a],
            )
            .unwrap();
    }

    // 2.2.3 — 양쪽 peer list 확인
    {
        let mut db_a = Db::open(DbConfig {
            path: db_path(&dir_a),
            ..Default::default()
        })
        .unwrap();
        db_a.migrate().unwrap();
        let peers_a: Vec<_> = PeerStore::new(&mut db_a)
            .list()
            .unwrap()
            .into_iter()
            .filter(|p| p.alias == "bob")
            .collect();
        assert_eq!(peers_a.len(), 1, "A 의 peer list 에 bob 등록");

        let mut db_b = Db::open(DbConfig {
            path: db_path(&dir_b),
            ..Default::default()
        })
        .unwrap();
        db_b.migrate().unwrap();
        let peers_b: Vec<_> = PeerStore::new(&mut db_b)
            .list()
            .unwrap()
            .into_iter()
            .filter(|p| p.alias == "alice")
            .collect();
        assert_eq!(peers_b.len(), 1, "B 의 peer list 에 alice 등록");
    }

    // 2.3.1 — A → B 한 줄 메시지 (conversation_id 동봉)
    let conv_thread = "thread-2node-1234".to_string();
    unsafe { std::env::set_var("XGRAM_KEYSTORE_PASSWORD", PW_A); }
    run_peer_send_with_conv(
        &dir_a,
        "bob",
        None,
        "alice → bob: 안녕",
        PW_A,
        Some(conv_thread.clone()),
    )
    .await
    .unwrap();

    // server_b 가 envelope 받음 — drain + B 의 process_inbound
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let envs_b = server_b.drain_received();
    assert_eq!(envs_b.len(), 1, "2.3.1 — B 의 transport 가 envelope 1개 수신");
    assert_eq!(envs_b[0].conversation_id.as_deref(), Some(conv_thread.as_str()));
    process_inbound(&dir_b, &envs_b).unwrap();

    // 2.3.2 — B 메모리 기록 검증
    let mut db_b = Db::open(DbConfig {
        path: db_path(&dir_b),
        ..Default::default()
    })
    .unwrap();
    db_b.migrate().unwrap();
    let embedder = default_embedder().unwrap();
    let inbox_b = SessionStore::new(&mut db_b)
        .list()
        .unwrap()
        .into_iter()
        .find(|s| s.title == "inbox-from-alice")
        .expect("B 측 inbox-from-alice 세션 존재");
    let msgs_b = MessageStore::new(&mut db_b, embedder.as_ref())
        .list_for_session(&inbox_b.id)
        .unwrap();
    assert_eq!(msgs_b.len(), 1);
    assert_eq!(msgs_b[0].body, "alice → bob: 안녕");
    assert_eq!(msgs_b[0].sender, "peer:alice", "1.9.1.3 sender format");
    assert_eq!(msgs_b[0].conversation_id, conv_thread, "2.3.4 conv_id 보존");

    // 2.3.3 — B 응답 → A (같은 conversation_id thread 유지)
    unsafe { std::env::set_var("XGRAM_KEYSTORE_PASSWORD", PW_B); }
    run_peer_send_with_conv(
        &dir_b,
        "alice",
        None,
        "bob → alice: 잘 받음",
        PW_B,
        Some(conv_thread.clone()),
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let envs_a = server_a.drain_received();
    assert_eq!(envs_a.len(), 1, "2.3.3 — A 가 응답 envelope 받음");
    assert_eq!(envs_a[0].conversation_id.as_deref(), Some(conv_thread.as_str()));
    process_inbound(&dir_a, &envs_a).unwrap();

    let mut db_a = Db::open(DbConfig {
        path: db_path(&dir_a),
        ..Default::default()
    })
    .unwrap();
    db_a.migrate().unwrap();
    let inbox_a = SessionStore::new(&mut db_a)
        .list()
        .unwrap()
        .into_iter()
        .find(|s| s.title == "inbox-from-bob")
        .expect("A 측 inbox-from-bob 세션 존재");
    let msgs_a = MessageStore::new(&mut db_a, embedder.as_ref())
        .list_for_session(&inbox_a.id)
        .unwrap();
    assert_eq!(msgs_a.len(), 1);
    assert_eq!(msgs_a[0].body, "bob → alice: 잘 받음");
    assert_eq!(msgs_a[0].sender, "peer:bob");
    assert_eq!(msgs_a[0].conversation_id, conv_thread);

    // 2.3.4 — 양 노드의 같은 conversation thread 확인
    let thread_a = MessageStore::new(&mut db_a, embedder.as_ref())
        .list_for_conversation(&conv_thread)
        .unwrap();
    let thread_b = MessageStore::new(&mut db_b, embedder.as_ref())
        .list_for_conversation(&conv_thread)
        .unwrap();
    assert_eq!(thread_a.len(), 1, "A 의 thread = bob 응답 1개");
    assert_eq!(thread_b.len(), 1, "B 의 thread = alice 송신 1개");
    // 두 노드의 thread 메시지가 같은 conversation_id 로 묶임 (cross-node 동기)
    assert_eq!(thread_a[0].conversation_id, thread_b[0].conversation_id);

    server_a.shutdown();
    server_b.shutdown();
}
