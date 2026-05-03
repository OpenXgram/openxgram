//! openxgram-db — SQLite + sqlite-vec storage layer
//!
//! ~/.openxgram/data.db 에 5층 메모리와 Vault를 저장합니다.
//! Phase 1: 인터페이스 및 마이그레이션 정의 (stub). 구현은 Phase 2 이후.
//! TODO(Phase 2): rusqlite + sqlite-vec 연동

/// DB 작업 에러
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("connection error: {0}")]
    Connection(String),

    #[error("migration error: {0}")]
    Migration(String),

    #[error("query error: {0}")]
    Query(String),

    #[error("other: {0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, DbError>;

/// DB 설정
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// DB 파일 경로 (기본: ~/.openxgram/data.db)
    pub path: std::path::PathBuf,
    /// WAL 모드 활성화 (기본: true)
    pub wal_mode: bool,
}

impl Default for DbConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self {
            path: std::path::PathBuf::from(home).join(".openxgram").join("data.db"),
            wal_mode: true,
        }
    }
}

/// DB 연결 핸들 (Phase 2 구현 예정)
pub struct Db {
    pub config: DbConfig,
}

impl Db {
    pub fn open(config: DbConfig) -> Result<Self> {
        tracing::info!("DB open (stub): {:?}", config.path);
        Ok(Self { config })
    }

    /// 마이그레이션 실행 (Phase 2 구현 예정)
    pub fn migrate(&self) -> Result<()> {
        tracing::info!("migrate (stub): Phase 2 구현 예정");
        Ok(())
    }
}
