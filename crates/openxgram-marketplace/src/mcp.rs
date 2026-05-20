//! 4 MCP 도구 핸들러 — `marketplace_search` / `marketplace_get_agent` /
//! `purchase_service` / `get_job_status`.
//!
//! 도메인 핸들러만 제공. JSON-RPC 어댑터는 openxgram-mcp 또는 openxgram-cli의
//! `mcp_serve`에서 래핑 (별 task A21).
//!
//! 결제 자동화 흐름 (purchase_service):
//!   1. 마켓에서 서비스 가격 조회 (또는 호출자가 명시 max_price)
//!   2. SpendPolicy.evaluate(...) → AutoApprove / RequireConfirm / Reject
//!   3. AutoApprove → PaymentGateway.pay(...) → tx_hash를 X-Payment-Tx 헤더로
//!      MarketplaceClient.create_job(...) 호출
//!   4. RequireConfirm → PurchaseDecision::NeedsConfirmation 반환 (job 생성 X)

use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::agent::{Agent, AgentId, Job, JobId, NewJobRequest, Service, ServiceId};
use crate::client::MarketplaceClient;
use crate::policy::{PaymentGateway, PaymentReceipt, SpendPolicy, SpendPolicyDecision};
use crate::MarketplaceError;

/// 4 MCP 도구 묶음.
///
/// `gateway`는 `Arc<dyn PaymentGateway>`로 trait object — 테스트는 `NoopPaymentGateway`,
/// 프로덕션은 vault+payment 연동 구현체를 주입.
pub struct MarketplaceTools {
    client: MarketplaceClient,
    policy: SpendPolicy,
    gateway: Arc<dyn PaymentGateway>,
    /// 결제 체인 (기본 "base").
    chain: String,
}

impl MarketplaceTools {
    /// 신규.
    pub fn new(
        client: MarketplaceClient,
        policy: SpendPolicy,
        gateway: Arc<dyn PaymentGateway>,
    ) -> Self {
        Self {
            client,
            policy,
            gateway,
            chain: "base".into(),
        }
    }

    /// 체인 override (예: "base-sepolia" 테스트).
    pub fn with_chain(mut self, chain: impl Into<String>) -> Self {
        self.chain = chain.into();
        self
    }

    /// 내부 client 접근 (디버깅·확장용).
    pub fn client(&self) -> &MarketplaceClient {
        &self.client
    }

    /// 내부 policy mutable 접근 (한도 갱신 등).
    pub fn policy_mut(&mut self) -> &mut SpendPolicy {
        &mut self.policy
    }

