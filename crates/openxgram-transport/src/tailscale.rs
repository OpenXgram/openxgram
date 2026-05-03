//! Tailscale 통합 — `tailscale` CLI 호출로 노드 IP·상태 조회.
//!
//! 마스터 결정 (option A): Tailscale 데몬 의존 OK. tailscaled 가 설치·실행
//! 중이라고 가정. mTLS 는 Tailscale 의 WireGuard 터널이 네트워크 레이어에서
//! 제공 — 별도 axum-level TLS 설정 불필요 (PRD §15).
//!
//! Phase 1 first PR:
//!   - `tailscale ip --4` 로 IPv4 조회
//!   - `tailscale status --json` 으로 상태·peers 조회 (간략 파싱)
//!   - daemon --tailscale 플래그 시 위 IP 로 자동 bind
//!
//! 후속:
//!   - peer ACL (tailnet ACL 정책)
//!   - mDNS 디스커버리

use std::net::Ipv4Addr;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

/// `tailscale` 바이너리 절대 경로 후보.
const TAILSCALE_BIN: &str = "tailscale";

/// 현재 노드의 Tailscale IPv4 — `tailscale ip --4`. 데몬 미설치/미실행 시 raise.
pub fn local_ipv4() -> Result<Ipv4Addr> {
    let out = Command::new(TAILSCALE_BIN)
        .args(["ip", "--4"])
        .output()
        .with_context(|| format!("`{TAILSCALE_BIN}` 실행 실패 — 설치되어 있는지 확인"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("tailscale ip --4 실패: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().ok_or_else(|| anyhow!("빈 출력"))?;
    line.trim()
        .parse::<Ipv4Addr>()
        .with_context(|| format!("Ipv4Addr 파싱 실패: {line}"))
}

/// Tailscale 데몬 health — `tailscale status` 종료 코드 0 여부.
pub fn is_running() -> bool {
    Command::new(TAILSCALE_BIN)
        .args(["status", "--peers=false", "--json"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// status JSON 의 BackendState 필드만 추출 (Running / NeedsLogin / Stopped 등).
pub fn backend_state() -> Result<String> {
    let out = Command::new(TAILSCALE_BIN)
        .args(["status", "--peers=false", "--json"])
        .output()
        .with_context(|| format!("`{TAILSCALE_BIN}` 실행 실패"))?;
    if !out.status.success() {
        bail!(
            "tailscale status 실패: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .context("tailscale status JSON 파싱 실패")?;
    v.get("BackendState")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("BackendState 필드 누락"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CI 환경에 따라 tailscale 바이너리 유무·로그인 상태가 다르므로
    /// 본 테스트는 결과를 확인하지 않고 panic 만 회피.
    #[test]
    fn is_running_does_not_panic() {
        let _ = is_running();
    }

    #[test]
    fn local_ipv4_returns_result_or_error() {
        let _ = local_ipv4();
    }
}
