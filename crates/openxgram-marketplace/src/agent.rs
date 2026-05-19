//! 도메인 객체 — OpenAgentX 마켓플레이스 API 응답에 대응.
//!
//! 정본: PRD-OpenAgentX §4 (마켓 데이터 모델)와 호환되도록 설계.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// 에이전트 ID — 마켓플레이스에서 발급한 식별자 (예: `agent:<uuid>` 또는 `oga-<slug>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    /// 내부 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AgentId {
    type Err = crate::MarketplaceError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(crate::MarketplaceError::Invalid(
                "agent_id must not be empty".into(),
            ));
        }
        Ok(AgentId(s.to_string()))
    }
}

/// 서비스 ID (한 에이전트 내).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceId(pub String);

impl ServiceId {
    /// 내부.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ServiceId {
    type Err = crate::MarketplaceError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(crate::MarketplaceError::Invalid(
                "service_id must not be empty".into(),
            ));
        }
        Ok(ServiceId(s.to_string()))
    }
}

/// 작업 ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub String);

impl JobId {
    /// 내부.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for JobId {
    type Err = crate::MarketplaceError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(crate::MarketplaceError::Invalid(
                "job_id must not be empty".into(),
            ));
        }
        Ok(JobId(s.to_string()))
    }
}

/// 마켓 에이전트 (검색 결과 + 상세 공용).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// `agent:<...>`.
    pub id: AgentId,
    /// 표시명.
    pub name: String,
    /// 한 줄 설명.
    pub description: String,
    /// 메이커(판매자) 식별자 — DID 또는 별칭.
    pub maker_id: Option<String>,
    /// 카테고리 (예: "writing", "code", "data").
    pub category: Option<String>,
    /// 별점 평균 (0.0~5.0). 미평가 = None.
    pub rating: Option<f32>,
    /// 평가 수.
    pub rating_count: Option<u32>,
    /// 제공 서비스들 (검색 응답에서는 비어있을 수 있음, 상세 응답에서 채워짐).
    #[serde(default)]
    pub services: Vec<Service>,
}

/// 한 에이전트가 제공하는 서비스.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    /// 서비스 ID.
    pub id: ServiceId,
    /// 표시명.
    pub name: String,
    /// 설명.
    pub description: String,
    /// USDC 마이크로 단위 (1 USDC = 1_000_000). 가격 = price_usdc_micro / 1_000_000.
    pub price_usdc_micro: i64,
    /// 입력 스키마 (JSON Schema 같은 자유 구조).
    #[serde(default)]
    pub input_schema: serde_json::Value,
    /// 평균 처리 시간 (초).
    pub avg_duration_sec: Option<u32>,
}

impl Service {
    /// 사람이 읽기 쉬운 USDC 표현 (예: "0.50 USDC").
    pub fn price_display(&self) -> String {
        format_usdc(self.price_usdc_micro)
    }
}

/// 작업 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// 접수.
    Queued,
    /// 처리 중.
    Running,
    /// 완료.
    Completed,
    /// 실패.
    Failed,
    /// 취소.
    Cancelled,
}

impl JobStatus {
    /// 문자열 표현.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 종결 상태 (더 이상 변하지 않음).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// 신규 작업 발주 요청 (purchase_service 입력).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewJobRequest {
    /// 에이전트.
    pub agent_id: AgentId,
    /// 서비스.
    pub service_id: ServiceId,
    /// 사용자 정의 입력 (서비스 schema에 따름).
    pub input: serde_json::Value,
    /// 사용자가 명시한 최대 지불 의향 (USDC 마이크로). 없으면 서비스 정가.
    pub max_price_usdc_micro: Option<i64>,
}

impl NewJobRequest {
    /// 검증.
    pub fn validate(&self) -> Result<(), crate::MarketplaceError> {
        if self.agent_id.as_str().trim().is_empty() {
            return Err(crate::MarketplaceError::Invalid(
                "agent_id required".into(),
            ));
        }
        if self.service_id.as_str().trim().is_empty() {
            return Err(crate::MarketplaceError::Invalid(
                "service_id required".into(),
            ));
        }
        if let Some(m) = self.max_price_usdc_micro {
            if m < 0 {
                return Err(crate::MarketplaceError::Invalid(
                    "max_price_usdc_micro must be >= 0".into(),
                ));
            }
        }
        Ok(())
    }
}

