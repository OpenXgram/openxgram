//! Vault confirm/mfa policy enforcement 통합 테스트.

use openxgram_db::{Db, DbConfig};
use openxgram_vault::{AclAction, AclPolicy, PendingStatus, VaultError, VaultStore};
use tempfile::tempdir;

const PW: &str = "policy-test-12345";

fn open_db(dir: &std::path::Path) -> Db {
    let mut db = Db::open(DbConfig {
        path: dir.join("test.sqlite"),
        ..Default::default()
    })
    .unwrap();
    db.migrate().unwrap();
    db
}

#[test]
fn confirm_policy_creates_pending_and_blocks() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xAlice", &[AclAction::Get], 0, AclPolicy::Confirm)
        .unwrap();

    let err = v.get_as("k", PW, "0xAlice").unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("마스터 승인 대기"));
    // pending 큐에 1건 들어가야 함
    let pending = v.list_pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].agent, "0xAlice");
    assert_eq!(pending[0].action, AclAction::Get);
    assert_eq!(pending[0].status, PendingStatus::Pending);
}

#[test]
fn approve_then_retry_succeeds_once() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xAlice", &[AclAction::Get], 0, AclPolicy::Confirm)
        .unwrap();

    // 1차 호출 — pending 생성
    let _ = v.get_as("k", PW, "0xAlice");
    let id = v.list_pending().unwrap()[0].id.clone();

    // 마스터 승인
    v.approve_confirmation(&id).unwrap();

    // 재시도 — 성공
    let bytes = v.get_as("k", PW, "0xAlice").unwrap();
    assert_eq!(bytes, b"V");

    // 2차 재시도 — 다시 pending 생성 (consume 후 새 승인 필요)
    let err = v.get_as("k", PW, "0xAlice").unwrap_err();
    assert!(format!("{err}").contains("마스터 승인 대기"));
}

#[test]
fn deny_blocks_subsequent_calls() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xA", &[AclAction::Get], 0, AclPolicy::Confirm)
        .unwrap();

    let _ = v.get_as("k", PW, "0xA");
    let id = v.list_pending().unwrap()[0].id.clone();
    v.deny_confirmation(&id).unwrap();
    // denied 는 consume 안 되므로 다시 pending 생성됨
    let err = v.get_as("k", PW, "0xA").unwrap_err();
    assert!(format!("{err}").contains("마스터 승인 대기"));
    let pending_count = v.list_pending().unwrap().len();
    assert_eq!(pending_count, 1, "새 pending 만 남음 (이전은 denied)");
}

#[test]
fn approve_unknown_id_raises() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    let err = v.approve_confirmation("nope").unwrap_err();
    assert!(matches!(err, VaultError::NotFound(_)));
}

#[test]
fn approve_already_decided_raises() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xA", &[AclAction::Get], 0, AclPolicy::Confirm)
        .unwrap();
    let _ = v.get_as("k", PW, "0xA");
    let id = v.list_pending().unwrap()[0].id.clone();
    v.approve_confirmation(&id).unwrap();
    // 두 번째 approve → already decided
    let err = v.approve_confirmation(&id).unwrap_err();
    assert!(matches!(err, VaultError::NotFound(_)));
}

#[test]
fn mfa_policy_requires_code() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xA", &[AclAction::Get], 0, AclPolicy::Mfa)
        .unwrap();

    let err = v.get_as("k", PW, "0xA").unwrap_err();
    assert!(format!("{err}").contains("TOTP 코드 필요"));
}

#[test]
fn mfa_validates_correct_code_and_rejects_wrong() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xA", &[AclAction::Get], 0, AclPolicy::Mfa)
        .unwrap();

    let secret_b32 = v.issue_mfa_secret("0xA").unwrap();
    // 직접 TOTP 도구로 현재 코드 계산 → 검증 통과
    let raw = totp_rs::Secret::Encoded(secret_b32).to_bytes().unwrap();
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        raw,
        Some("OpenXgram".into()),
        "0xA".into(),
    )
    .unwrap();
    let code = totp.generate_current().unwrap();

    let bytes = v.get_as_authed("k", PW, "0xA", Some(&code)).unwrap();
    assert_eq!(bytes, b"V");

    // 잘못된 코드 → 거부
    let err = v.get_as_authed("k", PW, "0xA", Some("000000")).unwrap_err();
    assert!(format!("{err}").contains("mfa 코드 검증 실패"));
}

#[test]
fn mfa_unregistered_agent_raises() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xA", &[AclAction::Get], 0, AclPolicy::Mfa)
        .unwrap();
    let err = v.get_as_authed("k", PW, "0xA", Some("123456")).unwrap_err();
    assert!(format!("{err}").contains("mfa secret 미등록"));
}

#[test]
fn auto_policy_unaffected() {
    let tmp = tempdir().unwrap();
    let mut db = open_db(tmp.path());
    let mut v = VaultStore::new(&mut db);
    v.set("k", b"V", PW, &[]).unwrap();
    v.upsert_acl("k", "0xA", &[AclAction::Get], 0, AclPolicy::Auto)
        .unwrap();
    // auto 는 즉시 통과
    assert_eq!(v.get_as("k", PW, "0xA").unwrap(), b"V");
}
