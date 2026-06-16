import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { invoke, terminalWsUrl } from "@/api/client";

// rc.339 — tmux 라이브 인터랙티브 터미널.
//   주 경로: 인증된 WebSocket(GET /sessions/{id}/terminal?token=) ↔ 백엔드 PTY(tmux attach).
//             xterm.js 가 실시간 출력 렌더 + 키 입력을 WS 로 그대로 전송(진짜 터미널).
//   폴백: WS 연결 실패/끊김 시 read-only capture-pane 폴링(이전 동작) + send-keys 입력바.
//   tmux attach 라서 detach(창 닫기) 해도 세션은 살아있다.

type ScreenDto = { content: string; lines?: number; source_note?: string; fetched_at?: string };
const ESC = String.fromCharCode(27);
const CTRL_C = String.fromCharCode(3);

function stripAnsi(s: string): string {
  const esc = String.fromCharCode(27);
  const re = new RegExp(esc + "\\[[0-9;?]*[ -/]*[@-~]", "g");
  const osc = new RegExp(esc + "\\][^" + String.fromCharCode(7) + "]*" + String.fromCharCode(7), "g");
  return s.replace(re, "").replace(osc, "");
}

export function TmuxLive(props: { identifier: string; display: string; onClose: () => void }) {
  // ── 연결 상태: "connecting" | "live"(WS) | "fallback"(capture 폴링) ──
  const [mode, setMode] = createSignal<"connecting" | "live" | "fallback">("connecting");
  const [err, setErr] = createSignal<string | null>(null);
  const [note, setNote] = createSignal("");
  // 폴백(capture) 상태
  const [content, setContent] = createSignal("연결 중…");
  const [cmd, setCmd] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [dragOver, setDragOver] = createSignal(false);

  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let preRef: HTMLPreElement | undefined; // 폴백 pre
  let termRef: HTMLDivElement | undefined; // xterm 컨테이너

  // xterm + WS 핸들
  let term: Terminal | undefined;
  let fit: FitAddon | undefined;
  let ws: WebSocket | undefined;
  let resizeObs: ResizeObserver | undefined;
  let closedByUser = false;

  // ── 폴백 경로(read-only capture + send-keys) ──
  async function refreshCapture() {
    try {
      const r = await invoke<ScreenDto>("session_screen", { identifier: props.identifier });
      setErr(null);
      setContent(stripAnsi(r.content ?? ""));
      setNote(`${r.source_note ?? ""}${r.fetched_at ? " · " + r.fetched_at.slice(11, 19) : ""}`);
      queueMicrotask(() => { if (preRef) preRef.scrollTop = preRef.scrollHeight; });
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    }
  }
  async function sendKeys(data: string) {
    setBusy(true);
    try {
      await invoke("session_input", { identifier: props.identifier, data });
      await refreshCapture();
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }
  function submitCmd() {
    const c = cmd();
    setCmd("");
    void sendKeys(c + "\n");
  }
  function startFallbackPolling() {
    if (pollTimer) return;
    void refreshCapture();
    pollTimer = setInterval(() => void refreshCapture(), 1500);
  }
  function stopFallbackPolling() {
    if (pollTimer) { clearInterval(pollTimer); pollTimer = undefined; }
  }

  // ── 주 경로: 인증된 WS + xterm.js ──
  function sendResize() {
    if (!term || !ws || ws.readyState !== WebSocket.OPEN) return;
    try {
      ws.send(JSON.stringify({ t: "resize", cols: term.cols, rows: term.rows }));
    } catch { /* noop */ }
  }
  function fitNow() {
    if (!fit || !term) return;
    try { fit.fit(); sendResize(); } catch { /* noop */ }
  }

  function connectWs() {
    const url = terminalWsUrl(props.identifier);
    if (!url) {
      // 토큰 없음 → 폴백.
      setMode("fallback");
      setErr("인증 토큰 없음 — read-only 미러로 표시(입력은 send-keys)");
      startFallbackPolling();
      return;
    }
    // xterm 초기화 (한 번).
    if (!term && termRef) {
      term = new Terminal({
        convertEol: false,
        cursorBlink: true,
        fontSize: 13,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
        theme: { background: "#0b0e14", foreground: "#d6deeb" },
        scrollback: 5000,
      });
      fit = new FitAddon();
      term.loadAddon(fit);
      term.open(termRef);
      // 키 입력 → WS 로 raw 전송(진짜 터미널). resize 제어 프레임과 충돌 없음(키는 JSON 아님).
      term.onData((d) => {
        if (ws && ws.readyState === WebSocket.OPEN) ws.send(d);
      });
      requestAnimationFrame(() => fitNow());
    }

    let sock: WebSocket;
    try {
      sock = new WebSocket(url);
    } catch (e) {
      setMode("fallback");
      setErr(`WS 생성 실패 — read-only 폴백: ${(e as Error).message}`);
      startFallbackPolling();
      return;
    }
    sock.binaryType = "arraybuffer";
    ws = sock;

    sock.onopen = () => {
      setMode("live");
      setErr(null);
      setNote("인터랙티브 (PTY · tmux attach · 인증됨)");
      stopFallbackPolling();
      requestAnimationFrame(() => fitNow());
    };
    sock.onmessage = (ev) => {
      if (!term) return;
      if (typeof ev.data === "string") {
        term.write(ev.data);
      } else {
        // ArrayBuffer → UTF-8 decode 후 write(멀티바이트 안전: TextDecoder stream).
        term.write(new Uint8Array(ev.data));
      }
    };
    sock.onerror = () => {
      // open 전 에러면 폴백으로 강등(인증 실패 등).
      if (mode() !== "live") {
        setMode("fallback");
        setErr("WS 연결 실패(인증/네트워크) — read-only 미러로 표시");
        startFallbackPolling();
      }
    };
    sock.onclose = () => {
      ws = undefined;
      if (closedByUser) return;
      // 라이브 중 끊김 → 폴백으로 강등(세션은 살아있음).
      setMode("fallback");
      setErr("터미널 연결 종료 — read-only 미러로 표시(세션은 유지)");
      startFallbackPolling();
    };
  }

  // 파일 → base64 (data URL 의 콤마 뒤 부분).
  function fileToB64(f: File): Promise<string> {
    return new Promise((res, rej) => {
      const r = new FileReader();
      r.onload = () => { const s = r.result as string; res(s.slice(s.indexOf(",") + 1)); };
      r.onerror = () => rej(r.error);
      r.readAsDataURL(f);
    });
  }
  // 드래그드롭 — 파일을 서버 <data_dir>/drops/ 에 저장하고 절대경로를 삽입.
  //   live 모드면 xterm 에 경로를 직접 타이핑(WS), 폴백이면 입력창에 삽입.
  async function onDrop(e: DragEvent) {
    e.preventDefault();
    setDragOver(false);
    const files = Array.from(e.dataTransfer?.files ?? []);
    if (!files.length) return;
    setBusy(true);
    try {
      for (const f of files) {
        const b64 = await fileToB64(f);
        const r = await invoke<{ path?: string }>("session_dropfile", {
          identifier: props.identifier, filename: f.name, content_b64: b64,
        });
        if (r?.path) {
          if (mode() === "live" && ws && ws.readyState === WebSocket.OPEN) {
            ws.send((cmd() ? " " : "") + r.path);
          } else {
            setCmd((c) => (c ? c + " " : "") + r.path);
          }
        }
      }
    } catch (e2) {
      setErr((e2 as Error)?.message ?? String(e2));
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    connectWs();
    if (typeof ResizeObserver !== "undefined" && termRef) {
      resizeObs = new ResizeObserver(() => fitNow());
      resizeObs.observe(termRef);
    }
  });
  onCleanup(() => {
    closedByUser = true;
    stopFallbackPolling();
    if (resizeObs) resizeObs.disconnect();
    try { ws?.close(); } catch { /* noop */ }
    try { term?.dispose(); } catch { /* noop */ }
  });

  return (
    <div
      class={`kk-tmux-full${dragOver() ? " dragover" : ""}`}
      onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
      onDragLeave={(e) => { e.preventDefault(); setDragOver(false); }}
      onDrop={onDrop}
    >
      <Show when={dragOver()}>
        <div class="kk-tmux-drop">📎 파일을 놓으면 서버에 저장하고 경로를 넣어요</div>
      </Show>
      <div class="kk-tmux-head">
        <span class="t">🖥 {props.display}</span>
        <span class="kk-tmux-note">
          {mode() === "live" ? "🟢 인터랙티브" : mode() === "connecting" ? "⏳ 연결 중" : "🟡 미러(read-only)"} · {note()}
        </span>
        <span class="x" onClick={() => props.onClose()}>✕</span>
      </div>
      <Show when={err()}>
        <div class="kk-tmux-err">⚠ {err()}</div>
      </Show>

      {/* 주 경로: xterm.js 인터랙티브 터미널 (live 또는 connecting 일 때 표시·입력 가능) */}
      <div
        ref={termRef}
        class="kk-tmux-xterm"
        style={{
          display: mode() === "fallback" ? "none" : "block",
          flex: "1 1 auto",
          "min-height": "0",
          padding: "6px",
          background: "#0b0e14",
        }}
      />

      {/* 폴백: read-only capture 미러 + send-keys 입력바 */}
      <Show when={mode() === "fallback"}>
        <pre class="kk-tmux-screen" ref={preRef}>{content()}</pre>
        <div class="kk-tmux-input">
          <input
            placeholder="명령 입력 후 Enter (tmux send-keys · 미러 모드)…"
            value={cmd()}
            disabled={busy()}
            onInput={(e) => setCmd(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); submitCmd(); } }}
          />
          <button class="kk-tmux-key" disabled={busy()} title="Ctrl-C" onClick={() => void sendKeys(CTRL_C)}>^C</button>
          <button class="kk-tmux-key" disabled={busy()} title="Esc" onClick={() => void sendKeys(ESC)}>Esc</button>
          <button class="kk-tmux-key" disabled={busy()} title="위" onClick={() => void sendKeys(ESC + "[A")}>↑</button>
          <button class="kk-tmux-key" disabled={busy()} title="아래" onClick={() => void sendKeys(ESC + "[B")}>↓</button>
          <button class="kk-tmux-key" disabled={busy()} title="Enter" onClick={() => void sendKeys("\n")}>⏎</button>
          <button class="kk-tmux-send" disabled={busy()} onClick={() => submitCmd()}>전송</button>
        </div>
      </Show>
    </div>
  );
}
