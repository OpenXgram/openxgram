import { createSignal, Show } from "solid-js";
import "./kakao.css";
import { TalkTab } from "./TalkTab";
import { WikiTab } from "./WikiTab";
import { ConfigTab } from "./ConfigTab";
import { AgentsTab } from "./AgentsTab";
import { FlowTab } from "./FlowTab";
import { MarketTab } from "./MarketTab";

// Phase 1 — 카카오톡 셸. 정본 디자인: _mockups/kakao-mockup.html
// 하단 6탭. 본문은 기존 실제 컴포넌트를 그대로 끼움(대화·위키·에이전트·설정).
// 흐름은 Phase 4에서 네이티브 패널로 교체 (현재 안내 자리). 마켓은 Phase 6 네이티브(MarketTab).
type KkTab = "agents" | "chat" | "flow" | "wiki" | "market" | "settings";

const TABS: { id: KkTab; ic: string; label: string }[] = [
  { id: "agents", ic: "🙂", label: "에이전트" },
  { id: "chat", ic: "💬", label: "대화" },
  { id: "flow", ic: "🔀", label: "흐름" },
  { id: "wiki", ic: "📚", label: "위키" },
  { id: "market", ic: "🌐", label: "마켓" },
  { id: "settings", ic: "⚙️", label: "설정" },
];

export function KakaoShell(props: { onLogout?: () => void }) {
  const [tab, setTab] = createSignal<KkTab>("chat");

  return (
    <div class="kk">
      <div class="kk-appbar">
        <span class="brand">OpenXgram</span>
        <span class="ver">v{__APP_VERSION__}</span>
        <span class="sp" />
        <Show when={props.onLogout}>
          <button type="button" onClick={() => props.onLogout!()}>잠금</button>
        </Show>
      </div>
      <div class="kk-cols">
      <div class="kk-main">
        <div class="kk-body">
          <Show when={tab() === "chat"}>
            <div class="kk-embed"><TalkTab onJumpToSettings={() => setTab("settings")} /></div>
          </Show>
          <Show when={tab() === "wiki"}>
            <WikiTab />
          </Show>
          <Show when={tab() === "agents"}>
            <div class="kk-embed"><AgentsTab onGotoChat={() => setTab("chat")} /></div>
          </Show>
          <Show when={tab() === "settings"}>
            <ConfigTab />
          </Show>
          <Show when={tab() === "flow"}>
            <FlowTab />
          </Show>
          <Show when={tab() === "market"}>
            <div class="kk-embed"><MarketTab /></div>
          </Show>
        </div>
        <div class="kk-tabs">
          {TABS.map((tb) => (
            <div class={`kk-tab${tab() === tb.id ? " sel" : ""}`} onClick={() => setTab(tb.id)}>
              <span class="ic">{tb.ic}</span>
              {tb.label}
            </div>
          ))}
        </div>
      </div>
      </div>
    </div>
  );
}
