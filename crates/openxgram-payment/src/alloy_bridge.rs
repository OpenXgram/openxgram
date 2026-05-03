//! alloy 통합 — master Keypair → PrivateKeySigner, ChainConfig → NamedChain (PRD-PAY-01).

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, B256};
use alloy::signers::local::PrivateKeySigner;
use openxgram_keystore::Keypair;

use crate::{PaymentError, Result};

/// master Keypair (k256 SecretKey) → alloy PrivateKeySigner.
/// 동일 secp256k1 secret 으로 EVM 서명도 동일 주소 산출.
pub fn signer_from_master(master: &Keypair) -> Result<PrivateKeySigner> {
    let secret_bytes = master.secret_key_bytes();
    let b256 = B256::from_slice(&secret_bytes);
    PrivateKeySigner::from_bytes(&b256)
        .map_err(|e| PaymentError::InvalidAmount(format!("alloy signer init: {e}")))
}

/// EthereumWallet 변환 — alloy ProviderBuilder.wallet 에 그대로 전달 가능.
pub fn wallet_from_master(master: &Keypair) -> Result<EthereumWallet> {
    let signer = signer_from_master(master)?;
    Ok(EthereumWallet::from(signer))
}

/// master 의 EVM checksum 주소 (0x prefix, 42자).
pub fn master_eth_address(master: &Keypair) -> Result<Address> {
    let signer = signer_from_master(master)?;
    Ok(signer.address())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_keystore::{FsKeystore, Keystore};
    use tempfile::tempdir;

    fn make_master() -> Keypair {
        let tmp = tempdir().unwrap();
        let ks = FsKeystore::new(tmp.path());
        let _ = ks.create("alloy-test", "pw").unwrap();
        ks.load("alloy-test", "pw").unwrap()
    }

    #[test]
    fn signer_address_matches_master_eth_address() {
        let m = make_master();
        let alloy_addr = master_eth_address(&m).unwrap();
        // alloy 는 EIP-55 checksum, master.address 도 EIP-55 → 직접 비교 (대문자 차이 가능 → ignore_ascii_case)
        let master_addr_str = m.address.to_string();
        let alloy_str = alloy_addr.to_string();
        assert!(
            master_addr_str.eq_ignore_ascii_case(&alloy_str),
            "주소 불일치: master={master_addr_str} alloy={alloy_str}"
        );
    }

    #[test]
    fn wallet_from_master_round_trip() {
        let m = make_master();
        let _wallet = wallet_from_master(&m).unwrap();
        // 단순 init 성공 — 실제 서명은 통합 테스트
    }

    #[test]
    fn signer_idempotent_for_same_master() {
        let m = make_master();
        let s1 = signer_from_master(&m).unwrap();
        let s2 = signer_from_master(&m).unwrap();
        assert_eq!(s1.address(), s2.address());
    }
}
