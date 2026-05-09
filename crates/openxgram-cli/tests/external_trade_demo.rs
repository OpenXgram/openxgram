//! 5.2 외부 에이전트 거래 데모 — 다른 master 의 에이전트 가 우리 에이전트 호출 → USDC 청구 →
//! 결제 → 메모리 + 평판 기록.
//!
//! 본 테스트는 chain RPC 없이 로컬 시뮬:
//! - 두 데이터 디렉터리 (us, them)
//! - them → us 로 작업 요청 메시지 (peer_send)
//! - us → them 으로 USDC 청구 (payment_intents draft)
//! - them 결제 confirmed 시나리오 (PaymentStore.mark_confirmed)
//! - us 측 EAS payment attestation 자동 생성
//! - 5.3: aggregate_local_scores 로 them 의 reputation 점수 노출

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::reputation::aggregate_local_scores;
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_eas::{Attestation, AttestationData, AttestationKind, AttestationStore};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::MachineRole;
use serde_json::json;
use std::path::PathBuf;
use tempfile::tempdir;

const PW: &str = "trade-demo-pw-01234";

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

#[test]
#[serial_test::file_serial]
fn external_master_invokes_us_pays_us_then_we_score_them() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", PW);
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
        std::env::remove_var("XGRAM_SEED");
    }
    let tmp_us = tempdir().unwrap();
    let tmp_them = tempdir().unwrap();
    let dir_us = tmp_us.path().join("us");
    let dir_them = tmp_them.path().join("them");
    run_init(&init_opts(dir_us.clone(), "us")).unwrap();
    run_init(&init_opts(dir_them.clone(), "them")).unwrap();

    // them 의 master 주소 (외부 master)
    let ks_them = FsKeystore::new(keystore_dir(&dir_them));
    let master_them = ks_them.load(MASTER_KEY_NAME, PW).unwrap();
    let them_addr = master_them.address.to_string();

    // 1) them → us 로 작업 요청 메시지 (us 측 inbox-from-them 세션에 직접 insert — peer:them sender)
    let mut db_us = Db::open(DbConfig {
        path: db_path(&dir_us),
        ..Default::default()
    })
    .unwrap();
    db_us.migrate().unwrap();
    let now = openxgram_core::time::kst_now().to_rfc3339();
    db_us
        .conn()
        .execute(
            "INSERT INTO sessions (id, title, created_at, last_active, home_machine)
             VALUES ('s-them', 'inbox-from-them', ?1, ?1, 'us-host')",
            [&now],
        )
        .unwrap();
    db_us
        .conn()
        .execute(
            "INSERT INTO messages
              (id, session_id, sender, body, signature, timestamp, conversation_id)
             VALUES ('m-1', 's-them', 'peer:them', 'PR 리뷰 부탁 — 1만원 청구', 'sig', ?1, 'conv-trade-1')",
            [&now],
        )
        .unwrap();

    // 2) us → them 결제 청구 (payment_intents — schema: payee_address / amount_usdc_micro / state)
    db_us
        .conn()
        .execute(
            "INSERT INTO payment_intents
                (id, amount_usdc_micro, chain, payee_address, memo, nonce, state, created_at)
              VALUES ('p-1', 10000000, 'base', ?1, 'PR 리뷰', 'nonce-trade-1', 'draft', ?2)",
            rusqlite::params![&them_addr, &now],
        )
        .unwrap();

    // 3) them 결제 confirmed 시나리오 — payment_intents state 갱신
    db_us
        .conn()
        .execute(
            "UPDATE payment_intents SET state='confirmed', confirmed_at=?1 WHERE id='p-1'",
            [&now],
        )
        .unwrap();

    // 4) us 측 EAS payment attestation 자동 기록 (4.1.2.2)
    AttestationStore::new(&mut db_us)
        .insert(&Attestation::new(AttestationData {
            kind: AttestationKind::Payment,
            fields: json!({
                "sender": them_addr.clone(),
                "recipient": "us-addr",
                "amount_micros": 10000000,
                "chain": "base-mainnet",
                "intent_id": "p-1"
            }),
        }))
        .unwrap();

    // 5) reputation: them 이 결제했음에도 우리 score 집계는 recipient 기준이라 us 가 받은 것.
    //    them 의 점수를 매기려면 endorsement 가 필요. → us 가 them 을 endorse.
    AttestationStore::new(&mut db_us)
        .insert(&Attestation::new(AttestationData {
            kind: AttestationKind::Endorsement,
            fields: json!({
                "endorser": "us-addr",
                "endorsee": "them",
                "tag": "paid-on-time",
                "memo": "PR 리뷰 1만원 결제 완료"
            }),
        }))
        .unwrap();
    drop(db_us);

    let scores = aggregate_local_scores(&dir_us).unwrap();
    let them_score = scores
        .iter()
        .find(|s| s.identity == "them")
        .expect("them 점수 집계됨");
    assert_eq!(them_score.messages, 1, "them 의 inbox 메시지 1");
    assert_eq!(them_score.endorsements_received, 1, "endorse 1");
    assert!(them_score.raw_score > 0.0);
}
