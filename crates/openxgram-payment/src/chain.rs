//! Supported chains + USDC contract 주소 (PRD §16 마스터 결정 — Base 우선).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainConfig {
    pub name: &'static str,
    pub chain_id: u64,
    pub usdc_contract: &'static str,
    pub default_rpc: &'static str,
}

/// Base L2 (PRD §16 우선) — Coinbase 발행 USDC, 낮은 가스, 빠른 확정.
pub const BASE: ChainConfig = ChainConfig {
    name: "base",
    chain_id: 8453,
    usdc_contract: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
    default_rpc: "https://mainnet.base.org",
};

/// Polygon PoS — 대안.
pub const POLYGON: ChainConfig = ChainConfig {
    name: "polygon",
    chain_id: 137,
    usdc_contract: "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359",
    default_rpc: "https://polygon-rpc.com",
};

/// Ethereum mainnet — 비싼 가스, 일반적으로 비추천.
pub const ETHEREUM: ChainConfig = ChainConfig {
    name: "ethereum",
    chain_id: 1,
    usdc_contract: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    default_rpc: "https://eth.llamarpc.com",
};

pub const ALL: &[ChainConfig] = &[BASE, POLYGON, ETHEREUM];

pub fn lookup(name: &str) -> Option<ChainConfig> {
    ALL.iter().copied().find(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_returns_known_chains() {
        assert_eq!(lookup("base").unwrap().chain_id, 8453);
        assert_eq!(lookup("polygon").unwrap().chain_id, 137);
        assert_eq!(lookup("ethereum").unwrap().chain_id, 1);
        assert!(lookup("nonexistent").is_none());
    }

    #[test]
    fn usdc_contracts_are_42_chars() {
        for c in ALL {
            assert_eq!(c.usdc_contract.len(), 42, "addr: {}", c.usdc_contract);
            assert!(c.usdc_contract.starts_with("0x"));
        }
    }
}
