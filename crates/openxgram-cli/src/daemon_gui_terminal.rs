//! rc.339 — 인증된 인터랙티브 터미널 브리지 (WebSocket + PTY → tmux attach).
//!
//! 보안 민감 기능: 웹 GUI 를 통해 로컬 tmux 세션에 **셸 제어**를 제공한다.
//! 따라서 read-only `session_screen`(capture-pane) 와 달리 다음 가드를 강제한다.
//!
//! ## 보안 가드 (security review 대상 — 절대 완화 금지)
//!  1. **AUTH 필수** — WS upgrade 전에 `verify_terminal_auth` 로 토큰 검증. 브라우저
//!     WebSocket 은 Authorization 헤더를 못 싣기에 `?token=<bearer>` 쿼리로 받되,
//!     검증 로직은 require_auth 와 동일(session_token 또는 mcp-token). 익명 허용 없음.
//!     (read-only screen 의 tailnet-anonymous 경로를 여기서는 **재사용하지 않는다**.)
//!  2. **세션 id 정제 + 존재 검증** — `kill_tmux_session` 과 동일 규칙: 영숫자·`_`·`-`·`.`
//!     만 허용(셸 메타·공백·`:` 거부) + `tmux has-session -t =<name>` 정확매칭.
//!  3. **로컬 tmux only** — peer:/portal:/aoe:/claude:/proc: 거부. 원격·외부 세션 미지원.
//!  4. **injection 불가** — tmux 실행은 `CommandBuilder` 인자 배열(셸 보간 X).
//!  5. **감사 로그** — 터미널 open(누가=auth subject, 어느 세션) + close 를 tracing 기록.
//!
//! ## 브리지 방식 — PTY + `tmux attach-session` (send-keys+capture 대신)
//!  진짜 인터랙티브 터미널(실시간 ANSI·커서·resize)을 위해 `portable-pty` 로
//!  `tmux attach-session -t =<name>` 를 PTY 안에서 실행한다. PTY 출력 바이트를
//!  그대로 WS 로 흘리고, WS 로 들어온 키 입력을 PTY 에 쓴다(= ttyd 와 동일한 모델,
//!  단 AUTH 강제). attach 라서 사용자가 보는 화면 = 실제 tmux 화면(detach 시 세션 유지).

use std::io::{Read, Write};

use axum::extract::ws::{Message, WebSocket};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

/// 세션 id 정제 + 로컬 tmux 범위 게이트. `Ok(name)` = 검증된 bare tmux 세션명.
/// `Err(reason)` = 거부 사유(WS open 전 close 코드/메시지로 전달).
///
/// `kill_tmux_session` 의 가드와 동일 규칙(중복 로직이지만 범위가 명확히 다르고,
/// kill 은 파괴/terminal 은 셸제어라 각자 명시 검증을 유지 — 우회 위험 차단).
pub fn validate_terminal_target(identifier: &str) -> Result<String, String> {
    // 범위 게이트 — 로컬 tmux 만. 원격/외부 세션 거부.
    if identifier.starts_with("peer:")
        || identifier.starts_with("portal:")
        || identifier.starts_with("aoe:")
        || identifier.starts_with("claude:")
        || identifier.starts_with("proc:")
    {
        return Err(
            "터미널은 로컬 tmux 세션만 지원합니다 (peer/portal/aoe/claude/proc 미지원)".into(),
        );
    }
    // tmux:<name>[:window] → <name>. bare 도 허용.
    let name = identifier
        .strip_prefix("tmux:")
        .unwrap_or(identifier)
        .split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err("빈 세션 이름".into());
    }
    if name.len() > 128 {
        return Err("세션 이름이 너무 김 (>128)".into());
    }
    // 허용 문자만 — injection 방지. 공백·';'·'|'·'$'·'`'·'&'·'/'·':' 등 거부.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(format!(
            "허용되지 않는 문자 포함 (영숫자·_-. 만 허용): {name}"
        ));
    }
    // 존재 검증 — `=` prefix 정확매칭(부분일치·패턴 금지).
    let target = format!("={name}");
    let exists = std::process::Command::new("tmux")
        .args(["has-session", "-t", &target])
        .output()
        .map_err(|e| format!("tmux 실행 실패 (미설치?): {e}"))?;
    if !exists.status.success() {
        return Err(format!("존재하지 않는 tmux 세션: {name}"));
    }
    Ok(name)
}

/// 클라이언트 → 서버 제어 프레임. 키 입력은 raw text(WS Text)로 받고, resize 는
/// JSON 제어 메시지(`{"t":"resize","cols":N,"rows":M}`)로 받는다. 키 입력 자체가
/// JSON 처럼 보일 일은 드물지만, 안전을 위해 resize 만 JSON 파싱하고 실패하면 키로 취급.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "t")]
enum CtrlFrame {
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
}

/// PTY-owner 블로킹 스레드로 보내는 입력 명령. PTY 핸들(`MasterPty`/writer)은 `Send`지만
/// `Sync` 아님 → async future 가 `Send` 가 되려면 PTY 핸들을 await 너머로 들지 않아야 한다.
/// 그래서 PTY 를 소유하는 전용 블로킹 스레드를 두고, async 측은 이 채널로만 통신한다.
enum PtyInput {
    /// 키 입력 바이트 → PTY write.
    Data(Vec<u8>),
    /// 터미널 리사이즈 (ioctl).
    Resize { cols: u16, rows: u16 },
}

