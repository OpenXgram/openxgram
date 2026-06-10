import { createResource, Show } from "solid-js";
import { invoke } from "@/api/client";
import { AcpConversation, aiTypeToAdapter, type AcpPreset } from "./AcpConversation";

// 대화창만 별도 창으로 — App 이 `?chat=<alias>` 를 감지하면 KakaoShell/앱크롬 없이 이 컴포넌트만 렌더.
// alias 로 agents_list 에서 에이전트를 찾아 preset(어댑터/cwd/실행모드)을 복원하고 AcpConversation 을 단독 구동.
// 대화 영속화(acp_messages)가 있으므로 이 창은 진입 즉시 이전 대화를 복원한다.

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

  return (
    <div class="kk-popup-root">
      <Show
        when={preset() !== undefined}
        fallback={<div class="kk-popup-loading">대화 불러오는 중…</div>}
      >
        {/* 닫기 = 창 닫기. 팝업이므로 popoutAlias 미전달(중첩 버튼 방지). */}
        <AcpConversation preset={preset()} onClose={() => window.close()} />
      </Show>
    </div>
  );
}