    /// `marketplace_search(query, limit?)`.
    pub async fn search(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<SearchResult, MarketplaceError> {
        if query.trim().is_empty() {
            return Err(MarketplaceError::Invalid("query must not be empty".into()));
        }
        let agents = self.client.search_agents(query, limit).await?;
        debug!(
            target = "openxgram_marketplace",
            query,
            hits = agents.len(),
            "marketplace_search"
        );
        Ok(SearchResult {
            query: query.to_string(),
            count: agents.len(),
            agents,
        })
    }

    /// `marketplace_get_agent(agent_id)`.
    pub async fn get_agent(&self, agent_id: &str) -> Result<Agent, MarketplaceError> {
        let id = AgentId::from_str(agent_id)?;
        let a = self.client.get_agent(&id).await?;
        Ok(a)
    }

    /// `purchase_service({agent_id, service_id, input, max_price?})`.
    ///
    /// 정책 통과 시 자동 결제 + job 생성. 통과 못하면 `NeedsConfirmation`.
    pub async fn purchase(
        &self,
        request: NewJobRequest,
    ) -> Result<PurchaseResult, MarketplaceError> {
        request.validate()?;

        // 1. 가격 결정 — max_price 명시되면 그것, 아니면 마켓에서 서비스 정가 조회
        let amount_micro = match request.max_price_usdc_micro {
            Some(m) => m,
            None => self.resolve_service_price(&request).await?,
        };

        // 2. 정책 평가
        let spent_hour = self
            .gateway
            .spent_last_hour_micro()
            .await
            .map_err(MarketplaceError::Payment)?;
        let spent_today = self
            .gateway
            .spent_today_micro()
            .await
            .map_err(MarketplaceError::Payment)?;
        let spent_month = self
            .gateway
            .spent_this_month_micro()
            .await
            .map_err(MarketplaceError::Payment)?;
        let decision = self.policy.evaluate(
            &request.agent_id,
            amount_micro,
            spent_hour,
            spent_today,
            spent_month,
        );

        match decision {
            SpendPolicyDecision::Reject(r) => Err(MarketplaceError::Invalid(r)),
            SpendPolicyDecision::RequireConfirm(reason) => {
                warn!(
                    target = "openxgram_marketplace",
                    agent_id = %request.agent_id,
                    amount_micro,
                    reason = %reason,
                    "purchase requires user confirmation"
                );
                Ok(PurchaseResult {
                    decision: PurchaseDecision::NeedsConfirmation { reason },
                    amount_usdc_micro: amount_micro,
                    receipt: None,
                    job: None,
                })
            }
            SpendPolicyDecision::AutoApprove => {
                // 3. 결제 — payee_address는 마켓이 결정 (서비스 응답에 보통 포함되나
                // 본 단계에선 마켓 측 escrow 주소를 약식으로 사용)
                let payee = format!("market:{}", request.agent_id);
                let memo = Some(format!(
                    "agent={} svc={}",
                    request.agent_id, request.service_id
                ));
                let receipt = self
                    .gateway
                    .pay(
                        &request.agent_id,
                        amount_micro,
                        &self.chain,
                        &payee,
                        memo.as_deref(),
                    )
                    .await
                    .map_err(MarketplaceError::Payment)?;

                // 4. job 생성 — payment_tx_hash를 헤더로 첨부
                let job = self
                    .client
                    .create_job(&request, receipt.tx_hash.as_deref())
                    .await?;
                info!(
                    target = "openxgram_marketplace",
                    job_id = %job.id,
                    agent_id = %request.agent_id,
                    amount_micro,
                    "purchase auto-approved"
                );
                Ok(PurchaseResult {
                    decision: PurchaseDecision::AutoApproved,
                    amount_usdc_micro: amount_micro,
                    receipt: Some(receipt),
                    job: Some(job),
                })
            }
        }
    }

    /// `get_job_status(job_id)`.
    pub async fn get_job_status(&self, job_id: &str) -> Result<Job, MarketplaceError> {
        let id = JobId::from_str(job_id)?;
        let j = self.client.get_job(&id).await?;
        Ok(j)
    }

    /// 마켓에서 서비스 정가 조회 (없으면 에러).
    async fn resolve_service_price(
        &self,
        request: &NewJobRequest,
    ) -> Result<i64, MarketplaceError> {
        let agent = self.client.get_agent(&request.agent_id).await?;
        let svc = find_service(&agent, &request.service_id).ok_or_else(|| {
            MarketplaceError::NotFound(format!(
                "service {} not in agent {}",
                request.service_id, request.agent_id
            ))
        })?;
        Ok(svc.price_usdc_micro)
    }
}

fn find_service<'a>(agent: &'a Agent, sid: &ServiceId) -> Option<&'a Service> {
    agent.services.iter().find(|s| s.id == *sid)
}

/// `marketplace_search` 응답.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    /// 입력 쿼리.
    pub query: String,
    /// 매칭 수.
    pub count: usize,
    /// 매칭 에이전트.
    pub agents: Vec<Agent>,
}

/// `purchase_service` 결정 종류.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PurchaseDecision {
    /// 자동 결제 + job 생성됨.
    AutoApproved,
    /// 사용자 승인 필요 (한도 초과 / 미화이트리스트). 결제 X, job 생성 X.
    NeedsConfirmation {
        /// 사유.
        reason: String,
    },
}

/// `purchase_service` 응답.
#[derive(Debug, Serialize, Deserialize)]
pub struct PurchaseResult {
    /// 결정.
    pub decision: PurchaseDecision,
    /// 평가된 금액.
    pub amount_usdc_micro: i64,
    /// 영수증 (AutoApproved 시).
    pub receipt: Option<PaymentReceipt>,
    /// 생성된 작업 (AutoApproved 시).
    pub job: Option<Job>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::NoopPaymentGateway;

    #[test]
    fn purchase_decision_serde() {
        let d = PurchaseDecision::NeedsConfirmation {
            reason: "over limit".into(),
        };
        let s = serde_json::to_string(&d).unwrap();
        assert!(s.contains("needs_confirmation"));
        assert!(s.contains("over limit"));
    }

    #[tokio::test]
    async fn empty_query_rejected() {
        let client = MarketplaceClient::builder()
            .base_url("http://127.0.0.1:1") // unused
            .build()
            .unwrap();
        let tools = MarketplaceTools::new(
            client,
            SpendPolicy::default(),
            Arc::new(NoopPaymentGateway::new()),
        );
        let res = tools.search("   ", None).await;
        assert!(matches!(res, Err(MarketplaceError::Invalid(_))));
    }
}
