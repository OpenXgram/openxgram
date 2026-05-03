use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("migration failed at version {version}: {reason}")]
    Migration { version: u32, reason: String },

    #[error("expected {expected} rows affected, got {actual}")]
    UnexpectedRowCount { expected: u64, actual: u64 },

    #[error("sqlite-vec extension load failed: {0}")]
    VecExtension(String),
}
