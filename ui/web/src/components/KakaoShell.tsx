import { createSignal, Show } from "solid-js";
import "./kakao.css";
import { ChatTab } from "./ChatTab";
import { MemoryTab } from "./MemoryTab";
import { NetworkTab } from "./NetworkTab";
import { SettingsTab } from "./SettingsTab";

// Phase 1 — 카카오톡 셸. 정본 디자인: _mockups/kakao-mockup.html
// 하단 6탭. 본문은 기존 실제 컴포넌트를 그대로 끼움(대화·위키·에이전트·설정).
// 흐름·마켓은 Phase 4·6에서 네이티브 패널로 교체 (현재 안내 자리).
type KkTab = "agents" | "chat" | "flow" | "wiki" | "market" | "settings";

const TABS: { id: KkTab; ic: string; label: string }[] = [
  { id: "agents", ic: "🙂", label: "에이전트" },
  { id: "chat", ic: "💬", label: "대화" },
  { id: "flow", ic: "🔀", label: "흐름" },
  { id: "wiki", ic: "📚", label: "위키" },
  { id: "market", ic: "🌐", label: "마켓" },
  { id: "settings", ic: "⚙️", label: "설정" },
];

function Placeholder(props: { title: string; phase: string }) {
  return (
    <div style="padding:40px 28px; color:var(--kk-sub); max-width:560px;">
      <h2 style="font-size:17px; font-weight:800; color:var(--kk-ink); margin:0 0 8px;">{props.title}</h2>
      <p style="font-size:13px; line-height:1.6; margin:0;">
        이 화면은 <b>{props.phase}</b>에서 네이티브로 구현됩니다. 디자인 정본은
        <code> _mockups/kakao-mockup.html</code> 이며, 백엔드(엔진·바인딩) 배선 후 이 자리에 들어옵니다.
      </p>
    </div>
  );
}

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
            <div class="kk-embed"><ChatTab onJumpToSettings={() => setTab("settings")} /></div>
          </Show>
          <Show when={tab() === "wiki"}>
            <div class="kk-embed"><MemoryTab /></div>
          </Show>
          <Show when={tab() === "agents"}>
            <div class="kk-embed"><NetworkTab /></div>
          </Show>
          <Show when={tab() === "settings"}>
            <div class="kk-embed"><SettingsTab /></div>
          </Show>
          <Show when={tab() === "flow"}>
            <div class="kk-embed"><Placeholder title="🔀 워크플로우" phase="Phase 4 (엔진 + cron + 하트비트 큐)" /></div>
          </Show>
          <Show when={tab() === "market"}>
            <div class="kk-embed"><Placeholder title="🌐 OpenAgentX 마켓" phase="Phase 6 (마켓 + 지갑 + 수익)" /></div>
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
