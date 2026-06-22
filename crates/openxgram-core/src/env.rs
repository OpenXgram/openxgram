//! 환경변수 키 + 헬퍼 — 모든 crate 가 이 이름·메시지를 공유한다.

use crate::{CoreError, Result};

pub const PASSWORD_ENV: &str = "XGRAM_KEYSTORE_PASSWORD";
pub const SEED_ENV: &str = "XGRAM_SEED";
pub const PORTAL_TOKEN_ENV: &str = "XGRAM_PORTAL_TOKEN";
pub const CHAIN_ENV: &str = "XGRAM_CHAIN";

pub const MIN_PASSWORD_LEN: usize = 12;

/// 결제 체인 기본값 — `XGRAM_CHAIN` 미설정 시 사용. chain.rs 레지스트리 키와 일치해야 한다.
pub const DEFAULT_CHAIN: &str = "base";

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

/// portal 인증 토큰 — `XGRAM_PORTAL_TOKEN` env 에서만 읽는다.
/// 평문 폴백 없음(시크릿을 소스에 두지 않는다). 미설정 시 `None` 반환.
/// 빈/공백 값도 미설정으로 취급.
pub fn portal_token() -> Option<String> {
    std::env::var(PORTAL_TOKEN_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// portal 토큰을 반드시 요구. 미설정 시 raise(조용한 폴백 금지).
pub fn require_portal_token() -> Result<String> {
    portal_token().ok_or(CoreError::MissingEnv(PORTAL_TOKEN_ENV))
}

/// 결제 체인 이름 — `XGRAM_CHAIN` env, 미설정 시 [`DEFAULT_CHAIN`].
pub fn chain_name() -> String {
    std::env::var(CHAIN_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_CHAIN.to_string())
}
