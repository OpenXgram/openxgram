//! openxgram-keystore — Keypair management
//!
//! secp256k1 HD 키페어 생성, 저장, 파생을 담당합니다.
//! Phase 1: 인터페이스 정의 (stub). 구현은 Phase 2 이후.
//! TODO(Phase 2): k256 + bip39 크레이트 연동

use openxgram_core::AgentIdentity;

/// 키스토어 작업 에러
#[derive(Debug, thiserror::Error)]
pub enum KeystoreError {
    #[error("key not found: {0}")]
    NotFound(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid mnemonic")]
    InvalidMnemonic,

    #[error("other: {0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, KeystoreError>;

/// 키스토어 트레이트 — Phase 2에서 구현
pub trait Keystore {
    /// 새 HD 키페어 생성 (BIP-39 mnemonic 반환)
    fn generate(&self) -> Result<(AgentIdentity, String)>;

    /// 기존 mnemonic으로 복원
    fn restore(&self, mnemonic: &str) -> Result<AgentIdentity>;

    /// 저장된 신원 로드
    fn load(&self) -> Result<AgentIdentity>;
}

/// 파일시스템 기반 키스토어 (Phase 2 구현 예정)
pub struct FsKeystore {
    /// 키 저장 경로 (기본: ~/.openxgram/keystore/)
    pub path: std::path::PathBuf,
}

impl FsKeystore {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}
