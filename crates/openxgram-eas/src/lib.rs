//! 4.1 EAS — Ethereum Attestation Service 어댑터.
//!
//! 본 crate 는 schema 정의 + offchain 어테스테이션(서명) 생성 + (옵션) onchain 제출을 분리한다.
//! 실제 Base mainnet 트랜잭션 제출은 alloy 가 필요하지만, 본 crate 는 schema/UID 계산 +
//! attestation row 저장 + 가스 정책 검증까지 책임. onchain 제출은 호출자(또는 다음 PR)에서.
//!
//! Why: master 의 "코드 회로 / 실서버 검증 분리" 원칙. 단위 테스트가 schema UID·attestation hash·
//! 가스 정책을 검증할 수 있으면 chain RPC 없이도 신뢰 가능.

pub mod attest;
pub mod gas;
pub mod schema;
pub mod store;

pub use attest::{Attestation, AttestationData, AttestationKind};
pub use gas::{GasPolicy, GasQuote};
pub use schema::{
    schema_uid, EndorsementSchema, MessageSchema, PaymentSchema, SchemaDefinition, SchemaRegistry,
};
pub use store::AttestationStore;

#[derive(Debug, thiserror::Error)]
pub enum EasError {
    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("openxgram-db: {0}")]
    Xdb(#[from] openxgram_db::DbError),

    #[error("invalid schema: {0}")]
    InvalidSchema(String),

    #[error("gas policy violation: estimated {estimated_usd:.4} USD > limit {limit_usd:.4} USD")]
    GasOverLimit {
        estimated_usd: f64,
        limit_usd: f64,
    },

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid kind: {0}")]
    InvalidKind(String),
}

pub type Result<T> = std::result::Result<T, EasError>;