/// WS 업그레이드 후 본체 — PTY(tmux attach) ↔ WebSocket 양방향 펌프.
///
/// `auth_subject` = 감사 로그용(누가 열었는지). `tmux_name` = 검증된 bare 세션명.
pub async fn run_terminal_bridge(socket: WebSocket, tmux_name: String, auth_subject: String) {
    tracing::warn!(
        session = %tmux_name,
        subject = %auth_subject,
        "GUI 인터랙티브 터미널 OPEN (셸 제어 — 인증됨)"
    );

    // PTY 생성 + tmux attach 실행 (blocking 작업 → spawn_blocking 으로 격리).
    let pty_pair = match tokio::task::spawn_blocking(|| {
        let pty_system = NativePtySystem::default();
        pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
    })
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            let _ = close_with(socket, &format!("PTY 생성 실패: {e}")).await;
            return;
        }
        Err(e) => {
            let _ = close_with(socket, &format!("PTY spawn 실패: {e}")).await;
            return;
        }
    };

    // tmux attach-session -t =<name>. `=` 정확매칭. 인자 배열 → 셸 보간 없음.
    let target = format!("={tmux_name}");
    let mut cmd = CommandBuilder::new("tmux");
    cmd.args(["attach-session", "-t", &target]);
    // TERM 명시 — xterm.js 와 호환.
    cmd.env("TERM", "xterm-256color");

    let mut child = match pty_pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            let _ = close_with(socket, &format!("tmux attach spawn 실패: {e}")).await;
            return;
        }
    };
    // slave 핸들은 spawn 후 drop (master 만 보유).
    drop(pty_pair.slave);

    // PTY reader/writer 추출.
    let mut reader = match pty_pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = close_with(socket, &format!("PTY reader 실패: {e}")).await;
            let _ = child.kill();
            return;
        }
    };
    let mut writer = match pty_pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            let _ = close_with(socket, &format!("PTY writer 실패: {e}")).await;
            let _ = child.kill();
            return;
        }
    };

    // ── PTY → WS (출력 펌프). blocking read 를 mpsc 채널로 브릿지(전용 OS 스레드). ──
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
    let reader_handle = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF (tmux detach/세션 종료)
                Ok(n) => {
                    if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break; // 수신측 drop
                    }
                }
                Err(_) => break,
            }
        }
    });

    // ── PTY-owner 전용 스레드: master(resize)+writer(input) 단독 소유. async 측은 채널로만 통신
    //   → async future 가 PTY 핸들(Send 지만 non-Sync)을 await 너머로 들지 않아 `Send` 유지
    //   (axum on_upgrade 의 Future: Send 바운드 충족). ──
    let (in_tx, in_rx) = std::sync::mpsc::channel::<PtyInput>();
    let master = pty_pair.master; // 이 스레드가 단독 소유.
    let pty_owner = std::thread::spawn(move || {
        while let Ok(cmd) = in_rx.recv() {
            match cmd {
                PtyInput::Data(bytes) => {
                    let _ = writer.write_all(&bytes);
                    let _ = writer.flush();
                }
                PtyInput::Resize { cols, rows } => {
                    let _ = master.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
        }
        // 채널 닫힘 → drop(master)/drop(writer). attach 종료(세션은 유지).
    });

    use futures_util::{SinkExt, StreamExt};
    let (mut ws_tx, mut ws_rx) = socket.split();

    // ── 출력 task: PTY bytes → WS Binary ──
    let out_task = tokio::spawn(async move {
        while let Some(chunk) = out_rx.recv().await {
            if ws_tx.send(Message::Binary(chunk.into())).await.is_err() {
                break;
            }
        }
        // PTY EOF → WS close.
        let _ = ws_tx.send(Message::Close(None)).await;
    });

    // ── 입력 루프: WS → PTY-owner 채널 (현재 task; PTY 핸들 미보유 → future Send). ──
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(txt) => {
                // resize 제어 프레임이면 PTY resize, 아니면 키 입력으로 PTY write.
                let txt_str: &str = txt.as_str();
                if let Ok(CtrlFrame::Resize { cols, rows }) =
                    serde_json::from_str::<CtrlFrame>(txt_str)
                {
                    if in_tx.send(PtyInput::Resize { cols, rows }).is_err() {
                        break;
                    }
                } else if in_tx
                    .send(PtyInput::Data(txt_str.as_bytes().to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Message::Binary(bin) => {
                if in_tx.send(PtyInput::Data(bin.to_vec())).is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // ── 정리 — detach(child kill) + 채널 닫기로 PTY-owner 스레드 종료. tmux 세션은 유지. ──
    let _ = child.kill();
    drop(in_tx); // pty_owner 스레드 종료 신호(drop master/writer).
    out_task.abort();
    let _ = reader_handle.join();
    let _ = pty_owner.join();
    tracing::info!(
        session = %tmux_name,
        subject = %auth_subject,
        "GUI 인터랙티브 터미널 CLOSE (detach — 세션은 유지)"
    );
}

/// WS open 실패 시 close 프레임으로 사유 전달(절대 규칙: 조용한 폴백 금지).
async fn close_with(socket: WebSocket, reason: &str) -> Result<(), axum::Error> {
    use futures_util::SinkExt;
    let mut s = socket;
    let _ = s
        .send(Message::Text(format!("\x1b[31m{reason}\x1b[0m\r\n").into()))
        .await;
    s.send(Message::Close(None)).await
}
