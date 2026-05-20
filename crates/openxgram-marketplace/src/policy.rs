//! 결제 정책 (보수적 — PRD §9 결정 4) + 게이트웨이 trait.
//!
//! 한도:
//!   - per_tx_usdc_micro    : 1회 한도 (default $0.50 = 500_000)
//!   - hourly_usdc_micro    : 시간당 (default $2 = 2_000_000)
//!   - daily_usdc_micro     : 일별 (default $10 = 10_000_000)
//!   - monthly_usdc_micro   : 월별 (default $50 = 50_000_000)
//!   - whitelist_required   : 화이트리스트 필수 (default true)
//!
//! 한도 초과·미화이트리스트 → `SpendPolicyDecision::RequireConfirm`.
//!
//! `PaymentGateway` trait는 실제 결제 실행을 추상화 — 테스트는 mock, 프로덕션은
//! openxgram-vault + openxgram-payment 연동 구현체를 주입.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;

use crate::agent::AgentId;

/// 보수적 기본값.
pub const DEFAULT_PER_TX_USDC_MICRO: i64 = 500_000; // $0.50
/// 시간당.
pub const DEFAULT_HOURLY_USDC_MICRO: i64 = 2_000_000; // $2
/// 일별.
pub const DEFAULT_DAILY_USDC_MICRO: i64 = 10_000_000; // $10
/// 월별.
pub const DEFAULT_MONTHLY_USDC_MICRO: i64 = 50_000_000; // $50

/// 결제 자동화 정책.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendPolicy {
    /// 1회 한도.
    pub per_tx_usdc_micro: i64,
    /// 시간당.
    pub hourly_usdc_micro: i64,
    /// 일별.
    pub daily_usdc_micro: i64,
    /// 월별.
    pub monthly_usdc_micro: i64,
    /// 화이트리스트 필수?
    pub whitelist_required: bool,
    /// 자동 결제 허용 에이전트 화이트리스트.
    pub whitelist: HashSet<String>,
}

impl Default for SpendPolicy {
    fn default() -> Self {
        Self::conservative()
    }
}

impl SpendPolicy {
    /// 보수적 기본 (PRD §9 결정 4).
    pub fn conservative() -> Self {
        Self {
            per_tx_usdc_micro: DEFAULT_PER_TX_USDC_MICRO,
            hourly_usdc_micro: DEFAULT_HOURLY_USDC_MICRO,
            daily_usdc_micro: DEFAULT_DAILY_USDC_MICRO,
            monthly_usdc_micro: DEFAULT_MONTHLY_USDC_MICRO,
            whitelist_required: true,
            whitelist: HashSet::new(),
        }
    }

    /// 화이트리스트에 추가.
    pub fn allow(&mut self, agent: &AgentId) {
        self.whitelist.insert(agent.as_str().to_string());
    }

    /// 정책 평가 — 한도·화이트리스트 검사.
    ///
    /// `spent_today_micro` / `spent_hour_micro` / `spent_month_micro` 는 호출자가
    /// 결제 기록 DB(openxgram-payment의 일일 한도 테이블)에서 조회해 주입.
    pub fn evaluate(
        &self,
        agent: &AgentId,
        amount_micro: i64,
        spent_hour_micro: i64,
        spent_today_micro: i64,
        spent_month_micro: i64,
    ) -> SpendPolicyDecision {
        let mut reasons: Vec<String> = Vec::new();
        if amount_micro <= 0 {
            return SpendPolicyDecision::Reject("amount must be > 0".into());
        }
        if amount_micro > self.per_tx_usdc_micro {
            reasons.push(format!(
                "per-tx limit exceeded: {} > {}",
                crate::agent::format_usdc(amount_micro),
                crate::agent::format_usdc(self.per_tx_usdc_micro)
            ));
        }
        if spent_hour_micro + amount_micro > self.hourly_usdc_micro {
            reasons.push(format!(
                "hourly limit exceeded: {} + {} > {}",
                crate::agent::format_usdc(spent_hour_micro),
                crate::agent::format_usdc(amount_micro),
                crate::agent::format_usdc(self.hourly_usdc_micro)
            ));
        }
        if spent_today_micro + amount_micro > self.daily_usdc_micro {
            reasons.push(format!(
                "daily limit exceeded: {} + {} > {}",
                crate::agent::format_usdc(spent_today_micro),
                crate::agent::format_usdc(amount_micro),
                crate::agent::format_usdc(self.daily_usdc_micro)
            ));
        }
        if spent_month_micro + amount_micro > self.monthly_usdc_micro {
            reasons.push(format!(
                "monthly limit exceeded: {} + {} > {}",
                crate::agent::format_usdc(spent_month_micro),
                crate::agent::format_usdc(amount_micro),
                crate::agent::format_usdc(self.monthly_usdc_micro)
            ));
        }
        if self.whitelist_required && !self.whitelist.contains(agent.as_str()) {
            reasons.push(format!("agent not in whitelist: {}", agent));
        }
        if reasons.is_empty() {
            SpendPolicyDecision::AutoApprove
        } else {
            SpendPolicyDecision::RequireConfirm(reasons.join("; "))
        }
    }
}

/// 정책 평가 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpendPolicyDecision {
    /// 한도 내 + 화이트리스트 → 자동 결제 OK.
    AutoApprove,
    /// 사용자 명시적 승인 필요.
    RequireConfirm(String),
    /// 거부 (입력 잘못 — 음수 등).
    Reject(String),
}

/// 결제 영수증.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentReceipt {
    /// 내부 payment intent id.
    pub intent_id: String,
    /// USDC 마이크로.
    pub amount_usdc_micro: i64,
    /// 체인 (예: "base").
    pub chain: String,
    /// 수취 주소.
    pub payee_address: String,
    /// on-chain tx 해시 (제출 후).
    pub tx_hash: Option<String>,
    /// 메모 (job_id 등).
    pub memo: Option<String>,
}

