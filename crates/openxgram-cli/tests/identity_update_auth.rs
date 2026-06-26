//! 보안 핫픽스 통합 테스트 — identity_update auth-bypass 패치.
//!
//! 취약점(패치 전): identity_update envelope 은 인가 게이트 없이 임의 alias 의
//! display_name/role 을 원격 변경할 수 있었다. 누구든(미등록·미신뢰 발신자) 가능.
//!
//! 기대(패치 후): identity_update 는 발신자가
//!   (1) 등록 peer + (2) 서명검증 통과 + (3) XGRAM_TRUSTED_ISSUERS allowlist 멤버
//! 일 때만 DB 를 변경한다. 그 외에는 무시(변경 0).

use openxgram_cli::daemon::process_inbound;
use openxgram_cli::init::{run_init, InitOpts};
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keypair, Keystore};
use openxgram_manifest::MachineRole;
use openxgram_peer::{PeerRole, PeerStore};
use openxgram_transport::Envelope;
use std::path::PathBuf;
use tempfile::tempdir;

const TEST_PASSWORD: &str = "test-password-12345";

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "idupd-auth".into(),
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

/// identity_update envelope 빌더: 발신자가 payload 를 자기 키로 서명.
fn identity_update_env(sender: &Keypair, alias: &str, new_role: &str) -> Envelope {
    let payload = serde_json::json!({ "alias": alias, "role": new_role })
        .to_string()
        .into_bytes();
    let signature = sender.sign(&payload);
    Envelope {
        from: sender.address.to_string(),
        to: "irrelevant".into(),
        payload_hex: hex::encode(&payload),
        timestamp: openxgram_core::time::kst_now(),
        signature_hex: hex::encode(signature),
        nonce: None,
        conversation_id: None,
        sender_alias: None,
        sender_transport_url: None,
        sender_pubkey_hex: None,
        recipient_alias: None,
        envelope_type: Some("identity_update".into()),
        ack_for_ulid: None,
        ack_status: None,
    }
}

/// 대상 행(victim) + 발신 peer 를 등록하고 victim 의 현재 role 을 돌려준다.
fn setup(data_dir: &std::path::Path, sender: &Keypair, sender_alias: &str) {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    let mut ps = PeerStore::new(&mut db);
    // victim 행 — 누군가 바꾸려는 대상.
    ps.add_with_eth(
        "victim",
        // victim pubkey 는 아무거나(서명 대상 아님). 발신자 pubkey 와 무관.
        &"02".to_string().repeat(33),
        "http://127.0.0.1:0",
        Some("0xVictimEth"),
        PeerRole::Worker,
        None,
    )
    .ok();
    // 발신 peer 등록 (eth = sender.address, pubkey = sender pubkey).
    ps.add_with_eth(
        sender_alias,
        &hex::encode(sender.public_key_bytes()),
        "http://127.0.0.1:0",
        Some(&sender.address.to_string()),
        PeerRole::Worker,
        None,
    )
    .unwrap();
}

fn victim_role(data_dir: &std::path::Path) -> Option<String> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    db.conn()
        .query_row(
            "SELECT role FROM peers WHERE alias = 'victim'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
}

#[test]
#[serial_test::file_serial]
fn untrusted_sender_cannot_mutate_identity() {
    set_env();
    unsafe {
        // allowlist 비움 → default-deny, 아무도 mutate 못 함.
        std::env::remove_var("XGRAM_TRUSTED_ISSUERS");
    }
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // 공격자 keypair (등록은 되지만 allowlist 엔 없음).
    let attacker = Keypair::from_secret_bytes(&[7u8; 32]).unwrap();
    setup(&data_dir, &attacker, "attacker");
    let before = victim_role(&data_dir);

    // 공격자가 victim 의 role 을 "primary" 로 바꾸려 시도.
    let env = identity_update_env(&attacker, "victim", "primary");
    process_inbound(&data_dir, &[env]).unwrap();

    let after = victim_role(&data_dir);
    assert_eq!(before, after, "미신뢰 발신자는 identity 변경 불가 (auth-bypass 차단)");
}

#[test]
#[serial_test::file_serial]
fn trusted_fleet_sender_can_mutate_identity() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    // master 를 신뢰 fleet 발행자로 사용.
    let ks = FsKeystore::new(keystore_dir(&data_dir));
    let fleet = ks.load(MASTER_KEY_NAME, TEST_PASSWORD).unwrap();
    setup(&data_dir, &fleet, "fleet");

    unsafe {
        // allowlist 에 fleet 발행자 eth 추가.
        std::env::set_var("XGRAM_TRUSTED_ISSUERS", fleet.address.to_string());
    }

    let env = identity_update_env(&fleet, "victim", "primary");
    process_inbound(&data_dir, &[env]).unwrap();

    let after = victim_role(&data_dir);
    assert_eq!(
        after.as_deref(),
        Some("primary"),
        "신뢰 fleet 발신자 + 정당 서명 → identity 변경 적용"
    );
    unsafe {
        std::env::remove_var("XGRAM_TRUSTED_ISSUERS");
    }
}