/// 작업 객체 (POST /api/jobs 응답 + GET /api/jobs/[id]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// `job:<uuid>` 또는 마켓이 정한 식별자.
    pub id: JobId,
    /// 에이전트.
    pub agent_id: AgentId,
    /// 서비스.
    pub service_id: ServiceId,
    /// 상태.
    pub status: JobStatus,
    /// 결과 (completed 시).
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    /// 에러 메시지 (failed 시).
    #[serde(default)]
    pub error: Option<String>,
    /// 결제된 USDC 마이크로.
    #[serde(default)]
    pub paid_usdc_micro: Option<i64>,
    /// 결제 tx 해시 (on-chain 시).
    #[serde(default)]
    pub payment_tx_hash: Option<String>,
    /// 생성 시각 (RFC3339).
    pub created_at: String,
    /// 갱신 시각 (RFC3339).
    pub updated_at: String,
}

/// `n` USDC 마이크로 → "X.YZ USDC" 표현 (trailing zero 제거).
pub fn format_usdc(micro: i64) -> String {
    let whole = micro / 1_000_000;
    let frac = (micro.abs()) % 1_000_000;
    let frac_str = format!("{frac:06}").trim_end_matches('0').to_string();
    if frac_str.is_empty() {
        format!("{whole} USDC")
    } else {
        format!("{whole}.{frac_str} USDC")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_parse_roundtrip() {
        let id: AgentId = "agent:abc".parse().unwrap();
        assert_eq!(id.to_string(), "agent:abc");
    }

    #[test]
    fn empty_agent_id_rejected() {
        let res: Result<AgentId, _> = "".parse();
        assert!(res.is_err());
    }

    #[test]
    fn job_status_terminal() {
        assert!(!JobStatus::Queued.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
        assert!(JobStatus::Completed.is_terminal());
        assert!(JobStatus::Failed.is_terminal());
        assert!(JobStatus::Cancelled.is_terminal());
    }

    #[test]
    fn job_status_serde() {
        let s = serde_json::to_string(&JobStatus::Completed).unwrap();
        assert_eq!(s, "\"completed\"");
        let back: JobStatus = serde_json::from_str("\"queued\"").unwrap();
        assert_eq!(back, JobStatus::Queued);
    }

    #[test]
    fn format_usdc_basic() {
        assert_eq!(format_usdc(500_000), "0.5 USDC");
        assert_eq!(format_usdc(1_500_000), "1.5 USDC");
        assert_eq!(format_usdc(2_000_000), "2 USDC");
        assert_eq!(format_usdc(123_456), "0.123456 USDC");
    }

    #[test]
    fn service_price_display() {
        let s = Service {
            id: ServiceId("svc1".into()),
            name: "Translate".into(),
            description: "EN→KO".into(),
            price_usdc_micro: 500_000,
            input_schema: serde_json::Value::Null,
            avg_duration_sec: Some(30),
        };
        assert_eq!(s.price_display(), "0.5 USDC");
    }

    #[test]
    fn new_job_request_validates() {
        let r = NewJobRequest {
            agent_id: AgentId("".into()),
            service_id: ServiceId("svc".into()),
            input: serde_json::Value::Null,
            max_price_usdc_micro: None,
        };
        assert!(r.validate().is_err());

        let r2 = NewJobRequest {
            agent_id: AgentId("agent:1".into()),
            service_id: ServiceId("svc".into()),
            input: serde_json::Value::Null,
            max_price_usdc_micro: Some(-1),
        };
        assert!(r2.validate().is_err());

        let r3 = NewJobRequest {
            agent_id: AgentId("agent:1".into()),
            service_id: ServiceId("svc".into()),
            input: serde_json::Value::Null,
            max_price_usdc_micro: Some(1_000_000),
        };
        assert!(r3.validate().is_ok());
    }

    #[test]
    fn agent_deserialize_minimal() {
        let json = r#"{
            "id": "agent:test",
            "name": "Test Agent",
            "description": "does stuff"
        }"#;
        let a: Agent = serde_json::from_str(json).unwrap();
        assert_eq!(a.id.as_str(), "agent:test");
        assert!(a.services.is_empty());
    }
}
