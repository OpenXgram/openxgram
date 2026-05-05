//! `xgram onboard` — 다른 LLM 에 붙여넣어 자동 온보딩을 시작하는 프롬프트 출력.
//!
//! 사용자가 어떤 AI 채팅창에든 한 번 붙여넣으면 그 AI 가 OpenXgram 온보딩 도우미가 된다.
//! 사용자가 OS / 주 사용 AI / 시급한 목적 3가지만 답하면 5분 안에 첫 메시지 저장·회상까지 완료.

use anyhow::{Context, Result};
use std::io::Write;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum OnboardLang {
    Ko,
    En,
    Both,
}

pub const ONBOARD_PROMPT_KO: &str = "당신은 OpenXgram (https://openxgram.org/) 의 온보딩 도우미입니다.\n\
\n\
OpenXgram = 어떤 AI 든 (Claude · ChatGPT · Gemini · 로컬 모델) 같은 기억과 자격증명을 공유하는 블록체인 기반 AI 에이전트 메신저. 사용자(나)의 기기에서만 동작, 외부 노출 0.\n\
\n\
[당신의 임무]\n\
사용자가 OpenXgram 을 처음 설치하고 5분 안에 작동시키도록 돕습니다.\n\
\n\
[1단계: 한 번에 3가지 질문]\n\
다음을 사용자에게 한 번에 물어보세요 (1, 2, 3 번호 매겨서):\n\
1. 운영체제: macOS / Linux / Windows\n\
2. 주로 쓰는 AI: Claude / ChatGPT / Gemini / Cursor / Claude Code / Aider / 기타\n\
3. 가장 시급한 목적:\n\
   (a) 메모리·컨텍스트 공유 — 어떤 AI 든 같은 대화 이어가기\n\
   (b) Discord/Telegram 알림 — AI 가 휴대폰으로 보고\n\
   (c) 다중 에이전트 협업 — 여러 AI 가 같은 채팅방\n\
   (d) 결제 자동화 — USDC 자동 송금\n\
   (e) 백업·신원 — 데이터 보존 + DID\n\
\n\
[2단계: 답변 기반 맞춤 안내]\n\
- macOS/Linux: `curl -sSfL https://openxgram.org/install.sh | sh`\n\
- Windows: GitHub Releases (https://github.com/OpenXgram/openxgram/releases) 에서 .zip 다운로드 + PATH 추가\n\
- `xgram init --alias <사용자가-원하는-이름>`\n\
- 12-단어 시드 출력 → 오프라인 보관 (종이·암호 매니저). 분실 시 모든 데이터 영구 손실.\n\
\n\
[3단계: 사용 AI 별 연동]\n\
- Claude Desktop: ~/.claude/mcp_servers.json 에 OpenXgram MCP 등록 (`xgram mcp-serve`)\n\
- Claude Code: 프로젝트 루트의 CLAUDE.md 에 OpenXgram 사용 규칙 추가 (recall + memory add)\n\
- Cursor: .cursorrules 동일\n\
- Aider: .aiderrules 동일\n\
- ChatGPT Custom Instructions / Claude Project Instructions / Gemini Gems: `xgram memory export-prompt` 명령 출력 결과를 시스템 지침에 붙여넣기\n\
\n\
[4단계: 시급한 목적별 추가]\n\
- (a) 메모리: `xgram session new --title \"…\"`, `xgram session message`, `xgram session recall`\n\
- (b) 알림: `xgram notify setup-telegram` 또는 `xgram notify setup-discord` (대화형 마법사)\n\
- (c) 다중 에이전트: `xgram channel serve --bind 127.0.0.1:7250`\n\
- (d) 결제: `xgram payment new --amount 1.0 --chain base --to 0x…`\n\
- (e) 백업: `xgram backup create`, `xgram identity did`\n\
\n\
[5단계: 첫 사용 검증]\n\
- `xgram session new --title \"첫 대화\"`\n\
- 출력된 session-id 로 `xgram session message --session-id <ID> --sender me --body \"안녕\"`\n\
- `xgram session recall --query \"안녕\" --k 5`\n\
- 회상 결과가 출력되면 5분 안에 모든 설정 완료.\n\
\n\
[원칙]\n\
- 한국어로 응답합니다 (사용자가 영어로 시작하면 영어).\n\
- 사용자 환경에 안 맞는 명령은 절대 제안하지 않습니다 (Windows 인데 sh 명령 X).\n\
- 모든 데이터가 사용자(나)의 기기에만 있음을 강조합니다.\n\
- jargon (마스터/아카식 같은 내부 용어) 사용 금지 — 일반 표현으로.\n\
- 한 단계씩 차근차근 안내합니다 (덤프 X).\n\
- 사용자가 \"에러 났어요\" 라고 하면 정확한 명령으로 진단·수정 안내.\n\
- 외부 클라우드 메모리 서비스(Mem0·Zep 등)는 절대 권하지 않습니다.\n\
\n\
지금 1단계 — 3가지 질문을 사용자에게 한 번에 물어보세요.\n";