/// 결제 게이트웨이 추상화.
///
/// 구현체:
/// - 프로덕션: vault에서 USDC 지갑 키 가져와 `openxgram_payment::PaymentStore`로 draft→sign→submit.
/// - 테스트: `NoopPaymentGateway` (영수증만 즉시 반환).
#[async_trait]
pub trait PaymentGateway: Send + Sync {
    /// 작업 결제 — 호출자가 한도·정책 검증을 마쳤다고 가정.
    ///
    /// 반환: 결제 영수증 (성공 시).
    async fn pay(
        &self,
        agent: &AgentId,
        amount_usdc_micro: i64,
        chain: &str,
        payee_address: &str,
        memo: Option<&str>,
    ) -> Result<PaymentReceipt, String>;

    /// 시간당 누적 사용량 (USDC 마이크로).
    async fn spent_last_hour_micro(&self) -> Result<i64, String> {
        Ok(0)
    }

    /// 오늘 누적 사용량.
    async fn spent_today_micro(&self) -> Result<i64, String> {
        Ok(0)
    }

    /// 이번 달 누적 사용량.
    async fn spent_this_month_micro(&self) -> Result<i64, String> {
        Ok(0)
    }
}

/// 테스트용 게이트웨이 — 결제 시도를 메모리에 기록.
pub struct NoopPaymentGateway {
    receipts: Mutex<Vec<PaymentReceipt>>,
}

impl Default for NoopPaymentGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl NoopPaymentGateway {
    /// 신규.
    pub fn new() -> Self {
        Self {
            receipts: Mutex::new(Vec::new()),
        }
    }

    /// 지금까지 발급된 영수증 개수.
    pub fn count(&self) -> usize {
        self.receipts.lock().unwrap().len()
    }

    /// 총 누적 USDC 마이크로.
    pub fn total_spent_micro(&self) -> i64 {
        self.receipts
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.amount_usdc_micro)
            .sum()
    }
}

#[async_trait]
impl PaymentGateway for NoopPaymentGateway {
    async fn pay(
        &self,
        _agent: &AgentId,
        amount_usdc_micro: i64,
        chain: &str,
        payee_address: &str,
        memo: Option<&str>,
    ) -> Result<PaymentReceipt, String> {
        let r = PaymentReceipt {
            intent_id: uuid::Uuid::new_v4().to_string(),
            amount_usdc_micro,
            chain: chain.to_string(),
            payee_address: payee_address.to_string(),
            tx_hash: Some(format!("0x_test_{}", uuid::Uuid::new_v4().simple())),
            memo: memo.map(str::to_string),
        };
        self.receipts.lock().unwrap().push(r.clone());
        Ok(r)
    }

    async fn spent_today_micro(&self) -> Result<i64, String> {
        Ok(self.total_spent_micro())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(s: &str) -> AgentId {
        AgentId(s.into())
    }

    #[test]
    fn conservative_defaults() {
        let p = SpendPolicy::default();
        assert_eq!(p.per_tx_usdc_micro, 500_000);
        assert!(p.whitelist_required);
    }

    #[test]
    fn within_limits_and_whitelisted_auto_approves() {
        let mut p = SpendPolicy::default();
        let a = aid("agent:trusted");
        p.allow(&a);
        let d = p.evaluate(&a, 100_000, 0, 0, 0);
        assert_eq!(d, SpendPolicyDecision::AutoApprove);
    }

    #[test]
    fn over_per_tx_requires_confirm() {
        let mut p = SpendPolicy::default();
        let a = aid("agent:trusted");
        p.allow(&a);
        match p.evaluate(&a, 600_000, 0, 0, 0) {
            SpendPolicyDecision::RequireConfirm(reason) => {
                assert!(reason.contains("per-tx"));
            }
            other => panic!("expected RequireConfirm, got {other:?}"),
        }
    }

    #[test]
    fn over_daily_requires_confirm() {
        let mut p = SpendPolicy::default();
        let a = aid("agent:trusted");
        p.allow(&a);
        match p.evaluate(&a, 100_000, 0, 9_950_000, 0) {
            SpendPolicyDecision::RequireConfirm(reason) => {
                assert!(reason.contains("daily"));
            }
            other => panic!("expected RequireConfirm, got {other:?}"),
        }
    }

    #[test]
    fn non_whitelisted_requires_confirm() {
        let p = SpendPolicy::default();
        match p.evaluate(&aid("agent:unknown"), 100_000, 0, 0, 0) {
            SpendPolicyDecision::RequireConfirm(reason) => {
                assert!(reason.contains("whitelist"));
            }
            other => panic!("expected RequireConfirm, got {other:?}"),
        }
    }

    #[test]
    fn whitelist_disabled_allows_any() {
        let p = SpendPolicy {
            whitelist_required: false,
            ..Default::default()
        };
        let d = p.evaluate(&aid("agent:anyone"), 100_000, 0, 0, 0);
        assert_eq!(d, SpendPolicyDecision::AutoApprove);
    }

    #[test]
    fn negative_amount_rejected() {
        let p = SpendPolicy::default();
        match p.evaluate(&aid("a"), -1, 0, 0, 0) {
            SpendPolicyDecision::Reject(_) => {}
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn noop_gateway_records_payments() {
        let g = NoopPaymentGateway::new();
        let r = g
            .pay(&aid("agent:x"), 100_000, "base", "0xabc", Some("job:1"))
            .await
            .unwrap();
        assert_eq!(r.amount_usdc_micro, 100_000);
        assert_eq!(g.count(), 1);
        assert_eq!(g.total_spent_micro(), 100_000);
    }
}
