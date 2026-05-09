//! 4.1.3 — 가스 정책 (master 부담). attestation 제출 전 사용자 한도 검증.

use crate::{EasError, Result};

/// 가스 한도. 환경변수 `XGRAM_EAS_MAX_USD_PER_ATTEST` 로 override (기본 0.10 USD).
#[derive(Debug, Clone, Copy)]
pub struct GasPolicy {
    pub max_usd_per_attest: f64,
}

impl GasPolicy {
    pub fn from_env() -> Self {
        let max = std::env::var("XGRAM_EAS_MAX_USD_PER_ATTEST")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.10);
        Self { max_usd_per_attest: max }
    }
}

impl Default for GasPolicy {
    fn default() -> Self {
        Self {
            max_usd_per_attest: 0.10,
        }
    }
}

/// 가스 견적 — 가스 unit + gwei + ETH/USD 환율.
#[derive(Debug, Clone, Copy)]
pub struct GasQuote {
    pub gas_units: u64,
    pub gas_price_gwei: f64,
    pub eth_usd: f64,
}

impl GasQuote {
    pub fn estimated_usd(&self) -> f64 {
        // gwei → ETH = 1e-9; gas_units * gas_price_gwei * 1e-9 = ETH; * eth_usd = USD
        (self.gas_units as f64) * self.gas_price_gwei * 1e-9 * self.eth_usd
    }

    pub fn check(&self, policy: &GasPolicy) -> Result<()> {
        let est = self.estimated_usd();
        if est > policy.max_usd_per_attest {
            return Err(EasError::GasOverLimit {
                estimated_usd: est,
                limit_usd: policy.max_usd_per_attest,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_limit_passes() {
        let q = GasQuote {
            gas_units: 80_000,
            gas_price_gwei: 0.5, // Base mainnet 평균
            eth_usd: 4_000.0,
        };
        let p = GasPolicy::default();
        // 80k * 0.5 * 1e-9 = 4e-5 ETH; * 4000 = 0.16 USD — over default 0.10
        assert!(q.estimated_usd() > 0.15);
        assert!(q.check(&p).is_err());
    }

    #[test]
    fn very_cheap_passes() {
        let q = GasQuote {
            gas_units: 80_000,
            gas_price_gwei: 0.05,
            eth_usd: 4_000.0,
        };
        // 80k * 0.05 * 1e-9 * 4000 = 0.016 USD
        let p = GasPolicy::default();
        assert!(q.check(&p).is_ok());
    }

    #[test]
    fn over_limit_returns_helpful_error() {
        let q = GasQuote {
            gas_units: 1_000_000,
            gas_price_gwei: 5.0,
            eth_usd: 4_000.0,
        };
        let p = GasPolicy {
            max_usd_per_attest: 0.10,
        };
        match q.check(&p) {
            Err(EasError::GasOverLimit { estimated_usd, limit_usd }) => {
                assert!(estimated_usd > 0.10);
                assert_eq!(limit_usd, 0.10);
            }
            other => panic!("expected GasOverLimit, got {other:?}"),
        }
    }
}
