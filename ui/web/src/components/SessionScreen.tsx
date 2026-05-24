import { createEffect, createSignal, onCleanup, onMount} from "solid-js";
import { Terminal} from "@xterm/xterm";
import { FitAddon} from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { invoke} from "@/api/client";

// UI-MESSENGER-SPEC v1.3 §4.3 (S5) — 세션 클릭 시 중앙 라이브 터미널.
// tmux 면 capture-pane -e (ANSI), claude_project 면 .jsonl tail.
// 2초 폴링 → xterm.js writeUtf8.

interface SessionScreenDto {
 identifier: string;
 kind: string;
 display: string;
 content: string;
 lines: number;
 source_note: string;
 fetched_at: string;
}

export function SessionScreen(props: { identifier: string; display: string}) {
 let containerRef: HTMLDivElement | undefined;
 const [error, setError] = createSignal<string | null>(null);
 const [meta, setMeta] = createSignal<SessionScreenDto | null>(null);
 let term: Terminal | undefined;
 let fit: FitAddon | undefined;
 let pollTimer: number | undefined;
 let lastContent = "";

 async function refresh() {
 try {
 const dto = await invoke<SessionScreenDto>("session_screen", {
 identifier: props.identifier,
});
 setMeta(dto);
 setError(null);
 // 단순 전략: 매 polling 마다 clear + write 전체. xterm 자체 scrollback 에서 보존.
 if (dto.content !== lastContent && term) {
 term.clear();
 term.write(dto.content.replace(/\n/g, "\r\n"));
 lastContent = dto.content;
}
} catch (e) {
 setError(String(e));
}
}

 let resizeObs: ResizeObserver | undefined;
 const [inputMode, setInputMode] = createSignal(false);
 onMount(() => {
 if (!containerRef) return;
 term = new Terminal({
 fontSize: 12,
 fontFamily: 'ui-monospace, "SF Mono", Menlo, monospace',
 theme: { background: "#0b0f1a", foreground: "#e6e6e6"},
 convertEol: true,
 scrollback: 4000,
 cols: 80,
 rows: 24,
});
 fit = new FitAddon();
 term.loadAddon(fit);
 term.open(containerRef);
 try { fit.fit();} catch {}
 resizeObs = new ResizeObserver(() => { try { fit?.fit();} catch {}});
 resizeObs.observe(containerRef);
 window.setTimeout(() => { try { fit?.fit();} catch {}}, 100);

 // 선택 복사 — Ctrl+C / Cmd+C 누르면 선택된 텍스트를 클립보드로
 term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
 if ((e.ctrlKey || e.metaKey) && (e.key === "c" || e.key === "C")) {
 const sel = term?.getSelection();
 if (sel) {
 navigator.clipboard.writeText(sel).catch(() => {});
 return false; // 기본 처리 막음
 }
 }
 return true;
 });

 // 드래그앤드롭 — 파일/텍스트 drop 시 input 모드 ON 이면 내용을 send-keys
 containerRef.addEventListener("dragover", (e) => { e.preventDefault();});
 containerRef.addEventListener("drop", async (e) => {
 e.preventDefault();
 if (!inputMode()) {
 setError("드래그앤드롭 paste 는 '입력 모드 ON' 일 때만 작동");
 return;
 }
 const dt = e.dataTransfer;
 if (!dt) return;
 // 파일 우선
 if (dt.files && dt.files.length > 0) {
 const f = dt.files[0];
 const text = await f.text().catch(() => "");
 if (text) {
 try {
 await invoke("session_input", { identifier: props.identifier, data: text});
 } catch (er) { setError("drop file send 실패: " + er);}
 }
 return;
 }
 // 텍스트
 const text = dt.getData("text/plain");
 if (text) {
 try {
 await invoke("session_input", { identifier: props.identifier, data: text});
 } catch (er) { setError("drop text send 실패: " + er);}
 }
 });
 // 입력 모드 ON 일 때만 onData → POST /v1/gui/sessions/<id>/input (tmux send-keys -l)
 term.onData(async (data: string) => {
 if (!inputMode()) return;
 try {
 await invoke("session_input", { identifier: props.identifier, data});
 } catch (e) { setError("input 실패: " + e);}
 });
 void refresh();
 pollTimer = window.setInterval(() => void refresh(), 2000);
});

 // identifier 변경 시 (다른 터미널 선택) 즉시 화면 초기화 + refresh
 createEffect(() => {
 const _id = props.identifier;
 lastContent = "";
 if (term) {
 term.clear();
 term.reset();
 }
 void refresh();
 });

 onCleanup(() => {
 if (pollTimer) clearInterval(pollTimer);
 resizeObs?.disconnect();
 term?.dispose();
});

 return (
 <div style="display:flex; flex-direction:column; height:100%;">
 <header style="padding:8px 12px; border-bottom:1px solid var(--border); display:flex; justify-content:space-between; align-items:center; gap:8px;">
 <div style="min-width:0; flex:1;">
 <strong>{props.display}</strong>
 <div style="font-size:11px; color:var(--text-3);">
 {props.identifier} · {meta()?.source_note ?? "loading…"}
 </div>
 </div>
 <label style="font-size:11px; color:var(--text-3); display:flex; align-items:center; gap:4px; cursor:pointer; white-space:nowrap;">
 <input type="checkbox" checked={inputMode()} onChange={(e) => setInputMode(e.currentTarget.checked)} />
 입력 모드 {inputMode() ? "(ON)" : "(읽기 전용)"}
 </label>
 <div style="font-size:11px; color:var(--text-3); white-space:nowrap;">
 {meta()?.lines ?? 0} lines · {meta()?.fetched_at?.slice(11, 19) ?? "—"}
 </div>
 </header>
 <div
 ref={(el) => (containerRef = el)}
 style="flex:1; min-height:300px; background:#0b0f1a; padding:6px;"
 />
 {error() && (
 <div style="padding:6px; color:#f88; font-size:12px;"> {error()}</div>
)}
 </div>
);
}
