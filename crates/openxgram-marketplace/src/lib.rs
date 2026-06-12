//! openxgram-marketplace — OpenAgentX 마켓플레이스 클라이언트.
//!
//! 정본: docs/PRD-OpenXgram.md §4.4 + §9 결정 4 (보수적 결제 자동화).
//!
//! "사용자 LLM이 OpenAgentX 마켓플레이스의 에이전트를 자기 도구처럼 부를 수 있게."
//!
//! 4 MCP 도구:
//! - `marketplace_search(query, limit?)`        → HTTP GET /api/agents?q=
//! - `marketplace_get_agent(agent_id)`          → HTTP GET /api/agents/[id]
//! - `purchase_service({agent_id, service_id, input})` → HTTP POST /api/jobs (+ 결제 자동화)
//! - `get_job_status(job_id)`                   → HTTP GET /api/jobs/[id]
//!
//! 결제 자동화 (PRD §9 결정 4 — 보수적):
//!   - 1회 한도 (default $0.50) 초과 시 user confirm 요구
//!   - 일 한도 (default $10) 초과 시 user confirm 요구
//!   - 화이트리스트 외 판매자: 항상 user confirm
//!   - 한도 내 + 화이트리스트 → 자동 결제
//!
//! 모듈:
//!   - agent   : 도메인 객체 (Agent, Service, Job, JobStatus)
//!   - client  : reqwest HTTP 클라이언트 (마켓플레이스 API)
//!   - policy  : SpendPolicy (한도) + PaymentGateway trait
//!   - mcp     : 4 도구 핸들러 (MarketplaceTools)

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod agent;
pub mod client;
pub mod mcp;
pub mod policy;

pub use agent::{Agent, AgentId, Job, JobId, JobStatus, NewJobRequest, Service, ServiceId};
pub use client::{MarketplaceClient, MarketplaceClientError};
pub use mcp::{MarketplaceTools, PurchaseDecision, PurchaseResult, SearchResult};
pub use policy::{
    FreeQuotaGate, NoopPaymentGateway, PaymentGateway, PaymentReceipt, SpendPolicy,
    SpendPolicyDecision,
};

use thiserror::Error;

/// 마켓플레이스 전반 에러.
#[derive(Debug, Error)]
pub enum MarketplaceError {
    /// HTTP 클라이언트.
    #[error("client: {0}")]
    Client(#[from] MarketplaceClientError),

    /// 결제 게이트웨이.
    #[error("payment: {0}")]
    Payment(String),

    /// 입력 검증 실패.
    #[error("invalid: {0}")]
    Invalid(String),

    /// 정책에 의해 거부 (한도 초과 등) — 사용자 확인 필요.
    #[error("denied: {0}")]
    PolicyDenied(String),

    /// 찾을 수 없음.
    #[error("not found: {0}")]
    NotFound(String),

    /// 직렬화.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    /// 일반.
    #[error("{0}")]
    Other(String),
}
