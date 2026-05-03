//! Payment intent 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_payment::{PaymentError, PaymentState, PaymentStore};
use tempfile::tempdir;

fn open_db(dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: dir.join("test.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    db
}

fn make_master(dir: &std::path::Path) -> openxgram_keystore::Keypair {
    let ks = FsKeystore::new(dir);
    let (_addr, _phrase) = ks.create("master", "test-pw-12345").unwrap();
    ks.load("master", "test-pw-12345").unwrap()
}

#[test]
fn migration_creates_payment_intents_table() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 11",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn create_draft_then_sign() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let master = make_master(tmp.path());
    let mut store = PaymentStore::new(&mut db);

    let intent = store
        .create_draft(
            1_500_000, // 1.50 USDC
            "base",
            "0xrecipient000000000000000000000000000000",
            Some("test memo"),
        )
        .unwrap();
    assert_eq!(intent.state, PaymentState::Draft);
    assert!(intent.signature_hex.is_none());
    assert_eq!(intent.amount_display(), "1.5 USDC");

    let signed = store.sign(&intent.id, &master).unwrap();
    assert_eq!(signed.state, PaymentState::Signed);
    let sig = signed.signature_hex.expect("signature 채워짐");
    assert!(!sig.is_empty());
    // ECDSA secp256k1 → 일반적으로 64 bytes (compact) → hex 128
    // signature 형식은 keystore 구현에 따라 길이 다를 수 있어 0보다 크면 OK
    assert!(sig.len() >= 64);
    assert!(signed.signed_at.is_some());
}

#[test]
fn invalid_amount_rejected() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PaymentStore::new(&mut db);
    let err = store.create_draft(0, "base", "0xa", None).unwrap_err();
    assert!(matches!(err, PaymentError::InvalidAmount(_)));
    let err = store.create_draft(-100, "base", "0xa", None).unwrap_err();
    assert!(matches!(err, PaymentError::InvalidAmount(_)));
}

#[test]
fn sign_twice_rejected() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let master = make_master(tmp.path());
    let mut store = PaymentStore::new(&mut db);
    let intent = store.create_draft(1, "base", "0xa", None).unwrap();
    store.sign(&intent.id, &master).unwrap();
    let err = store.sign(&intent.id, &master).unwrap_err();
    assert!(matches!(err, PaymentError::InvalidTransition { .. }));
}

#[test]
fn state_transitions() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let master = make_master(tmp.path());
    let mut store = PaymentStore::new(&mut db);

    let intent = store
        .create_draft(1_000_000, "polygon", "0xb", None)
        .unwrap();
    store.sign(&intent.id, &master).unwrap();
    store.mark_submitted(&intent.id, "0xtxhash123").unwrap();
    store.mark_confirmed(&intent.id).unwrap();

    let final_state = store.get(&intent.id).unwrap().unwrap();
    assert_eq!(final_state.state, PaymentState::Confirmed);
    assert_eq!(
        final_state.submitted_tx_hash.as_deref(),
        Some("0xtxhash123")
    );
    assert!(final_state.confirmed_at.is_some());
}

#[test]
fn skip_state_rejected() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PaymentStore::new(&mut db);
    let intent = store.create_draft(1, "base", "0xa", None).unwrap();
    // draft → submitted (skip signed) → reject
    let err = store.mark_submitted(&intent.id, "0xtx").unwrap_err();
    assert!(matches!(err, PaymentError::InvalidTransition { .. }));
}

#[test]
fn mark_failed_works_from_any_state() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PaymentStore::new(&mut db);
    let intent = store.create_draft(1, "base", "0xa", None).unwrap();
    store.mark_failed(&intent.id, "RPC reject").unwrap();
    let s = store.get(&intent.id).unwrap().unwrap();
    assert_eq!(s.state, PaymentState::Failed);
    assert_eq!(s.error_reason.as_deref(), Some("RPC reject"));
}

#[test]
fn list_returns_descending_creation() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PaymentStore::new(&mut db);
    for i in 0..3 {
        store
            .create_draft((i + 1) * 1_000_000, "base", "0xa", None)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let list = store.list().unwrap();
    assert_eq!(list.len(), 3);
    assert!(list[0].created_at >= list[1].created_at);
}

#[test]
fn signing_bytes_canonical() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PaymentStore::new(&mut db);
    let intent = store
        .create_draft(2_500_000, "base", "0xrecipient", Some("test"))
        .unwrap();
    let bytes = intent.signing_bytes();
    let s = std::str::from_utf8(&bytes).unwrap();
    assert!(s.starts_with("openxgram-payment-v1\n"));
    assert!(s.contains("\nbase\n"));
    assert!(s.contains("\n0xrecipient\n"));
    assert!(s.contains("\n2500000\n"));
    assert!(s.ends_with("\ntest"));
}

#[test]
fn amount_display_formats_correctly() {
    let cases = [
        (1_000_000, "1 USDC"),
        (1_500_000, "1.5 USDC"),
        (1_234_567, "1.234567 USDC"),
        (100, "0.0001 USDC"),
        (1, "0.000001 USDC"),
    ];
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut store = PaymentStore::new(&mut db);
    for (micro, expected) in cases {
        let intent = store.create_draft(micro, "base", "0xa", None).unwrap();
        assert_eq!(intent.amount_display(), expected, "micro={micro}");
    }
}
