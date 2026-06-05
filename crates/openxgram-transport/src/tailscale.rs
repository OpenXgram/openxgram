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

/// 머신의 LAN(사설망) IPv4 — Tailscale 미설치/미로그인 환경의 fallback.
///
/// 외부 라우팅 IP(8.8.8.8:80)로 UDP "connect" 만 수행 (실제 패킷 전송 없음 —
/// 커널이 라우팅 테이블을 보고 outbound interface 의 source IP 를 고름).
/// loopback(127.x)·link-local(169.254.x)·unspecified 는 reachable 아님 → 제외.
/// 의존성 추가 없이 std 만으로 구현.
pub fn lan_ipv4() -> Option<Ipv4Addr> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) => {
            let ip = *v4.ip();
            if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
                None
            } else {
                Some(ip)
            }
        }
        _ => None,
    }
}

/// 이 머신의 cross-machine reachable transport URL 을 동적 계산 — `http://<ip>:<port>`.
///
/// 우선순위: ① Tailscale IPv4(`tailscale ip --4`, CGNAT 100.64.0.0/10) →
/// ② LAN IPv4(`lan_ipv4`). 둘 다 실패 시 None (silent fallback 금지 — 호출측 로깅).
///
/// 등록(register_subagent / retroactive) + ACK(sender_transport_url) 경로가 공유.
/// `127.0.0.1`/`0.0.0.0` 같은 도달 불가 주소를 절대 반환하지 않음.
pub fn self_reachable_url(port: u16) -> Option<String> {
    if let Ok(ip) = local_ipv4() {
        return Some(format!("http://{ip}:{port}"));
    }
    lan_ipv4().map(|ip| format!("http://{ip}:{port}"))
}

/// 주어진 주소 문자열이 cross-machine 도달 불가(localhost/unspecified/빈값)인지.
/// gossip merge·self-heal 에서 "오염된" 주소를 식별·거부하는 데 사용.
pub fn is_unreachable_address(address: &str) -> bool {
    let a = address.trim();
    if a.is_empty() {
        return true;
    }
    a.contains("127.0.0.1")
        || a.contains("0.0.0.0")
        || a.contains("localhost")
        || a.contains("[::1]")
        || a.contains("://unknown")
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
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("tailscale status JSON 파싱 실패")?;
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

    #[test]
    fn lan_ipv4_is_not_loopback() {
        // 환경에 네트워크가 없을 수도 있으므로 None 허용. Some 일 때만 reachable 검증.
        if let Some(ip) = lan_ipv4() {
            assert!(!ip.is_loopback(), "lan_ipv4 가 loopback 반환: {ip}");
            assert!(!ip.is_unspecified(), "lan_ipv4 가 0.0.0.0 반환");
        }
    }

    #[test]
    fn self_reachable_url_never_localhost() {
        // self_reachable_url 은 절대 127.0.0.1/0.0.0.0 을 반환하지 않는다.
        if let Some(url) = self_reachable_url(47300) {
            assert!(url.starts_with("http://"));
            assert!(url.ends_with(":47300"));
            assert!(
                !is_unreachable_address(&url),
                "self_reachable_url 이 도달 불가 주소 반환: {url}"
            );
        }
    }

    #[test]
    fn is_unreachable_address_classifies() {
        assert!(is_unreachable_address("http://127.0.0.1:47300"));
        assert!(is_unreachable_address("http://0.0.0.0:47300"));
        assert!(is_unreachable_address("http://localhost:47300"));
        assert!(is_unreachable_address("http://unknown"));
        assert!(is_unreachable_address(""));
        assert!(!is_unreachable_address("http://100.101.237.9:47300"));
        assert!(!is_unreachable_address("http://192.168.1.20:17400"));
    }
}
