//! 환경변수 키 + 헬퍼 — 모든 crate 가 이 이름·메시지를 공유한다.

use crate::{CoreError, Result};

pub const PASSWORD_ENV: &str = "XGRAM_KEYSTORE_PASSWORD";
pub const SEED_ENV: &str = "XGRAM_SEED";

pub const MIN_PASSWORD_LEN: usize = 12;

/// XGRAM_KEYSTORE_PASSWORD 를 읽어 반환. 미설정 시 raise.
pub fn require_password() -> Result<String> {
    std::env::var(PASSWORD_ENV).map_err(|_| CoreError::MissingEnv(PASSWORD_ENV))
}

/// XGRAM_SEED 를 읽어 반환. 미설정 시 raise.
pub fn require_seed_phrase() -> Result<String> {
    std::env::var(SEED_ENV).map_err(|_| CoreError::MissingEnv(SEED_ENV))
}
