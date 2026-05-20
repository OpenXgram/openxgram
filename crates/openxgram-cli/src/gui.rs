//! `xgram gui` — 웹 GUI(Tailscale Funnel) 진입점.
//!
//! v0.2.0-rc.24 이후 Tauri 데스크톱 앱 폐기 → 웹 GUI 로 대체.
//! 흐름:
//!   1. `tailscale status --json` 으로 Funnel/Serve 활성 URL 조회
//!   2. URL 발견 시 OS 기본 브라우저로 열고 stdout 에도 출력
//!   3. URL 미발견 시 활성화 안내 (silent fallback 금지)
//!
//! 옵션:
//!   --port <PORT>   nginx 가 GUI 정적 자산을 서빙하는 로컬 포트 (default 47310)
//!   --no-open       브라우저 자동 실행 안 함 (URL stdout 출력만)
//!
//! Tauri 별 바이너리 `xgram-desktop` 호출은 완전히 제거 — install tarball 에서도 미동봉.

use std::process::Command;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde_json::Value;

#[derive(Debug, Parser)]
pub struct GuiOpts {
    /// nginx 가 GUI 정적 자산을 서빙하는 로컬 포트. Tailscale Funnel 안내 메시지에서 사용.
    #[arg(long, default_value_t = 47310)]
    pub port: u16,

    /// 브라우저를 자동으로 열지 않음 (URL stdout 출력만).
    #[arg(long, default_value_t = false)]
    pub no_open: bool,
}

impl GuiOpts {
    /// 기존 `Commands::Gui { args }` 에서 받은 trailing args 를 GuiOpts 로 파싱.
    /// args = ["--port", "47310", "--no-open"] 등.
    pub fn from_trailing(args: &[String]) -> Result<Self> {
        // clap try_parse_from 은 첫 인자를 binary name 으로 취급 — dummy 한 칸 채움.
        let mut argv: Vec<String> = vec!["gui".to_string()];
        argv.extend(args.iter().cloned());
        Self::try_parse_from(argv).map_err(|e| anyhow!("gui 인자 파싱 실패: {e}"))
    }
}

/// `tailscale` CLI 호출 — `--json` 으로 구조화된 상태 조회.
/// 결과 JSON 에서 Self.CapMap 의 `https://funnel/*` capability 와 ServeConfig 의
/// Funnel 항목을 찾아 첫 번째 활성 URL 을 반환.
fn detect_funnel_url() -> Result<Option<String>> {
    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            // tailscale 미설치 — null 반환 (호출자가 안내)
            return Err(anyhow!(
                "tailscale CLI 실행 실패: {e}\n\
                 Tailscale 미설치이거나 PATH 에 없음. 설치:\n\
                 \n\
                 Linux  : curl -fsSL https://tailscale.com/install.sh | sh\n\
                 macOS  : brew install --cask tailscale\n\
                 Windows: https://tailscale.com/download/windows"
            ));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "tailscale status --json 실패 (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let json: Value =
        serde_json::from_slice(&output.stdout).context("tailscale status --json 파싱 실패")?;

    // Self.DNSName — 자기 머신의 MagicDNS 이름 (예: "whitegun-win-1.tail0957ca.ts.net.")
    let dns_name = json
        .get("Self")
        .and_then(|s| s.get("DNSName"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim_end_matches('.').to_string());

    let dns_name = match dns_name {
        Some(n) if !n.is_empty() => n,
        _ => return Ok(None), // tailnet 미가입 — Funnel 불가
    };

    // CurrentTailnet.MagicDNSEnabled / Self.CapMap 에 funnel cap 있는지 확인.
    let funnel_enabled = json
        .get("Self")
        .and_then(|s| s.get("CapMap"))
        .and_then(|m| m.as_object())
        .map(|m| {
            m.keys()
                .any(|k| k.starts_with("https://tailscale.com/cap/funnel"))
        })
        .unwrap_or(false);

    if !funnel_enabled {
        // CapMap 미노출 버전(tailscale <1.50) 도 있어서 — DNSName 있으면 진행 시도.
        // 단 활성화 안 됐을 가능성 안내는 호출자가.
        return Ok(Some(format!("https://{dns_name}")));
    }

    Ok(Some(format!("https://{dns_name}")))
}

/// OS 기본 브라우저로 URL 오픈. 실패해도 panic 안 함 — stderr 경고만.
fn open_in_browser(url: &str) -> Result<()> {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        // PowerShell `Start-Process` 가 cmd.exe `start` 보다 안정적 (URL escape).
        ("cmd", vec!["/C", "start", "", url])
    } else if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else {
        // Linux/BSD — xdg-open (xdg-utils 패키지).
        ("xdg-open", vec![url])
    };

    let status = Command::new(cmd)
        .args(&args)
        .status()
        .with_context(|| format!("브라우저 실행 실패: {cmd} {args:?}"))?;

    if !status.success() {
        return Err(anyhow!(
            "브라우저 종료 코드 비정상 ({}). URL: {url}",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

pub fn run_gui(args: &[String]) -> Result<()> {
    let opts = GuiOpts::from_trailing(args)?;

    let url_result = detect_funnel_url();

    let url = match url_result {
        Ok(Some(u)) => u,
        Ok(None) => {
            eprintln!(
                "Tailscale tailnet 미가입 — Funnel URL 추출 불가.\n\
                 \n\
                 활성화 방법:\n\
                 \n\
                 1) Tailscale 로그인 (한 번만):\n\
                    sudo tailscale up\n\
                 \n\
                 2) Funnel 활성화 (nginx GUI 포트 → :443 노출):\n\
                    sudo tailscale funnel --bg --https=443 http://localhost:{port}\n\
                 \n\
                 3) 재실행: xgram gui",
                port = opts.port,
            );
            return Err(anyhow!("Tailscale tailnet 미가입"));
        }
        Err(e) => {
            eprintln!("{e}");
            return Err(e);
        }
    };

    // URL 항상 stdout — 사용자가 복사·붙여넣기 가능.
    println!("OpenXgram 웹 GUI URL:");
    println!("  {url}");
    println!();
    println!("Funnel 미활성 시 다음 명령으로 활성화:");
    println!(
        "  sudo tailscale funnel --bg --https=443 http://localhost:{port}",
        port = opts.port
    );

    if opts.no_open {
        println!();
        println!("(--no-open 지정 — 브라우저 자동 실행 생략)");
        return Ok(());
    }

    match open_in_browser(&url) {
        Ok(()) => {
            println!();
            println!("✓ 브라우저에서 GUI 열림");
        }
        Err(e) => {
            eprintln!();
            eprintln!("⚠ 브라우저 자동 실행 실패: {e}");
            eprintln!("  위 URL 을 직접 복사하여 브라우저에 붙여넣어 주세요.");
        }
    }

    Ok(())
}
