//! 환경변수 키 + 헬퍼 — 모든 crate 가 이 이름·메시지를 공유한다.

use crate::{CoreError, Result};

pub const PASSWORD_ENV: &str = "XGRAM_KEYSTORE_PASSWORD";
pub const SEED_ENV: &str = "XGRAM_SEED";

pub const MIN_PASSWORD_LEN: usize = 12;

/// XGRAM_KEYSTORE_PASSWORD 를 읽어 반환. 미설정 시 raise.
/// rc.191: cmd `set X=Y && cmd` 가 trailing space 를 value 에 포함시키는 Windows quirk → trim.
/// (예: "sd4132sd " 9 chars → "sd4132sd" 8 chars). peer_send invalid password root cause.
pub fn require_password() -> Result<String> {
    std::env::var(PASSWORD_ENV)
        .map_err(|_| CoreError::MissingEnv(PASSWORD_ENV))
        .map(|s| s.trim().to_string())
}

/// XGRAM_SEED 를 읽어 반환. 미설정 시 raise.
pub fn require_seed_phrase() -> Result<String> {
    std::env::var(SEED_ENV)
        .map_err(|_| CoreError::MissingEnv(SEED_ENV))
        .map(|s| s.trim().to_string())
}
