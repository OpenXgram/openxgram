import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { invoke } from "@/api/client";

// tmux 라이브 — 선택 tmux 세션 화면(capture-pane)을 폴링 표시 + 입력 주입(send-keys).
// 새 창(?tmux=<identifier>)에서 풀윈도우로 렌더. 백엔드: GET /sessions/{id}/screen · POST /sessions/{id}/input.

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
  const [content, setContent] = createSignal("불러오는 중…");
  const [err, setErr] = createSignal<string | null>(null);
  const [note, setNote] = createSignal("");
  const [cmd, setCmd] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [dragOver, setDragOver] = createSignal(false);
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

  // 파일 → base64 (data URL 의 콤마 뒤 부분).
  function fileToB64(f: File): Promise<string> {
    return new Promise((res, rej) => {
      const r = new FileReader();
      r.onload = () => { const s = r.result as string; res(s.slice(s.indexOf(",") + 1)); };
      r.onerror = () => rej(r.error);
      r.readAsDataURL(f);
    });
  }
  // 드래그드롭 — 파일을 서버 <data_dir>/drops/ 에 저장하고 절대경로를 입력창에 삽입.
  // 같은 머신의 tmux 에이전트가 그 경로로 파일을 바로 읽는다.
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
        if (r?.path) setCmd((c) => (c ? c + " " : "") + r.path);
      }
    } catch (e2) {
      setErr((e2 as Error)?.message ?? String(e2));
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    void refresh();
    timer = setInterval(() => void refresh(), 1500);
  });
  onCleanup(() => { if (timer) clearInterval(timer); });

  return (
    <div
      class={`kk-tmux-full${dragOver() ? " dragover" : ""}`}
      onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
      onDragLeave={(e) => { e.preventDefault(); setDragOver(false); }}
      onDrop={onDrop}
    >
      <Show when={dragOver()}>
        <div class="kk-tmux-drop">📎 파일을 놓으면 서버에 저장하고 경로를 입력창에 넣어요</div>
      </Show>
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
        <button class="kk-tmux-key" disabled={busy()} title="Esc (메뉴 취소)" onClick={() => void send(ESC)}>Esc</button>
        <button class="kk-tmux-key" disabled={busy()} title="위 방향키" onClick={() => void send(ESC + "[A")}>↑</button>
        <button class="kk-tmux-key" disabled={busy()} title="아래 방향키" onClick={() => void send(ESC + "[B")}>↓</button>
        <button class="kk-tmux-key" disabled={busy()} title="Enter" onClick={() => void send("\n")}>⏎</button>
        <button class="kk-tmux-send" disabled={busy()} onClick={() => submitCmd()}>전송</button>
      </div>
    </div>
  );
}
