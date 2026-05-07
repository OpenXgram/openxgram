//! 패스워드 입력 — 환경변수 우선, 미설정 + TTY 면 인터랙티브 prompt.
//!
//! 동기:
//!   - core 의 `require_password()` 는 env 만 — UX 친화적이지 않음 (env 안 설정한 사용자는 에러만 봄).
//!   - CI · 자동화에선 env 가 옳음. 인터랙티브 사용자에겐 prompt 가 옳음.
//!   - 둘 다 만족: env 있으면 사용, 없고 TTY 면 prompt, 둘 다 아니면(파이프·non-tty) 명시 에러.
//!
//! 절대 규칙: silent fallback 금지. prompt 진입·env fallback 모두 명시 안내.

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::env::{require_password, MIN_PASSWORD_LEN, PASSWORD_ENV};

/// init / wizard 등에서 사용 — 새 키페어용 패스워드 (확인 입력 포함).
/// 길이는 `MIN_PASSWORD_LEN` 이상 강제.
pub fn obtain_password_for_init() -> Result<String> {
    if let Ok(pw) = require_password() {
        return Ok(pw);
    }
    if !is_tty_interactive() {
        bail!(
            "환경변수 {PASSWORD_ENV} 누락 + TTY 아님 — 자동화에선 export {PASSWORD_ENV}=... 또는 인터랙티브 터미널 사용"
        );
    }
    println!();
    println!("→ keystore 패스워드 입력 (최소 {MIN_PASSWORD_LEN}자, 화면 미표시)");
    println!("  영구 보존하려면 다음 세션부터 export {PASSWORD_ENV}='...'");
    let pw1 =
        rpassword::prompt_password("패스워드: ").map_err(|e| anyhow!("패스워드 입력 실패: {e}"))?;
    if pw1.len() < MIN_PASSWORD_LEN {
        bail!("패스워드는 최소 {MIN_PASSWORD_LEN}자 (현재: {})", pw1.len());
    }
    let pw2 = rpassword::prompt_password("패스워드 확인: ")
        .map_err(|e| anyhow!("패스워드 확인 입력 실패: {e}"))?;
    if pw1 != pw2 {
        bail!("두 입력이 일치하지 않습니다.");
    }
    Ok(pw1)
}

/// 기존 keystore 사용처 — 새 입력 확인 없이 단일 prompt.
pub fn obtain_password_for_unlock() -> Result<String> {
    if let Ok(pw) = require_password() {
        return Ok(pw);
    }
    if !is_tty_interactive() {
        bail!(
            "환경변수 {PASSWORD_ENV} 누락 + TTY 아님 — export {PASSWORD_ENV}=... 또는 인터랙티브 터미널 사용"
        );
    }
    rpassword::prompt_password("keystore 패스워드: ")
        .map_err(|e| anyhow!("패스워드 입력 실패: {e}"))
        .context("패스워드 입력 실패")
}

fn is_tty_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}
