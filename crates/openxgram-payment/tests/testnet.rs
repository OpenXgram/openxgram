//! Base Sepolia testnet 통합 테스트 (PRD-PAY-08).
//!
//! 활성화 조건: 환경변수 `RUN_TESTNET=1` + `XGRAM_TESTNET_PASSWORD` (master 키 로드용)
//! 비활성 시: #[ignore] 으로 cargo test 가 자동 스킵
//!
//! 실행: RUN_TESTNET=1 cargo test --test testnet --release -- --ignored --nocapture

use openxgram_payment::alloy_bridge::signer_from_master;
use openxgram_payment::submit::{
    confirmation_from_blocks, ConfirmationStatus, RpcConfig, FINAL_CONFIRM_BLOCKS,
    SOFT_CONFIRM_BLOCKS,
};

fn testnet_enabled() -> bool {
    std::env::var("RUN_TESTNET").as_deref() == Ok("1")
}

#[tokio::test]
#[ignore = "Base Sepolia testnet 필요 — RUN_TESTNET=1 로 활성화"]
async fn base_sepolia_signer_address_matches_chain() {
    if !testnet_enabled() {
        return;
    }
    use openxgram_keystore::{FsKeystore, Keystore};
    let tmp = tempfile::tempdir().unwrap();
    let ks = FsKeystore::new(tmp.path());
    ks.create("testnet", "pw").unwrap();
    let master = ks.load("testnet", "pw").unwrap();
    let signer = signer_from_master(&master).unwrap();
    let addr = signer.address();
    eprintln!("testnet signer address: {addr:?}");
    eprintln!("master.address: {}", master.address);
    // 동일 secp256k1 — 두 주소 케이스 인센서티브 비교
    let m = master.address.to_string();
    let a = format!("{addr:?}");
    assert!(
        a.to_lowercase().contains(&m.to_lowercase()[2..])
            || m.to_lowercase().contains(&a.to_lowercase()[2..]),
        "주소 불일치: master={m} signer={a}"
    );
}

#[tokio::test]
#[ignore = "Base Sepolia testnet 필요 — RUN_TESTNET=1 로 활성화"]
async fn base_sepolia_rpc_config_loads() {
    if !testnet_enabled() {
        return;
    }
    let cfg = RpcConfig::base_sepolia_default();
    assert!(!cfg.urls.is_empty(), "Base Sepolia RPC URL 비어있음");
    eprintln!("Base Sepolia RPCs: {:?}", cfg.urls);
}

#[test]
fn confirmation_status_thresholds_consistent() {
    // Base reorg 안전선이 PRD §2.3 와 일치하는지 sanity check
    assert_eq!(SOFT_CONFIRM_BLOCKS, 5, "soft 5블록 (~10초)");
    assert_eq!(FINAL_CONFIRM_BLOCKS, 64, "final 64블록 (~2분)");

    // 5 블록 soft, 64 블록 final 경계 검증 (testnet 의존 X)
    assert_eq!(
        confirmation_from_blocks(Some(0), 4),
        ConfirmationStatus::NotMined
    );
    assert_eq!(
        confirmation_from_blocks(Some(0), 5),
        ConfirmationStatus::SoftConfirmed
    );
    assert_eq!(
        confirmation_from_blocks(Some(0), 64),
        ConfirmationStatus::Final
    );
}
