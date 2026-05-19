import { createResource, createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { I18nProvider, useI18n } from "./i18n";
import { Onboarding } from "./components/Onboarding";
import { ChatTab } from "./components/ChatTab";
import { MemoryTab } from "./components/MemoryTab";
import { NetworkTab } from "./components/NetworkTab";
import { SettingsTab } from "./components/SettingsTab";

// 4탭 단순화 (PRD-OpenXgram §4.8 v0.9 Beta).
//   - chat     : Messenger + 검색 (SearchView)
//   - memory   : Vault(=Pending+Reveal) + Wiki·Mistakes·Patterns 진입 stub
//   - network  : Peers + Notify(Telegram·Discord) + Channel 대시보드
//   - settings : Schedule + Chain + PaymentLimits + Locale
//   - onboarding 은 init 전에만 표시.
type Tab = "onboarding" | "chat" | "memory" | "network" | "settings";

async function checkInitialized(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_initialized");
  } catch {
    return false;
  }
}

function Inner() {
  const { t, setLocale, locale } = useI18n();
  const [initialized] = createResource(checkInitialized);
  const [tab, setTab] = createSignal<Tab>("onboarding");

  // 초기화된 사용자 → 첫 화면을 Chat 으로 자동 전환 (한 번만).
  let autoSwitched = false;
  const maybeAutoSwitch = () => {
    if (!autoSwitched && initialized() === true && tab() === "onboarding") {
      autoSwitched = true;
      setTab("chat");
    }
  };
  queueMicrotask(maybeAutoSwitch);

  const tabs: { id: Exclude<Tab, "onboarding">; label: () => string }[] = [
    { id: "chat", label: () => t("tab.chat") },
    { id: "memory", label: () => t("tab.memory") },
    { id: "network", label: () => t("tab.network") },
    { id: "settings", label: () => t("tab.settings") },
  ];

  return (
    <div class="app-shell">
      <header class="app-header">
        <h1 class="app-title">OpenXgram</h1>
        <div>
          <select
            value={locale()}
            onChange={(e) => setLocale(e.currentTarget.value as "ko" | "en")}
            aria-label="Locale"
          >
            <option value="ko">한국어</option>
            <option value="en">English</option>
          </select>
        </div>
      </header>
      <Show when={tab() !== "onboarding"}>
        <nav class="tabnav" aria-label="OpenXgram tabs">
          {tabs.map((entry) => (
            <button
              type="button"
              class={tab() === entry.id ? "active" : ""}
              onClick={() => setTab(entry.id)}
            >
              {entry.label()}
            </button>
          ))}
        </nav>
      </Show>
      <main>
        <Show when={tab() === "onboarding"}>
          <Onboarding onReady={() => setTab("chat")} />
        </Show>
        <Show when={tab() === "chat"}>
          <ChatTab />
        </Show>
        <Show when={tab() === "memory"}>
          <MemoryTab />
        </Show>
        <Show when={tab() === "network"}>
          <NetworkTab />
        </Show>
        <Show when={tab() === "settings"}>
          <SettingsTab />
        </Show>
      </main>
    </div>
  );
}

export function App() {
  return (
    <I18nProvider>
      <Inner />
    </I18nProvider>
  );
}
