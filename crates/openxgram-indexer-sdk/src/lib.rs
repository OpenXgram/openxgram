//! 4.2 openxgram-indexer-sdk — 누구나 운영 가능한 인덱서 SDK.
//!
//! - `subscriber`: EAS attestation_log 구독 (4.2.1.1) — 본 노드 DB 의 eas_attestations 폴링
//! - `crawler`: ENS records 크롤러 (4.2.1.2) — handle 별 records 수집
//! - `ranking`: 랭킹 plugin 인터페이스 (4.2.1.3) — `Rank` trait + DefaultRanker

pub mod crawler;
pub mod ranking;
pub mod service;
pub mod subscriber;

pub use crawler::{EnsCrawler, EnsRecord, MockEnsResolver, RecordResolver};
pub use ranking::{DefaultRanker, IdentityScore, Rank};
pub use subscriber::{AttestationEvent, AttestationSubscriber};

#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("eas: {0}")]
    Eas(#[from] openxgram_eas::EasError),

    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("openxgram-db: {0}")]
    Xdb(#[from] openxgram_db::DbError),

    #[error("resolver: {0}")]
    Resolver(String),
}

pub type Result<T> = std::result::Result<T, IndexerError>;
