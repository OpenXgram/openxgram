//! alloy 통합 submit / 에러 분류 / confirmation watcher / RBF (PRD-PAY-02 ~ 06).
//!
//! 흐름:
//!   1. sol! IERC20 으로 transfer call data 빌드 (compile-time ABI)
//!   2. ProviderBuilder.with_recommended_fillers + wallet — TransactionRequest 빌드
//!   3. send_raw_transaction → tx_hash 반환 (idempotency key)
//!   4. eth_getTransactionReceipt 폴링 — 5블록 soft / 64블록 final
//!   5. RBF: 동일 nonce + tip +15%
//!
//! 절대 규칙:
//! - silent fallback 금지 — RPC primary/secondary 전환 시 명시 로그
//! - idempotent — 동일 (from, nonce, chain_id) 의 RBF 만 허용

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, B256, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;

use crate::{PaymentError, Result};

// USDC 등 ERC-20 표준 인터페이스. compile-time ABI.
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }
}

/// RPC primary + ordered fallback URL list.
#[derive(Debug, Clone)]
pub struct RpcConfig {
    pub urls: Vec<String>,
}

impl RpcConfig {
    /// Base mainnet 기본 — Coinbase → Alchemy → LlamaRPC. 환경변수 override 가능.
    /// XGRAM_BASE_RPC_PRIMARY / SECONDARY / TERTIARY
    pub fn base_mainnet_default() -> Self {
        let primary = std::env::var("XGRAM_BASE_RPC_PRIMARY")
            .unwrap_or_else(|_| "https://mainnet.base.org".to_string());
        let secondary = std::env::var("XGRAM_BASE_RPC_SECONDARY").ok();
        let tertiary = std::env::var("XGRAM_BASE_RPC_TERTIARY").ok();
        let mut urls = vec![primary];
        if let Some(s) = secondary {
            if !s.is_empty() {
                urls.push(s);
            }
        }
        if let Some(t) = tertiary {
            if !t.is_empty() {
                urls.push(t);
            }
        }
        Self { urls }
    }

    /// Base Sepolia 테스트넷.
    pub fn base_sepolia_default() -> Self {
        let primary = std::env::var("XGRAM_BASE_SEPOLIA_RPC")
            .unwrap_or_else(|_| "https://sepolia.base.org".to_string());
        Self {
            urls: vec![primary],
        }
    }
}

/// submit 결과 — chain 응답 분류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// 성공 — tx_hash 산출, status='submitted' 로 진행
    Submitted(String),
    /// 동일 nonce 가 이미 chain 에 commit — receipt 조회로 confirmed/failed 판별 필요
    NonceTooLow,
    /// 동일 nonce 다른 tx 가 mempool 에 있음 — RBF 필요 (tip +12.5% 이상)
    ReplacementUnderpriced,
    /// timeout 이나 일시적 RPC 장애 — draft → signed 로 재시도 가능
    TransientError(String),
    /// 영구 실패 — chain rejected (insufficient funds 등)
    PermanentError(String),
}

/// alloy provider 에러 → SubmitOutcome 분류.
pub fn classify_submit_error<E: std::fmt::Display>(err: E) -> SubmitOutcome {
    let msg = err.to_string().to_lowercase();
    if msg.contains("nonce too low") || msg.contains("nonce_too_low") {
        SubmitOutcome::NonceTooLow
    } else if msg.contains("replacement transaction underpriced") || msg.contains("underpriced") {
        SubmitOutcome::ReplacementUnderpriced
    } else if msg.contains("timeout") || msg.contains("connection") || msg.contains("eof") {
        SubmitOutcome::TransientError(err.to_string())
    } else {
        SubmitOutcome::PermanentError(err.to_string())
    }
}

/// RBF — 동일 nonce 의 새 attempt 에 대한 tip rebump 계산.
/// 최소 +12.5% (DOS 룰), 안전 마진 +15%.
pub const RBF_BUMP_FACTOR_NUMERATOR: u128 = 115;
pub const RBF_BUMP_FACTOR_DENOMINATOR: u128 = 100;

pub fn rbf_bump(prev_max_priority_wei: u128) -> u128 {
    prev_max_priority_wei * RBF_BUMP_FACTOR_NUMERATOR / RBF_BUMP_FACTOR_DENOMINATOR
}

/// confirmation 임계치.
pub const SOFT_CONFIRM_BLOCKS: u64 = 5;
pub const FINAL_CONFIRM_BLOCKS: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationStatus {
    NotMined,
    SoftConfirmed,
    Final,
    /// receipt 가 사라지면 reorg — submitted 회귀
    Reorg,
}

pub fn confirmation_from_blocks(receipt_block: Option<u64>, head_block: u64) -> ConfirmationStatus {
    match receipt_block {
        None => ConfirmationStatus::NotMined,
        Some(b) if head_block < b => ConfirmationStatus::Reorg,
        Some(b) if head_block - b >= FINAL_CONFIRM_BLOCKS => ConfirmationStatus::Final,
        Some(b) if head_block - b >= SOFT_CONFIRM_BLOCKS => ConfirmationStatus::SoftConfirmed,
        Some(_) => ConfirmationStatus::NotMined,
    }
}

