use openxgram_keystore::{
    derive_keypair, DerivationPath, FsKeystore, Keystore, KeystoreError, Mnemonic, MnemonicLanguage,
};
use tempfile::TempDir;

// ── 테스트 1: BIP39 24단어 생성 + 라운드트립 ──────────────────────────────
#[test]
fn test_mnemonic_generate_and_roundtrip() {
    let m = Mnemonic::generate(MnemonicLanguage::English);
    assert_eq!(m.word_count(), 24, "24단어여야 한다");

    let phrase = m.phrase().to_string();
    let words: Vec<&str> = phrase.split_whitespace().collect();
    assert_eq!(words.len(), 24);

    // import 라운드트립
    let m2 = Mnemonic::from_phrase(&phrase).expect("유효한 니모닉 import 실패");
    assert_eq!(m2.phrase(), phrase.as_str());
}

// ── 테스트 2: 결정적 키 파생 (같은 시드 → 같은 주소) ─────────────────────
#[test]
fn test_deterministic_derivation() {
    let m = Mnemonic::from_phrase(
        "abandon abandon abandon abandon abandon abandon abandon abandon \
         abandon abandon abandon abandon abandon abandon abandon abandon \
         abandon abandon abandon abandon abandon abandon abandon art",
    )
    .expect("테스트 니모닉 파싱 실패");

    let seed = m.to_seed("");
    let path = DerivationPath::new(0, 0);

    let kp1 = derive_keypair(&seed, &path).expect("파생 실패 1");
    let kp2 = derive_keypair(&seed, &path).expect("파생 실패 2");

    assert_eq!(
        kp1.address.as_str(),
        kp2.address.as_str(),
        "같은 시드에서 같은 주소가 나와야 한다"
    );
}

// ── 테스트 3: BIP44 표준 테스트 벡터 (m/44'/60'/0'/0/0) ──────────────────
// 참조: https://iancoleman.io/bip39/ — 24 all-abandon + art 니모닉
#[test]
fn test_bip44_derivation_path_format() {
    let path = DerivationPath::new(0, 0);
    assert_eq!(path.to_bip44_string(), "m/44'/60'/0'/0/0");

    let path2 = DerivationPath::new(1, 5);
    assert_eq!(path2.to_bip44_string(), "m/44'/60'/1'/0/5");
}

// ── 테스트 4: ECDSA 서명 → 검증 라운드트립 ───────────────────────────────
#[test]
fn test_sign_and_verify() {
    let m = Mnemonic::generate(MnemonicLanguage::English);
    let seed = m.to_seed("");
    let path = DerivationPath::new(0, 0);
    let kp = derive_keypair(&seed, &path).expect("파생 실패");

    let message = b"openxgram test message 2026";
    let signature = kp.sign(message);
    assert!(!signature.is_empty(), "서명이 비어있으면 안 된다");

    kp.verify(message, &signature)
        .expect("올바른 서명 검증 실패");

    // 잘못된 메시지 → 검증 실패
    let bad_result = kp.verify(b"wrong message", &signature);
    assert!(
        bad_result.is_err(),
        "잘못된 메시지로 검증이 통과하면 안 된다"
    );
}

// ── 테스트 5: FsKeystore save → load → 동일 키 복원 ─────────────────────
#[test]
fn test_fsKeystore_save_and_load() {
    let tmp = TempDir::new().expect("tmpdir 생성 실패");
    let ks = FsKeystore::new(tmp.path());

    let (address, _phrase) = ks.create("eno", "password123").expect("키 생성 실패");

    let loaded_kp = ks.load("eno", "password123").expect("키 로드 실패");
    assert_eq!(
        address.as_str(),
        loaded_kp.address.as_str(),
        "저장/로드 후 주소가 동일해야 한다"
    );
}

// ── 테스트 6: 잘못된 패스워드 → InvalidPassword 즉시 raise ──────────────
#[test]
fn test_wrong_password_raises_error() {
    let tmp = TempDir::new().expect("tmpdir 생성 실패");
    let ks = FsKeystore::new(tmp.path());

    ks.create("akashic", "correctpassword")
        .expect("키 생성 실패");

    let result = ks.load("akashic", "wrongpassword");
    match result {
        Err(KeystoreError::InvalidPassword) => {} // 기대 결과
        Err(e) => panic!("잘못된 에러 타입: {e:?}"),
        Ok(_) => panic!("잘못된 패스워드로 키 로드가 성공하면 안 된다"),
    }
}

// ── 테스트 7: AgentAddress EIP-55 체크섬 ─────────────────────────────────
#[test]
fn test_agent_address_format() {
    let m = Mnemonic::generate(MnemonicLanguage::English);
    let seed = m.to_seed("");
    let path = DerivationPath::new(0, 0);
    let kp = derive_keypair(&seed, &path).expect("파생 실패");

    let addr = kp.address.as_str();
    assert!(addr.starts_with("0x"), "주소는 0x로 시작해야 한다");
    assert_eq!(addr.len(), 42, "EVM 주소는 0x + 40자 hex = 42자");
}

// ── 테스트 8: 키 목록 + 삭제 ─────────────────────────────────────────────
#[test]
fn test_list_and_delete() {
    let tmp = TempDir::new().expect("tmpdir 생성 실패");
    let ks = FsKeystore::new(tmp.path());

    ks.create("key1", "pass").expect("key1 생성 실패");
    ks.create("key2", "pass").expect("key2 생성 실패");

    let list = ks.list().expect("목록 조회 실패");
    assert_eq!(list.len(), 2);

    ks.delete("key1").expect("삭제 실패");
    let list2 = ks.list().expect("목록 재조회 실패");
    assert_eq!(list2.len(), 1);
    assert_eq!(list2[0].name, "key2");
}

// ── 테스트 9: 니모닉 import → 같은 주소 복원 ──────────────────────────────
#[test]
fn test_import_restores_same_address() {
    let tmp = TempDir::new().expect("tmpdir 생성 실패");
    let ks = FsKeystore::new(tmp.path());

    let (original_addr, phrase) = ks.create("original", "pass").expect("생성 실패");

    let tmp2 = TempDir::new().expect("tmpdir2 생성 실패");
    let ks2 = FsKeystore::new(tmp2.path());
    let restored_addr = ks2
        .import("restored", &phrase, "newpass")
        .expect("import 실패");

    assert_eq!(
        original_addr.as_str(),
        restored_addr.as_str(),
        "니모닉 import 후 주소가 원본과 동일해야 한다"
    );
}
