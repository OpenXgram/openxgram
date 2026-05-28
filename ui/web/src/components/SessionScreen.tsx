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
 const [inputMode, setInputMode] = createSignal(true);  // 기본 ON (마스터 요청)
 // keystroke batch — 매 키마다 HTTP request 면 병렬 race 로 글자 순서 깨짐.
 // 50ms 안에 들어온 keystroke 모아 한 번 송신.
 let inputBuf = "";
 let inputTimer: number | undefined;
 const flushInput = async () => {
 if (!inputBuf) return;
 const data = inputBuf; inputBuf = "";
 try {
 await invoke("session_input", { identifier: props.identifier, data});
 void refresh();
 } catch (e) { setError("input 실패: " + e); }
 };
 onMount(() => {
 if (!containerRef) return;
 term = new Terminal({
 fontSize: 14,           // 12 → 14 가독성
 lineHeight: 1.25,        // 기본 1.0 → 줄 간격 25% 확장
 letterSpacing: 0.5,      // 기본 0 → 글자 간격 약간 (빾빽함 해소)
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
 // 파일 우선 — attachments endpoint 로 hash 저장 + tmux 에 마커 송신.
 // binary/대용량 안전 처리 (이전: f.text() 만 → text 파일만 가능했음).
 if (dt.files && dt.files.length > 0) {
 const f = dt.files[0];
 try {
 const buf = await f.arrayBuffer();
 // base64 encode (Blob → FileReader 가 더 빠르지만 단순화).
 const bytes = new Uint8Array(buf);
 let bin = "";
 for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
 const content_b64 = btoa(bin);
 const r = await invoke<any>("attachment_upload", {
 content_b64,
 mime: f.type || "application/octet-stream",
 filename: f.name,
 });
 const hash = r.content_hash || r.hash || "";  // backend 는 content_hash 반환
 const marker = `📎 [attached: ${f.name} · ${(f.size/1024).toFixed(1)}KB · hash:${hash.slice(0,12)}]\n`;
 await invoke("session_input", { identifier: props.identifier, data: marker});
 } catch (er) { setError("drop file 업로드 실패: " + er);}
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
 // 입력 모드 ON 일 때 onData buffer 에 모아 50ms debounce 송신.
 // 매 keystroke 별 HTTP invoke 는 병렬 race → 스페이스/한글 조합 깨짐 (마스터 보고).
 // 예외: \r 또는 \n (Enter) 들어오면 timer 안 기다리고 즉시 flush — Enter 반응성.
 term.onData((data: string) => {
 if (!inputMode()) return;
 inputBuf += data;
 if (inputTimer) clearTimeout(inputTimer);
 if (data.includes("\r") || data.includes("\n")) {
 // Enter — 즉시 flush (debounce 안 함)
 void flushInput();
 } else {
 inputTimer = window.setTimeout(() => { void flushInput(); }, 50);
 }
 });
 void refresh();
 // 폴링 600ms (이전 2000ms) — idle 시에도 화면 변화 빠르게.
 pollTimer = window.setInterval(() => void refresh(), 600);
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
 {props.identifier}
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