/// USDC transfer 빌더 — sol! 매크로로 ABI 인코딩.
pub fn build_usdc_transfer(
    usdc_contract: Address,
    to: Address,
    amount_micro: u64,
) -> TransactionRequest {
    let amount = U256::from(amount_micro);
    let call = IERC20::transferCall { to, amount };
    let data = alloy::sol_types::SolCall::abi_encode(&call);
    TransactionRequest::default()
        .to(usdc_contract)
        .input(data.into())
}

/// signer + RPC URL → connected provider.
pub async fn connect_provider(
    rpc_url: &str,
    signer: PrivateKeySigner,
) -> Result<impl Provider + Clone> {
    let wallet = EthereumWallet::from(signer);
    let url = rpc_url
        .parse()
        .map_err(|e| PaymentError::InvalidAmount(format!("RPC URL 파싱: {e}")))?;
    let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);
    Ok(provider)
}

/// (low-level) signed tx (RLP) 를 RPC 에 전달. SubmitOutcome 분류.
pub async fn send_raw(provider: &impl Provider, raw_rlp: &[u8]) -> SubmitOutcome {
    match provider.send_raw_transaction(raw_rlp).await {
        Ok(pending) => {
            let tx_hash: B256 = *pending.tx_hash();
            SubmitOutcome::Submitted(format!("0x{}", hex::encode(tx_hash.as_slice())))
        }
        Err(e) => classify_submit_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    #[test]
    fn classify_nonce_too_low() {
        let o = classify_submit_error("nonce too low: have 1 want 0");
        assert!(matches!(o, SubmitOutcome::NonceTooLow));
    }

    #[test]
    fn classify_replacement_underpriced() {
        let o = classify_submit_error("replacement transaction underpriced");
        assert!(matches!(o, SubmitOutcome::ReplacementUnderpriced));
    }

    #[test]
    fn classify_timeout_is_transient() {
        let o = classify_submit_error("timeout while waiting");
        assert!(matches!(o, SubmitOutcome::TransientError(_)));
    }

    #[test]
    fn classify_unknown_is_permanent() {
        let o = classify_submit_error("insufficient funds for gas * price + value");
        assert!(matches!(o, SubmitOutcome::PermanentError(_)));
    }

    #[test]
    fn rbf_bump_15_percent() {
        assert_eq!(rbf_bump(1_000_000), 1_150_000);
        assert_eq!(rbf_bump(1_000_000_000_000_000), 1_150_000_000_000_000);
    }

    #[test]
    fn confirmation_states_per_block_distance() {
        assert_eq!(confirmation_from_blocks(None, 100), ConfirmationStatus::NotMined);
        assert_eq!(confirmation_from_blocks(Some(100), 99), ConfirmationStatus::Reorg);
        assert_eq!(confirmation_from_blocks(Some(100), 100), ConfirmationStatus::NotMined);
        assert_eq!(confirmation_from_blocks(Some(100), 104), ConfirmationStatus::NotMined);
        assert_eq!(
            confirmation_from_blocks(Some(100), 105),
            ConfirmationStatus::SoftConfirmed
        );
        assert_eq!(
            confirmation_from_blocks(Some(100), 163),
            ConfirmationStatus::SoftConfirmed
        );
        assert_eq!(
            confirmation_from_blocks(Some(100), 164),
            ConfirmationStatus::Final
        );
    }

    #[test]
    fn build_usdc_transfer_encodes_correctly() {
        // USDC on Base
        let usdc = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
        let to = address!("AAAAaaaaAAAAaaaaAAAAaaaaAAAAaaaaAAAAaaaa");
        let req = build_usdc_transfer(usdc, to, 1_000_000); // 1 USDC
        assert_eq!(req.to, Some(usdc.into()));
        let input = req.input.input.expect("input data");
        // selector = 0xa9059cbb
        assert_eq!(&input[..4], &[0xa9, 0x05, 0x9c, 0xbb]);
        // total len = 4 + 32 + 32
        assert_eq!(input.len(), 68);
    }

    #[test]
    fn rpc_config_env_override() {
        std::env::set_var("XGRAM_BASE_RPC_PRIMARY", "https://primary.test");
        std::env::set_var("XGRAM_BASE_RPC_SECONDARY", "https://secondary.test");
        std::env::remove_var("XGRAM_BASE_RPC_TERTIARY");
        let cfg = RpcConfig::base_mainnet_default();
        assert_eq!(cfg.urls.len(), 2);
        assert_eq!(cfg.urls[0], "https://primary.test");
        assert_eq!(cfg.urls[1], "https://secondary.test");
        std::env::remove_var("XGRAM_BASE_RPC_PRIMARY");
        std::env::remove_var("XGRAM_BASE_RPC_SECONDARY");
    }

    #[test]
    fn rpc_config_default_when_no_env() {
        std::env::remove_var("XGRAM_BASE_RPC_PRIMARY");
        std::env::remove_var("XGRAM_BASE_RPC_SECONDARY");
        std::env::remove_var("XGRAM_BASE_RPC_TERTIARY");
        let cfg = RpcConfig::base_mainnet_default();
        assert_eq!(cfg.urls.len(), 1);
        assert_eq!(cfg.urls[0], "https://mainnet.base.org");
    }
}
