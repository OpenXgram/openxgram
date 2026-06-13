//! rc.322 throwaway test — keystore rekey 가 비번 불일치 keyfile 을 skip 하고
//! 진행하는지(resilient) + 정상(일관된) keystore rekey 가 e2e 동작하는지 검증.
//!
//! 시나리오:
//!   - keyA: pw A 로 생성, keyB: pw B(다른 비번) 로 생성 → keystore 가 mixed.
//!   - reencrypt_all(A, B'(=new)) 호출:
//!       keyA 는 A 로 복호화 성공 → 재암호화(=1),
//!       keyB 는 A 로 복호화 실패(InvalidPassword) → skip(=1).
//!   - 반환 (reencrypted=1, skipped=[keyB]). 에러 없이 진행.
//!   - keyA 는 new 로 load 성공. keyB 는 원본(B) 그대로 남아 load(B) 성공.

use openxgram_keystore::{FsKeystore, Keystore};

#[test]
fn mixed_keystore_rekey_skips_mismatched_and_succeeds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ks = FsKeystore::new(tmp.path());

    let pw_a = "passwordA-aaaaaaaa";
    let pw_b = "passwordB-bbbbbbbb"; // keyB 는 완전히 다른 비번으로 만든다.
    let pw_new = "passwordNEW-cccccc";

    ks.create("keyA", pw_a).expect("create keyA");
    ks.create("keyB", pw_b).expect("create keyB (different pw → mixed keystore)");

    // old=pw_a 로 reencrypt: keyA 성공, keyB 는 InvalidPassword → skip.
    let (reencrypted, skipped) = ks
        .reencrypt_all(pw_a, pw_new)
        .expect("reencrypt_all must NOT error on mixed keystore (resilient)");

    assert_eq!(reencrypted, 1, "keyA 만 재암호화되어야 한다");
    assert_eq!(skipped, vec!["keyB".to_string()], "keyB 가 skip 되어야 한다");

    // keyA 는 새 비번으로 열려야 한다.
    ks.load("keyA", pw_new).expect("keyA 는 new 비번으로 load 성공해야 함");
    // keyA 는 더 이상 old 비번으로 열리면 안 된다(실제 재암호화됨).
    assert!(ks.load("keyA", pw_a).is_err(), "keyA 는 old 비번으로 열리면 안 됨");

    // keyB 는 손대지 않았으므로 원본 비번(B)으로 여전히 열린다.
    ks.load("keyB", pw_b).expect("keyB 는 원본 비번 B 로 그대로 load 되어야 함");
    // keyB 는 new 비번으로 열리면 안 된다(skip 되어 재암호화 안 됨).
    assert!(ks.load("keyB", pw_new).is_err(), "keyB 는 new 비번으로 열리면 안 됨");
}

#[test]
fn consistent_keystore_rekey_end_to_end() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ks = FsKeystore::new(tmp.path());

    let pw_old = "consistent-old-1234";
    let pw_new = "consistent-new-5678";

    ks.create("k1", pw_old).expect("create k1");
    ks.create("k2", pw_old).expect("create k2");

    let (reencrypted, skipped) = ks.reencrypt_all(pw_old, pw_new).expect("reencrypt_all");
    assert_eq!(reencrypted, 2, "일관된 keystore 는 전부 재암호화");
    assert!(skipped.is_empty(), "skip 없어야 한다");

    ks.load("k1", pw_new).expect("k1 new load");
    ks.load("k2", pw_new).expect("k2 new load");
    assert!(ks.load("k1", pw_old).is_err());
}
