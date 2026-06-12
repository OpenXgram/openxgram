//! 온체인 USDC 결제 e2e 실검증 — 실제 Ethereum Sepolia 체인에 USDC transfer 1건 제출.
//!
//! `OnchainPaymentGateway` 를 실제 keystore master 키로 생성 → `pay()` 직접 호출 →
//! 반환된 tx_hash 가 실제 on-chain tx 인지 RPC `eth_getTransactionReceipt` 로 검증.
//!
//! ## 활성화 조건 (모두 env 주입)
//!   - `RUN_ONCHAIN=1`                  — #[ignore] 우회 (기본 스킵)
//!   - `XGRAM_ONCHAIN_DATA_DIR`         — keystore(master.json) + db.sqlite 위치 (예: ~/.openxgram)
//!   - `XGRAM_KEYSTORE_PASSWORD`        — master 키 복호화 비밀번호 (평문 박지 말 것)
//!   - `XGRAM_CHAIN_RPC`                — 체인 RPC (예: https://ethereum-sepolia-rpc.publicnode.com)
//!   - `XGRAM_ONCHAIN_PAYEE` (선택)     — 수취 주소. 생략 시 0x000…dEaD burn 주소.
//!   - `XGRAM_ONCHAIN_AMOUNT_MICRO`(선택) — 전송액 micro-USDC. 생략 시 100000 (=0.1 USDC).
//!   - `XGRAM_CHAIN` (선택)             — chain name. 생략 시 "ethereum-sepolia".
//!
//! ## 실행
//! ```
//! RUN_ONCHAIN=1 \
//!   XGRAM_ONCHAIN_DATA_DIR=$HOME/.openxgram \
//!   XGRAM_KEYSTORE_PASSWORD=*** \
//!   XGRAM_CHAIN_RPC=https://ethereum-sepolia-rpc.publicnode.com \
//!   cargo test -p openxgram-cli --test onchain_pay --release -- --ignored --nocapture
//! ```
//!
//! ## 가짜 성공 금지
//!   - 자금 부족·RPC 오류·키 로드 실패 → `pay()` 가 Err 반환 → 테스트 panic (실패 그대로 노출).
//!   - 반환 tx_hash 가 0x 로 시작하고 66자(0x + 64 hex)인지 확인.

use std::path::PathBuf;

use openxgram_cli::onchain_gateway::OnchainPaymentGateway;
use openxgram_marketplace::{AgentId, PaymentGateway};

fn enabled() -> bool {
    std::env::var("RUN_ONCHAIN").as_deref() == Ok("1")
}

#[tokio::test]
#[ignore = "실제 Ethereum Sepolia USDC transfer — RUN_ONCHAIN=1 + env 주입으로 활성화"]
async fn onchain_usdc_transfer_real_tx() {
    if !enabled() {
        eprintln!("[skip] RUN_ONCHAIN!=1 — 온체인 실검증 비활성");
        return;
    }

    let data_dir = PathBuf::from(
        std::env::var("XGRAM_ONCHAIN_DATA_DIR")
            .expect("XGRAM_ONCHAIN_DATA_DIR 필요 (keystore+db 위치)"),
    );
    let vault_password = std::env::var("XGRAM_KEYSTORE_PASSWORD")
        .expect("XGRAM_KEYSTORE_PASSWORD 필요 (master 키 복호화)");
    let rpc_url = std::env::var("XGRAM_CHAIN_RPC").expect("XGRAM_CHAIN_RPC 필요");
    let chain = std::env::var("XGRAM_CHAIN").unwrap_or_else(|_| "ethereum-sepolia".to_string());
    let payee = std::env::var("XGRAM_ONCHAIN_PAYEE")
        .unwrap_or_else(|_| "0x000000000000000000000000000000000000dEaD".to_string());
    let amount_micro: i64 = std::env::var("XGRAM_ONCHAIN_AMOUNT_MICRO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000); // 0.1 USDC

    let db_path = openxgram_core::paths::db_path(&data_dir);

    eprintln!("=== 온체인 USDC 결제 e2e ===");
    eprintln!("data_dir : {}", data_dir.display());
    eprintln!("chain    : {chain}");
    eprintln!("payee    : {payee}");
    eprintln!("amount   : {amount_micro} micro-USDC (= {} USDC)", amount_micro as f64 / 1e6);

    let gw = OnchainPaymentGateway::open(data_dir, db_path, rpc_url, vault_password)
        .expect("OnchainPaymentGateway::open 실패");

    let agent = AgentId("agent:onchain-e2e-test".to_string());

    let receipt = gw
        .pay(
            &agent,
            amount_micro,
            &chain,
            &payee,
            Some("openxgram onchain e2e verification"),
        )
        .await
        .expect("pay() 실패 — 실제 온체인 제출 에러 (자금부족/nonce/RPC). 가짜 성공 금지.");

    let tx_hash = receipt
        .tx_hash
        .expect("tx_hash 없음 — 온체인 게이트웨이가 실제 tx 를 반환하지 않음");

    eprintln!("=== 결과 ===");
    eprintln!("tx_hash  : {tx_hash}");
    eprintln!("intent_id: {}", receipt.intent_id);
    eprintln!("etherscan: https://sepolia.etherscan.io/tx/{tx_hash}");

    assert!(tx_hash.starts_with("0x"), "tx_hash 가 0x 로 시작하지 않음: {tx_hash}");
    assert_eq!(tx_hash.len(), 66, "tx_hash 길이가 66 (0x+64hex) 이 아님: {tx_hash}");
    assert_eq!(receipt.amount_usdc_micro, amount_micro);
    assert_eq!(receipt.chain, chain);
}
