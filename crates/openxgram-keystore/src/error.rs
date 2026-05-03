use thiserror::Error;

/// Keystore 작업 에러 — 모든 실패는 명시적으로 raise
#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("key not found: {0}")]
    NotFound(String),

    #[error("invalid password")]
    InvalidPassword,

    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    #[error("invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    #[error("signature verification failed")]
    SignatureVerification,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("crypto error: {0}")]
    Crypto(String),

    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),
}
