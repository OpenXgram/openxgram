import { createSignal, Show } from "solid-js";
import "./kakao.css";
import { TalkTab } from "./TalkTab";
import { WikiTab } from "./WikiTab";
import { ConfigTab } from "./ConfigTab";
import { AgentsTab } from "./AgentsTab";
import { FlowTab } from "./FlowTab";
import { MarketTab } from "./MarketTab";
import { RuntimeTab } from "./RuntimeTab";
import { HomeDashboard, type CardId } from "./HomeDashboard";

// Phase 1 — 카카오톡 셸. 정본 디자인: _mockups/kakao-mockup.html + openxgram-conversation-model-mockup.html
// 바텀 나브 = 주요 콘텐츠(대화·에이전트·현황·워크플로우·마켓). 설정은 헤더 기어(⚙️)로 진입하며
// 그 안에 일반/런타임/위키 서브탭. (설정·런타임·위키는 바텀 나브에서 제외 — 마스터 지시)
//
// P1 — 랜딩 = 대화(chat). 현황(dashboard)은 별도 선택 탭(랜딩 아님) — 정본 목업의 📊현황 레일 항목.
type KkTab = "chat" | "agents" | "dash" | "flow" | "market" | "settings";

// 정본 목업 레일 순서: 💬채팅(랜딩) · 📊현황 · 🔀워크플로우 · 🛒마켓. 에이전트(친구 관리)도 노출.
const TABS: { id: KkTab; ic: string; label: string }[] = [
  { id: "chat", ic: "💬", label: "대화" },
  { id: "agents", ic: "🙂", label: "에이전트" },
  { id: "dash", ic: "📊", label: "현황" },
  { id: "flow", ic: "🔀", label: "워크플로우" },
  { id: "market", ic: "🌐", label: "마켓" },
];

type SettingsSub = "general" | "runtime" | "wiki";
const SETTINGS_SUB: { id: SettingsSub; ic: string; label: string }[] = [
  { id: "general", ic: "⚙️", label: "일반" },
  { id: "runtime", ic: "🧠", label: "하네스" },
  { id: "wiki", ic: "📚", label: "위키" },
];

export function KakaoShell(props: { onLogout?: () => void }) {
  // P1 — 랜딩은 대화(chat). 대시보드(현황)는 명시 선택 시에만 — 절대 랜딩 아님.
  const [tab, setTab] = createSignal<KkTab>("chat");
  const [sub, setSub] = createSignal<SettingsSub>("general");
  // 대화방(전체화면) 열림 — 카톡처럼 대화방에선 하단 네비를 숨긴다(루트에 kk-room-open 클래스).
  const [roomOpen, setRoomOpen] = createSignal(false);

  // 현황 대시보드 카드 클릭 → 셸 탭으로 매핑(레거시 카드 페이지는 셸에 없음).
  // 메신저→대화, 그 외 토대 카드→설정. 가짜 라우트 없이 셸 안에서 의미 있게 연결.
  function openDashCard(id: CardId) {
    if (id === "messenger") setTab("chat");
    else setTab("settings");
  }

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
          {/* 현황 — 별도 선택 탭(랜딩 아님). 기존 HomeDashboard 8카드 재사용. */}
          <Show when={tab() === "dash"}>
            <div class="kk-embed" style="overflow:auto; height:100%;"><HomeDashboard onOpen={openDashCard} /></div>
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
