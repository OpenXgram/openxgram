//! `xgram pair-desktop` — 서버측 페어링.
//!
//! 흐름:
//!   1. install-manifest 에서 alias 읽기 (없으면 init 권유 raise)
//!   2. `tailscale ip --4` 로 tailnet IP 획득 (없으면 tailscale up 안내 raise)
//!   3. `mcp-token` 새로 발급 (이름 "desktop")
//!   4. `oxg://<alias>@<ts-ip>:47302#token=<token>` URL 출력
//!
//! 절대 규칙:
//! - silent fallback 금지 — Tailscale 미설치/미로그인 시 명시 안내 + raise
//! - daemon 가동 여부는 검사만 (별도 시작 X) — 사용자 책임
//!
//! 출력 예:
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │ 데스크탑에 한 줄 입력:                                     │
//! │                                                            │
//! │ xgram link 'oxg://Starian@100.64.1.1:47302#token=xxx'      │
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! 사전 조건: `xgram init` 완료 + `tailscale up` 인증.
//! 페어링 후 사용자는 `xgram daemon` 을 띄워야 데스크탑이 실제로 연결됨.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::manifest_path;
use openxgram_core::ports::GUI_PORT;
use openxgram_manifest::InstallManifest;

pub fn run_pair_desktop(data_dir: &Path) -> Result<()> {
    // 1. manifest 로 alias 확인.
    let mp = manifest_path(data_dir);
    let manifest = InstallManifest::read(&mp).with_context(|| {
        format!(
            "install-manifest 미발견 ({}). `xgram init --alias <name>` 먼저 실행.",
            mp.display()
        )
    })?;
    let alias = manifest.machine.alias.clone();

    // 2. Tailscale IP.
    let out = Command::new("tailscale")
        .args(["ip", "--4"])
        .output()
        .map_err(|e| {
            anyhow::anyhow!(
                "tailscale 실행 실패: {e}\n\
                 \n\
                 Tailscale 미설치 — 다음 명령으로 설치 + 인증:\n\
                 \n\
                 [Linux]   curl -fsSL https://tailscale.com/install.sh | sh && sudo tailscale up\n\
                 [macOS]   brew install --cask tailscale  (또는 App Store) → 메뉴바 아이콘에서 Log in\n\
                 [Windows] https://tailscale.com/download/windows"
            )
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "tailscale ip --4 실패: {stderr}\n\
             \n\
             아직 인증 안 됐을 가능성:\n\
             $ sudo tailscale up\n\
             (브라우저 URL 열어 로그인)"
        );
    }
    let ts_ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if ts_ip.is_empty() {
        bail!("tailscale ip --4 출력 비어있음 — `sudo tailscale up` 으로 인증 후 재시도");
    }

    // 3. mcp-token 발급.
    let mut db = crate::mcp_tokens::open_db(data_dir).context("mcp-token DB open 실패")?;
    let (_id, token) = crate::mcp_tokens::create_token(&mut db, "desktop", Some("desktop pairing"))
        .context("mcp-token 발급 실패 — keystore/db 상태 확인")?;

    // 4. URL 출력.
    let pairing = format!("oxg://{alias}@{ts_ip}:{GUI_PORT}#token={token}");
    println!();
    println!("✓ 페어링 준비 완료");
    println!();
    println!("┌──────────────────────────────────────────────────────────────────┐");
    println!("│ 데스크탑에서 한 줄 입력:                                         │");
    println!("│                                                                  │");
    println!("│   xgram link '{pairing}'");
    println!("│                                                                  │");
    println!("└──────────────────────────────────────────────────────────────────┘");
    println!();
    println!("주의:");
    println!("  - 이 URL 은 keystore 비밀번호와 동급 — 외부 노출 금지");
    println!("  - daemon 이 떠 있어야 데스크탑이 실제로 연결됨:");
    println!("      $ xgram daemon &              # 빠른 테스트용");
    println!("      $ xgram daemon-install --bind {ts_ip}:{GUI_PORT}  # 영구");
    println!();
    Ok(())
}
