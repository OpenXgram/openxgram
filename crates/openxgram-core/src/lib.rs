//! openxgram-core — Core domain types and traits
//!
//! 이 crate는 OpenXgram의 핵심 도메인 모델을 정의합니다.
//! Phase 1: 타입 및 트레이트 골격 (stub). 구현은 Phase 2 이후.

/// 에이전트 신원 (secp256k1 공개키 기반)
/// TODO(Phase 2): k256 크레이트 연동, HD 파생 구현
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentIdentity {
    /// 에이전트 고유 ID (공개키 해시)
    pub id: String,
    /// 사람이 읽을 수 있는 별칭
    pub alias: Option<String>,
}

/// 메모리 레이어 (L0~L4)
/// TODO(Phase 2): 각 레이어별 스토리지 구현
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLayer {
    /// L0: 원시 메시지
    Message,
    /// L1: 에피소드 (컨텍스트 묶음)
    Episode,
    /// L2: 의미 기억 (임베딩 벡터)
    Semantic,
    /// L3: 패턴 (반복 행동 추출)
    Pattern,
    /// L4: 특성 (장기 페르소나)
    Trait,
}

/// Vault 자격증명 엔트리
/// TODO(Phase 2): 암호화 저장 구현
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VaultEntry {
    pub key: String,
    pub tags: Vec<String>,
    // 실제 값은 암호화 후 저장 — Phase 2 구현 예정
}

/// OpenXgram 공통 에러 타입
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("identity error: {0}")]
    Identity(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("other: {0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
