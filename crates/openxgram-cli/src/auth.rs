//! Web GUI 잠금 — 단일 keystore 비밀번호. PRD §1: 1 사람 = 1 메인 daemon + N 머신 attach.
//!
//! Multi-user / register / JWT / users 테이블 모두 폐기 — 사이드카 본질은 self-host 개인용.
//! daemon이 XGRAM_KEYSTORE_PASSWORD 환경변수로 시작 → GUI에서 그 비밀번호로 잠금 해제 → session_token 발급.
//!
//! 다른 머신이 attach (PRD §7 v0.9): `xgram pair-desktop` (oxg://) 또는 GUI에서 같은 비밀번호로 unlock.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

static SESSION_TOKEN: OnceLock<String> = OnceLock::new();

#[derive(Debug, Deserialize)]
pub struct UnlockRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UnlockResponse {
    pub session_token: String,
}

/// 입력 비밀번호 == XGRAM_KEYSTORE_PASSWORD 환경변수.
/// silent fallback 금지: env 미설정 시 무조건 false (daemon이 keystore unlock 안 됐다는 뜻).
pub fn verify_password(input: &str) -> bool {
    match std::env::var("XGRAM_KEYSTORE_PASSWORD") {
        Ok(stored) => {
            let a = Sha256::digest(stored.as_bytes());
            let b = Sha256::digest(input.as_bytes());
            a == b
        }
        Err(_) => false,
    }
}

/// 세션 토큰 — daemon 프로세스 생존 동안 1회 발급 후 재사용. 재시작 시 자동 만료.
pub fn session_token() -> &'static str {
    SESSION_TOKEN.get_or_init(|| {
        let mut h = Sha256::new();
        h.update(format!("{:?}", std::time::SystemTime::now()).as_bytes());
        h.update(b"openxgram-session");
        format!("{:x}", h.finalize())
    })
}

pub fn verify_session_token(token: &str) -> bool {
    SESSION_TOKEN
        .get()
        .map(|t| t.as_str() == token)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_match_when_env_set() {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", "test-pw-12-chars");
        assert!(verify_password("test-pw-12-chars"));
        assert!(!verify_password("wrong"));
    }

    #[test]
    fn session_token_stable() {
        let t1 = session_token().to_string();
        let t2 = session_token().to_string();
        assert_eq!(t1, t2);
        assert_eq!(t1.len(), 64);
    }
}