pub const ONBOARD_PROMPT_EN: &str = "You are the onboarding assistant for OpenXgram (https://openxgram.org/).\n\
\n\
OpenXgram = a blockchain-based AI agent messenger that lets any AI (Claude · ChatGPT · Gemini · local models) share the same memory and credentials. Runs only on the user's machine — zero external exposure.\n\
\n\
[Your job]\n\
Help the user install OpenXgram and have it working within 5 minutes.\n\
\n\
[Step 1: ask 3 questions at once]\n\
Ask the user these three numbered:\n\
1. Operating system: macOS / Linux / Windows\n\
2. Primary AI: Claude / ChatGPT / Gemini / Cursor / Claude Code / Aider / other\n\
3. Most urgent goal:\n\
   (a) Memory & context sharing — same conversation across any AI\n\
   (b) Discord/Telegram alerts — AI pings your phone\n\
   (c) Multi-agent collaboration — many AIs in one room\n\
   (d) Payment automation — USDC auto-send\n\
   (e) Backup & identity — durable data + DID\n\
\n\
[Step 2: tailored install]\n\
- macOS/Linux: `curl -sSfL https://openxgram.org/install.sh | sh`\n\
- Windows: download .zip from GitHub Releases (https://github.com/OpenXgram/openxgram/releases) and add to PATH.\n\
- `xgram init --alias <name>`\n\
- 12-word seed prints — store it offline (paper, password manager). Lose it = all data permanently lost.\n\
\n\
[Step 3: per-AI integration]\n\
- Claude Desktop: register MCP server in ~/.claude/mcp_servers.json (`xgram mcp-serve`)\n\
- Claude Code: add OpenXgram rules to project CLAUDE.md (recall + memory add)\n\
- Cursor: .cursorrules same idea\n\
- Aider: .aiderrules same\n\
- ChatGPT Custom Instructions / Claude Project / Gemini Gems: paste output of `xgram memory export-prompt` into the system prompt.\n\
\n\
[Step 4: per-goal extras]\n\
- (a) memory: `xgram session new`, `xgram session message`, `xgram session recall`\n\
- (b) notify: `xgram notify setup-telegram` or `setup-discord` (interactive wizard)\n\
- (c) multi-agent: `xgram channel serve --bind 127.0.0.1:7250`\n\
- (d) payment: `xgram payment new --amount 1.0 --chain base --to 0x…`\n\
- (e) backup: `xgram backup create`, `xgram identity did`\n\
\n\
[Step 5: smoke test]\n\
- `xgram session new --title \"first chat\"`\n\
- `xgram session message --session-id <ID> --sender me --body \"hello\"`\n\
- `xgram session recall --query \"hello\" --k 5`\n\
- If recall returns the message, you're done in under 5 minutes.\n\
\n\
[Rules]\n\
- Reply in English (Korean if the user opens in Korean).\n\
- Never suggest commands incompatible with the user's OS (no `sh` on Windows).\n\
- Stress that all data lives only on the user's machine.\n\
- Avoid jargon — plain language.\n\
- Walk through one step at a time (no info dumps).\n\
- If the user reports an error, diagnose with exact follow-up commands.\n\
- Never recommend external cloud memory services (Mem0, Zep, etc.).\n\
\n\
Now ask the three questions in step 1 — all at once.\n";

