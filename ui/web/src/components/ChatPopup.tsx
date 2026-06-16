import { createResource, createSignal, onCleanup, onMount, Show } from "solid-js";
import { invoke } from "@/api/client";
import { AcpConversation, aiTypeToAdapter, type AcpPreset } from "./AcpConversation";

// 대화창만 별도 창으로 — App 이 `?chat=<alias>` 를 감지하면 KakaoShell/앱크롬 없이 이 컴포넌트만 렌더.
// alias 로 agents_list 에서 에이전트를 찾아 preset(어댑터/cwd/실행모드)을 복원하고 AcpConversation 을 단독 구동.
// 대화 영속화(acp_messages)가 있으므로 이 창은 진입 즉시 이전 대화를 복원한다.
//
// AUTO-POP (rc.334) — 이 창은 A2A 협업 곁뷰에서 자동/수동으로 열린 팝업이다.
//   포커스 탈취 금지: 새 메시지가 와도 절대 window.focus() 를 호출하지 않는다.
//   대신 document.title 을 깜빡여(💬(N) <alias> ↔ <alias>) 활동을 알린다.
//   창이 다시 보이거나 포커스를 받으면 깜빡임을 멈추고 제목을 복원한다.

type AgentRow = {
  alias: string;
  ai_type?: string | null;
  project_path?: string | null;
  execution_mode?: string | null;
};

export function ChatPopup(props: { alias: string }) {
  const [preset] = createResource<AcpPreset | null>(async () => {
    try {
      const r = await invoke<AgentRow[]>("agents_list");
      const a = (Array.isArray(r) ? r : []).find((x) => x.alias === props.alias);
      if (!a) {
        // 명부에 없으면 alias 를 라벨로, 기본 어댑터로라도 진입(영속 대화는 alias 키로 복원됨).
        return { adapter: "claude-agent-acp", cwd: null, execMode: null, label: props.alias };
      }
      return {
        adapter: aiTypeToAdapter(a.ai_type),
        cwd: a.project_path ?? null,
        execMode: a.execution_mode ?? null,
        label: a.alias,
      };
    } catch {
      return { adapter: "claude-agent-acp", cwd: null, execMode: null, label: props.alias };
    }
  });

  // ── 포커스 탈취 없는 활동 알림: document.title 깜빡임 ──
  // unread 카운트를 올리고 0.9s 간격으로 제목을 토글한다. 절대 .focus() 안 함.
  const baseTitle = () => `💬 ${props.alias}`;
  const [unread, setUnread] = createSignal(0);
  let blinkTimer: number | undefined;
  let blinkOn = false;

  function setDocTitle(t: string) {
    try { document.title = t; } catch { /* ignore */ }
  }
  function stopBlink() {
    if (blinkTimer !== undefined) { clearInterval(blinkTimer); blinkTimer = undefined; }
    blinkOn = false;
    setUnread(0);
    setDocTitle(baseTitle());
  }
  function startBlink() {
    if (blinkTimer !== undefined) return;
    blinkTimer = window.setInterval(() => {
      blinkOn = !blinkOn;
      const n = unread();
      // 깜빡임: "💬(N) <alias>" ↔ "<alias>" — 포커스 없이 제목만 토글.
      setDocTitle(blinkOn ? `💬(${n}) ${props.alias}` : props.alias);
    }, 900);
  }
  // 새 버블 도착(에이전트/상대 활동) → 창이 보이지 않거나 포커스 없으면 깜빡임 시작.
  function onActivity() {
    const focused = (() => { try { return document.hasFocus() && document.visibilityState === "visible"; } catch { return true; } })();
    if (focused) { setDocTitle(baseTitle()); return; } // 이미 보고 있으면 알림 불필요.
    setUnread((n) => n + 1);
    startBlink();
  }

  onMount(() => {
    setDocTitle(baseTitle());
    // 창이 다시 보이거나 포커스를 받으면 깜빡임 정지 + 제목 복원. (사용자가 능동적으로 본 것 — focus 호출 아님)
    const onFocus = () => stopBlink();
    const onVis = () => { if (document.visibilityState === "visible") stopBlink(); };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVis);
    onCleanup(() => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVis);
      if (blinkTimer !== undefined) clearInterval(blinkTimer);
    });
  });

  return (
    <div class="kk-popup-root">
      <Show
        when={preset() !== undefined}
        fallback={<div class="kk-popup-loading">대화 불러오는 중…</div>}
      >
        {/* 닫기 = 창 닫기. 팝업이므로 popoutAlias 미전달(중첩 버튼 방지).
            onNewBubble → title-blink (포커스 탈취 없음). 사람 컴포저는 AcpConversation 이 그대로 렌더. */}
        <AcpConversation
          preset={preset()}
          onClose={() => window.close()}
          onNewBubble={() => onActivity()}
        />
      </Show>
    </div>
  );
}
