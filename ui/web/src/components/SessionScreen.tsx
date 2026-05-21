import { createSignal, onCleanup, onMount} from "solid-js";
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

 onMount(() => {
 if (!containerRef) return;
 term = new Terminal({
 fontSize: 12,
 fontFamily: 'ui-monospace, "SF Mono", Menlo, monospace',
 theme: { background: "#0b0f1a", foreground: "#e6e6e6"},
 convertEol: true,
 scrollback: 4000,
 cols: 120,
 rows: 30,
});
 fit = new FitAddon();
 term.loadAddon(fit);
 term.open(containerRef);
 try {
 fit.fit();
} catch {}
 void refresh();
 pollTimer = window.setInterval(() => void refresh(), 2000);
});

 onCleanup(() => {
 if (pollTimer) clearInterval(pollTimer);
 term?.dispose();
});

 return (
 <div style="display:flex; flex-direction:column; height:100%;">
 <header style="padding:8px 12px; border-bottom:1px solid var(--border); display:flex; justify-content:space-between; align-items:center;">
 <div>
 <strong>{props.display}</strong>
 <div style="font-size:11px; color:var(--text-3);">
 {props.identifier} · {meta()?.source_note ?? "loading…"}
 </div>
 </div>
 <div style="font-size:11px; color:var(--text-3);">
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
