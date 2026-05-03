//! openxgram-memory — L0~L4 메모리 레이어 + 임베딩 + 회상.
//!
//! 모듈 (단일 책임):
//!   - embed     : Embedder trait + DummyEmbedder
//!   - message   : L0 messages 저장 + sqlite-vec KNN 회상
//!   - episode   : L1 session reflection (L0 → L1 집계)
//!   - memory    : L2 fact/decision/reference/rule 저장
//!   - util      : 모듈 내부 공통 헬퍼

pub mod embed;
pub mod episode;
pub mod memory;
pub mod message;
pub mod session;
pub mod stats;
pub mod transfer;
mod util;

pub use embed::{DummyEmbedder, Embedder, EMBED_DIM};
pub use episode::{reflect_session, Episode, EpisodeStore};
pub use memory::{Memory, MemoryKind, MemoryStore};
pub use message::{Message, MessageStore, RecallHit};
pub use session::{Session, SessionStore};
pub use stats::{store_stats, StoreStats};
pub use transfer::{export_session, TextPackage};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("db error: {0}")]
    Db(#[from] openxgram_db::DbError),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("embedding dimension mismatch: got {got}, expected {expected}")]
    DimMismatch { got: usize, expected: usize },

    #[error("unexpected affected rows: expected {expected}, got {actual}")]
    UnexpectedRowCount { expected: u64, actual: u64 },

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),

    #[error("invalid memory kind: {0}")]
    InvalidKind(String),
}

pub type Result<T> = std::result::Result<T, MemoryError>;
