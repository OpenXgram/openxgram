import { createSignal, Show } from "solid-js";
import "./kakao.css";
import { TalkTab } from "./TalkTab";
import { WikiTab } from "./WikiTab";
import { ConfigTab } from "./ConfigTab";
import { AgentsTab } from "./AgentsTab";
import { FlowTab } from "./FlowTab";
import { MarketTab } from "./MarketTab";
import { RuntimeTab } from "./RuntimeTab";

// Phase 1 — 카카오톡 셸. 정본 디자인: _mockups/kakao-mockup.html
// 바텀 나브 = 주요 콘텐츠(에이전트·대화·워크플로우·마켓). 설정은 헤더 기어(⚙️)로 진입하며
// 그 안에 일반/런타임/위키 서브탭. (설정·런타임·위키는 바텀 나브에서 제외 — 마스터 지시)
type KkTab = "agents" | "chat" | "flow" | "market" | "settings";

const TABS: { id: KkTab; ic: string; label: string }[] = [
  { id: "agents", ic: "🙂", label: "에이전트" },
  { id: "chat", ic: "💬", label: "대화" },
  { id: "flow", ic: "🔀", label: "워크플로우" },
  { id: "market", ic: "🌐", label: "마켓" },
];

type SettingsSub = "general" | "runtime" | "wiki";
const SETTINGS_SUB: { id: SettingsSub; ic: string; label: string }[] = [
  { id: "general", ic: "⚙️", label: "일반" },
  { id: "runtime", ic: "🧠", label: "런타임" },
  { id: "wiki", ic: "📚", label: "위키" },
];

export function KakaoShell(props: { onLogout?: () => void }) {
  const [tab, setTab] = createSignal<KkTab>("chat");
  const [sub, setSub] = createSignal<SettingsSub>("general");
  // 대화방(전체화면) 열림 — 카톡처럼 대화방에선 하단 네비를 숨긴다(루트에 kk-room-open 클래스).
  const [roomOpen, setRoomOpen] = createSignal(false);

  return (
    <div class="kk" classList={{ [`kk-tab-${tab()}`]: true, "kk-room-open": roomOpen() && tab() === "chat" }}>
      <div class="kk-appbar">
        <span class="brand">OpenXgram</span>
        <span class="ver">v{__APP_VERSION__}</span>
        <span class="sp" />
        <button type="button" title="설정 (일반·런타임·위키)" classList={{ "kk-gear-on": tab() === "settings" }} onClick={() => setTab("settings")}>⚙️ 설정</button>
        <Show when={props.onLogout}>
          <button type="button" onClick={() => props.onLogout!()}>잠금</button>
        </Show>
      </div>
      <div class="kk-cols">
      <div class="kk-main">
        <div class="kk-body">
          <Show when={tab() === "chat"}>
            <div class="kk-embed"><TalkTab onJumpToSettings={() => setTab("settings")} onRoomChange={setRoomOpen} /></div>
          </Show>
          <Show when={tab() === "agents"}>
            <div class="kk-embed"><AgentsTab onGotoChat={() => setTab("chat")} /></div>
          </Show>
          <Show when={tab() === "flow"}>
            <FlowTab />
          </Show>
          <Show when={tab() === "market"}>
            <div class="kk-embed"><MarketTab /></div>
          </Show>
          {/* 설정 — 헤더 기어로 진입. 안에 일반/런타임/위키 서브탭. */}
          <Show when={tab() === "settings"}>
            <div class="kk-embed" style="display:flex; flex-direction:column; height:100%;">
              <div class="kk-subtabs" style="display:flex; gap:4px; padding:8px 12px; border-bottom:1px solid var(--border); flex:0 0 auto;">
                {SETTINGS_SUB.map((s) => (
                  <div
                    onClick={() => setSub(s.id)}
                    style={`cursor:pointer; padding:6px 14px; border-radius:8px; font-size:13px; ${sub() === s.id ? "background:var(--accent); color:var(--accent-fg);" : "color:var(--text-3);"}`}
                  >{s.ic} {s.label}</div>
                ))}
              </div>
              <div style="flex:1 1 auto; min-height:0; overflow:auto;">
                <Show when={sub() === "general"}><ConfigTab /></Show>
                <Show when={sub() === "runtime"}><RuntimeTab /></Show>
                <Show when={sub() === "wiki"}><WikiTab /></Show>
              </div>
            </div>
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
