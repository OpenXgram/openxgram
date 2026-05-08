//! `xgram link <url>` — 데스크탑이 원격 daemon 에 연결하기 위한 페어링.
//!
//! URL 형식: `oxg://<alias>@<host>:<port>#token=<bearer>`
//!   - `<alias>`: 정보용 (사용자 표시; 검증은 안 함)
//!   - `<host>:<port>`: daemon 의 GUI HTTP API 주소 (예: 100.64.1.1:47302)
//!   - `#token=...`: mcp-token (Bearer)
//!
//! 동작:
//!   1. URL 파싱·검증
//!   2. `<host>:<port>/v1/gui/health` 호출로 도달성 확인 (silent 실패 금지)
//!   3. 성공 시 `<data_dir>/desktop-link.json` 저장
//!      - daemon_client 가 env 우선 → json 폴백
//!
//! 절대 규칙: silent fallback 금지 — 검증 실패 시 명시 raise.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

const LINK_FILE: &str = "desktop-link.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct DesktopLink {
    pub alias: String,
    pub daemon_url: String,
    pub daemon_token: String,
}

impl DesktopLink {
    pub fn path(data_dir: &Path) -> std::path::PathBuf {
        data_dir.join(LINK_FILE)
    }

    pub fn load(data_dir: &Path) -> Result<Option<Self>> {
        let p = Self::path(data_dir);
        if !p.exists() {
            return Ok(None);
        }
        let s = std::fs::read_to_string(&p)
            .with_context(|| format!("desktop-link.json 읽기 실패: {}", p.display()))?;
        let link: DesktopLink = serde_json::from_str(&s).context("desktop-link.json 파싱 실패")?;
        Ok(Some(link))
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(data_dir).ok();
        let p = Self::path(data_dir);
        let s = serde_json::to_string_pretty(self).context("desktop-link.json 직렬화 실패")?;
        std::fs::write(&p, s)
            .with_context(|| format!("desktop-link.json 쓰기 실패: {}", p.display()))?;
        // POSIX: perm 0600 (token 평문 저장이라 사용자 외 read 차단).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

/// `oxg://alias@host:port#token=xxx` → 구조화 분해.
pub fn parse_oxg_url(url: &str) -> Result<DesktopLink> {
    let rest = url
        .strip_prefix("oxg://")
        .ok_or_else(|| anyhow!("URL 은 'oxg://' 로 시작해야 합니다"))?;
    let (auth_host, fragment) = rest
        .split_once('#')
        .ok_or_else(|| anyhow!("토큰 fragment(#token=...) 누락"))?;
    let (alias, host_port) = auth_host
        .split_once('@')
        .ok_or_else(|| anyhow!("alias 부분 누락 — 예: oxg://Starian@100.64.1.1:47302#token=..."))?;
    if alias.is_empty() {
        bail!("alias 가 비었습니다");
    }
    if host_port.is_empty() {
        bail!("host:port 가 비었습니다");
    }
    let token = fragment
        .strip_prefix("token=")
        .ok_or_else(|| anyhow!("fragment 는 'token=' 으로 시작해야 합니다"))?;
    if token.is_empty() {
        bail!("token 값이 비었습니다");
    }
    Ok(DesktopLink {
        alias: alias.to_string(),
        daemon_url: format!("http://{host_port}"),
        daemon_token: token.to_string(),
    })
}

/// `xgram link` 진입점. URL 파싱 → 도달성 확인 → 저장.
pub async fn run_link(data_dir: &Path, url: &str) -> Result<()> {
    let link = parse_oxg_url(url).context("URL 파싱 실패")?;
    println!("→ 페어링 정보:");
    println!("  alias        : {}", link.alias);
    println!("  daemon_url   : {}", link.daemon_url);
    println!(
        "  daemon_token : {}…",
        &link.daemon_token[..8.min(link.daemon_token.len())]
    );
    println!();

    println!("→ 도달성 확인 (GET /v1/gui/health) ...");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let resp = client
        .get(format!("{}/v1/gui/health", link.daemon_url))
        .send()
        .await
        .with_context(|| {
            format!(
                "daemon health 호출 실패 — {} 도달 불가 (Tailscale 인증·daemon 실행 확인)",
                link.daemon_url
            )
        })?;
    if !resp.status().is_success() {
        bail!("daemon health HTTP {}", resp.status());
    }
    println!("  ✓ daemon 도달");

    println!("→ /v1/gui/status 인증 확인 ...");
    let resp = client
        .get(format!("{}/v1/gui/status", link.daemon_url))
        .bearer_auth(&link.daemon_token)
        .send()
        .await
        .context("daemon status 호출 실패")?;
    if !resp.status().is_success() {
        bail!(
            "daemon status HTTP {} — token 불일치? mcp-token 재발급 필요할 수 있음",
            resp.status()
        );
    }
    println!("  ✓ 인증 통과");

    link.save(data_dir).context("desktop-link.toml 저장 실패")?;
    println!();
    println!("✓ 링크 저장: {}", DesktopLink::path(data_dir).display());
    println!();
    println!("이제 `xgram gui` 또는 다른 명령이 환경변수 없이도 원격 daemon 에 연결합니다.");
    println!("(env XGRAM_DAEMON_URL / XGRAM_DAEMON_TOKEN 이 우선 — 설정되어 있으면 link 무시)");
    Ok(())
}
