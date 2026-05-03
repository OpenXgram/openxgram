//! ERC-20 transfer call data 인코딩.
//!
//! USDC 등 모든 표준 ERC-20 토큰의 `transfer(address,uint256)` 함수 호출 데이터를
//! 32-byte aligned ABI 형식으로 빌드. RPC 통합 (alloy/ethers) 후속 PR 의 baseline.
//!
//! `transfer(address,uint256)` 의 4-byte function selector 는 standard:
//!   keccak256("transfer(address,uint256)")[..4] = 0xa9059cbb
//! ERC-20 명세의 일부로 모든 호환 토큰에서 동일.

use crate::{PaymentError, Result};

/// `transfer(address,uint256)` selector — 4 bytes.
pub const TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

/// 0x... 주소 hex (40자) → 20 bytes. 0x prefix 옵션. 검증: 길이 + hex.
pub fn parse_eth_address(s: &str) -> Result<[u8; 20]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 40 {
        return Err(PaymentError::InvalidAmount(format!(
            "ETH address 는 40자 hex (got {} chars)",
            s.len()
        )));
    }
    let bytes = hex::decode(s).map_err(PaymentError::Hex)?;
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

/// USDC transfer call data — 4-byte selector + 32-byte padded address + 32-byte amount.
/// 총 4 + 32 + 32 = 68 bytes.
pub fn encode_transfer(payee: &str, amount_micro: i64) -> Result<Vec<u8>> {
    if amount_micro < 0 {
        return Err(PaymentError::InvalidAmount(format!(
            "amount must be >= 0 (got {amount_micro})"
        )));
    }
    let payee_bytes = parse_eth_address(payee)?;

    let mut out = Vec::with_capacity(68);
    out.extend_from_slice(&TRANSFER_SELECTOR);

    // address — 32-byte left-padded with 12 zero bytes
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&payee_bytes);

    // amount — 32-byte big-endian (uint256). i64 → 24 zero bytes + 8 bytes BE.
    out.extend_from_slice(&[0u8; 24]);
    out.extend_from_slice(&(amount_micro as u64).to_be_bytes());

    Ok(out)
}

/// 사람용 hex 출력 — "0xa9059cbb..." (총 138자: 0x + 136자 hex).
pub fn encode_transfer_hex(payee: &str, amount_micro: i64) -> Result<String> {
    let bytes = encode_transfer(payee, amount_micro)?;
    Ok(format!("0x{}", hex::encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_is_well_known() {
        // ERC-20 spec 표준 — bytes4(keccak256("transfer(address,uint256)"))
        assert_eq!(TRANSFER_SELECTOR, [0xa9, 0x05, 0x9c, 0xbb]);
    }

    #[test]
    fn parse_eth_address_valid() {
        let addr = parse_eth_address("0xa0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap();
        assert_eq!(addr.len(), 20);
        // Ethereum mainnet USDC — 첫 바이트 0xa0
        assert_eq!(addr[0], 0xa0);
        assert_eq!(addr[19], 0x48);
    }

    #[test]
    fn parse_eth_address_no_prefix() {
        let addr = parse_eth_address("a0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap();
        assert_eq!(addr[0], 0xa0);
    }

    #[test]
    fn parse_eth_address_wrong_length() {
        assert!(parse_eth_address("0xshort").is_err());
        assert!(parse_eth_address("0x12").is_err());
    }

    #[test]
    fn encode_transfer_known_vector() {
        // 1 USDC (1_000_000 micro) → 0xa0...USDC contract recipient
        let out = encode_transfer("0x000000000000000000000000000000000000dEaD", 1_000_000).unwrap();
        assert_eq!(out.len(), 68);
        // selector
        assert_eq!(&out[0..4], &TRANSFER_SELECTOR);
        // address — 마지막 20 byte
        assert_eq!(
            &out[16..36],
            &hex::decode("000000000000000000000000000000000000dEaD").unwrap()[..]
        );
        // amount — 32-byte BE, 1_000_000 = 0x0F4240
        let amount_bytes = &out[36..68];
        // 마지막 4바이트가 0x000F4240
        assert_eq!(&amount_bytes[28..32], &[0x00, 0x0f, 0x42, 0x40]);
    }

    #[test]
    fn encode_transfer_hex_format() {
        let h = encode_transfer_hex("0x000000000000000000000000000000000000dEaD", 1).unwrap();
        assert!(h.starts_with("0xa9059cbb"));
        // 0x + 136 hex chars = 138
        assert_eq!(h.len(), 138);
    }

    #[test]
    fn encode_transfer_zero_amount_ok() {
        // ERC-20 spec: zero transfer 는 valid event 발행. 거부 안 함.
        let out = encode_transfer("0x000000000000000000000000000000000000dEaD", 0).unwrap();
        assert_eq!(out[36..68], [0u8; 32]);
    }

    #[test]
    fn encode_transfer_negative_amount_rejected() {
        assert!(encode_transfer("0x000000000000000000000000000000000000dEaD", -1).is_err());
    }
}
