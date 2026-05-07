//! 일회성 ad-hoc — Sepolia L1 → Base Sepolia L2 ETH 브릿지.
//!
//! 마스터 키로 Sepolia L1에서 Base Sepolia OptimismPortal proxy 로 plain ETH 전송.
//! Portal 의 receive() 가 자동으로 동일 주소(L2)에 ETH 입금. ~3분 후 Base Sepolia 도착.
//!
//! 실행:
//!   XGRAM_KEYSTORE_PASSWORD=demo-password \
//!     cargo run --example bridge_eth_sepolia_to_base -- \
//!       --data-dir /tmp/xgram-A --amount-eth 0.05
//!
//! 환경변수 override:
//!   XGRAM_SEPOLIA_RPC (기본 https://ethereum-sepolia-rpc.publicnode.com)
//!
//! 데모용 — 운영 코드 아님. 한 번 쓰고 폐기.

use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use alloy::network::TransactionBuilder;
use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_payment::alloy_bridge::wallet_from_master;

/// Base Sepolia OptimismPortal proxy (Sepolia L1) — 공식 base 문서 검증.
const BASE_SEPOLIA_OPTIMISM_PORTAL: &str = "0x49f53e41452C74589E85cA1677426Ba426459e85";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Args 파싱 — 단순 long-flag.
    let args: Vec<String> = env::args().collect();
    let mut data_dir: Option<PathBuf> = None;
    let mut amount_eth: Option<f64> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                data_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--amount-eth" => {
                amount_eth = Some(args[i + 1].parse()?);
                i += 2;
            }
            other => anyhow::bail!("unknown arg: {other}"),
        }
    }
    let data_dir = data_dir.ok_or_else(|| anyhow::anyhow!("--data-dir 필수"))?;
    let amount_eth = amount_eth.ok_or_else(|| anyhow::anyhow!("--amount-eth 필수"))?;
    let pw = env::var("XGRAM_KEYSTORE_PASSWORD")
        .map_err(|_| anyhow::anyhow!("XGRAM_KEYSTORE_PASSWORD env 필요"))?;

    // Keystore 로드.
    let ks_dir = data_dir.join("keystore");
    let ks = FsKeystore::new(&ks_dir);
    let master = ks.load("master", &pw)?;
    let from_addr = master.address.0.clone();
    println!("from   : {from_addr}");

    // Wallet + provider.
    let wallet = wallet_from_master(&master)?;
    let rpc = env::var("XGRAM_SEPOLIA_RPC")
        .unwrap_or_else(|_| "https://ethereum-sepolia-rpc.publicnode.com".to_string());
    println!("rpc    : {rpc}");
    let url = rpc.parse()?;
    let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

    // Plain ETH transfer 빌드 — value = wei.
    let portal: Address = BASE_SEPOLIA_OPTIMISM_PORTAL.parse()?;
    let wei: u128 = (amount_eth * 1e18) as u128;
    let value = U256::from(wei);
    println!("to     : {portal} (Base Sepolia OptimismPortal)");
    println!("amount : {amount_eth} ETH ({wei} wei)");

    let req = TransactionRequest::default()
        .with_to(portal)
        .with_value(value)
        // Sepolia L1 chain_id = 11155111
        .with_chain_id(11155111);

    let pending = provider
        .send_transaction(req)
        .await
        .map_err(|e| anyhow::anyhow!("RPC send_transaction 실패: {e}"))?;
    let tx_hash = format!("0x{}", hex::encode(pending.tx_hash().as_slice()));
    println!();
    println!("✓ 브릿지 트랜잭션 송신 완료");
    println!("  L1 tx_hash : {tx_hash}");
    println!("  L1 explorer: https://sepolia.etherscan.io/tx/{tx_hash}");
    println!();
    println!("약 3분 후 Base Sepolia 에서 동일 주소가 ETH 보유.");
    println!("  L2 explorer: https://sepolia.basescan.org/address/{from_addr}");

    let _ = Address::from_str(&from_addr); // silence unused
    Ok(())
}
