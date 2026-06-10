import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { invoke } from "@/api/client";

// tmux 라이브 열기 — 선택 tmux 세션의 화면(capture-pane)을 폴링 표시 + 입력 주입(send-keys).
// 백엔드: GET /sessions/{id}/screen (SessionScreenDto.content, ANSI 포함) · POST /sessions/{id}/input ({data}).

type ScreenDto = { content: string; lines?: number; source_note?: string; fetched_at?: string };

const ESC = String.fromCharCode(27);
const CTRL_C = String.fromCharCode(3);

// ANSI escape 제거(가독성용 — xterm 미사용 경량 뷰).
function stripAnsi(s: string): string {
  const esc = String.fromCharCode(27);
  const re = new RegExp(esc + "\\[[0-9;?]*[ -/]*[@-~]", "g");
  const osc = new RegExp(esc + "\\][^" + String.fromCharCode(7) + "]*" + String.fromCharCode(7), "g");
  return s.replace(re, "").replace(osc, "");
}

export function TmuxLiveModal(props: { identifier: string; display: string; onClose: () => void }) {
  const [content, setContent] = createSignal("불러오는 중…");
  const [err, setErr] = createSignal<string | null>(null);
  const [note, setNote] = createSignal("");
  const [cmd, setCmd] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  let timer: ReturnType<typeof setInterval> | undefined;
  let preRef: HTMLPreElement | undefined;

  async function refresh() {
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
  async function send(data: string) {
    setBusy(true);
    try {
      await invoke("session_input", { identifier: props.identifier, data });
      await refresh();
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }
  function submitCmd() {
    const c = cmd();
    setCmd("");
    void send(c + "\n");
  }

  onMount(() => {
    void refresh();
    timer = setInterval(() => void refresh(), 1500);
  });
  onCleanup(() => { if (timer) clearInterval(timer); });

  return (
    <div class="ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="kk-tmux-modal">
        <div class="kk-tmux-head">
          <span class="t">🖥 {props.display}</span>
          <span class="kk-tmux-note">{note()}</span>
          <span class="x" onClick={() => props.onClose()}>✕</span>
        </div>
        <Show when={err()}>
          <div class="kk-tmux-err">⚠ {err()}</div>
        </Show>
        <pre class="kk-tmux-screen" ref={preRef}>{content()}</pre>
        <div class="kk-tmux-input">
          <input
            placeholder="명령 입력 후 Enter (tmux send-keys)…"
            value={cmd()}
            disabled={busy()}
            onInput={(e) => setCmd(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); submitCmd(); } }}
          />
          <button class="kk-tmux-key" disabled={busy()} title="Ctrl-C" onClick={() => void send(CTRL_C)}>^C</button>
          <button class="kk-tmux-key" disabled={busy()} title="위 방향키" onClick={() => void send(ESC + "[A")}>↑</button>
          <button class="kk-tmux-key" disabled={busy()} title="Enter" onClick={() => void send("\n")}>⏎</button>
          <button class="kk-tmux-send" disabled={busy()} onClick={() => submitCmd()}>전송</button>
        </div>
      </div>
    </div>
  );
}