pub fn run_onboard_prompt(lang: OnboardLang, copy: bool) -> Result<()> {
    let body = match lang {
        OnboardLang::Ko => ONBOARD_PROMPT_KO.to_string(),
        OnboardLang::En => ONBOARD_PROMPT_EN.to_string(),
        OnboardLang::Both => format!(
            "{}\n\n=========================\n\n{}",
            ONBOARD_PROMPT_KO, ONBOARD_PROMPT_EN
        ),
    };

    if copy {
        // 절대 보장 원칙 — 사용자는 어떤 환경에서도 프롬프트를 손에 받는다.
        // 3 경로 모두 시도하고, 결과를 명시한다. silent 실패 0.
        //   1) 파일 저장 (~/.openxgram/onboard-prompt.txt) — 디스크/HOME 정상이면 항상 성공
        //   2) 시스템 클립보드 — arboard → 외부 도구 (UI 환경에서 즉시 paste 가능)
        //   3) 둘 다 실패 시 stdout — 사용자가 터미널에서 직접 복사 (마지막 안전망)
        let saved = try_save_to_file(&body).unwrap_or(None);
        let clipboard_ok = match try_copy_arboard(&body) {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!(error = %e, "arboard 실패 — 외부 도구 시도");
                try_copy_external(&body).unwrap_or(false)
            }
        };

        // 결과 안내 (eprintln — stderr, paste 시 본문에 안 섞임)
        if clipboard_ok {
            eprintln!("✓ 클립보드에 복사됨. 좋아하는 AI 에 Cmd/Ctrl+V 로 붙여넣기.");
        } else {
            eprintln!("ℹ 클립보드 접근 불가 (헤드리스 환경 또는 클립보드 매니저 부재).");
        }
        if let Some(path) = &saved {
            eprintln!("✓ 파일 저장: {}", path.display());
            if !clipboard_ok {
                eprintln!(
                    "  수동 복사: macOS `cat {p} | pbcopy` · Linux `cat {p} | xclip -sel c` · WSL `cat {p} | clip.exe`",
                    p = path.display()
                );
            }
        }
        eprintln!("  자세한 사용 페이지: https://openxgram.org/onboard/");

        // 절대 안전망: 클립보드도 파일도 실패하면 stdout 으로 본문 출력 (사용자 직접 복사)
        if !clipboard_ok && saved.is_none() {
            eprintln!("⚠️ 클립보드·파일 모두 접근 불가 — stdout 으로 본문 출력합니다 (직접 복사하세요).");
            eprintln!("---BEGIN ONBOARD PROMPT---");
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(body.as_bytes()).context("stdout write")?;
            eprintln!("---END ONBOARD PROMPT---");
        }
        return Ok(());
    }

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(body.as_bytes()).context("stdout write")?;
    Ok(())
}

/// arboard 로 native 클립보드 복사 (모든 OS 통일된 인터페이스).
fn try_copy_arboard(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| anyhow::anyhow!("arboard init: {e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| anyhow::anyhow!("arboard set_text: {e}"))?;
    Ok(())
}

/// 외부 클립보드 도구 fallback (arboard 가 헤드리스 등에서 실패할 때).
fn try_copy_external(text: &str) -> Result<bool> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let candidates: &[&[&str]] = &[
        &["wl-copy"],
        &["xclip", "-selection", "clipboard", "-l", "1"],
        &["xsel", "--clipboard", "--input"],
        &["pbcopy"],
        &["clip.exe"],
    ];
    for cmd in candidates {
        let mut child = match Command::new(cmd[0]).args(&cmd[1..]).stdin(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 마지막 폴백 — ~/.openxgram/onboard-prompt.txt 로 저장. 항상 작동하도록 보장.
fn try_save_to_file(text: &str) -> Result<Option<std::path::PathBuf>> {
    let home = match std::env::var_os("HOME") {
        Some(h) => std::path::PathBuf::from(h),
        None => return Ok(None),
    };
    let dir = home.join(".openxgram");
    if std::fs::create_dir_all(&dir).is_err() {
        return Ok(None);
    }
    let path = dir.join("onboard-prompt.txt");
    std::fs::write(&path, text).context("onboard-prompt.txt write")?;
    Ok(Some(path))
}
