import { createResource, createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { I18nProvider, useI18n } from "./i18n";
import { Onboarding } from "./components/Onboarding";
import { SearchView } from "./components/SearchView";
import { VaultView } from "./components/VaultView";
import { PeersView } from "./components/PeersView";
import { NotifySetup } from "./components/NotifySetup";
import { ChannelDashboard } from "./components/ChannelDashboard";
import { ScheduleView } from "./components/ScheduleView";
import { ChainView } from "./components/ChainView";
import { PaymentLimitsView } from "./components/PaymentLimitsView";

type Tab =
  | "onboarding"
  | "memory"
  | "vault"
  | "peers"
  | "notify"
  | "channel"
  | "schedule"
  | "chain"
  | "settings";

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

  // 초기화된 사용자 → 첫 화면을 Memory 로 자동 전환 (한 번만).
  let autoSwitched = false;
  const maybeAutoSwitch = () => {
    if (!autoSwitched && initialized() === true && tab() === "onboarding") {
      autoSwitched = true;
      setTab("memory");
    }
  };
  queueMicrotask(maybeAutoSwitch);

  const tabs: { id: Tab; label: () => string }[] = [
    { id: "onboarding", label: () => t("tab.onboarding") },
    { id: "memory", label: () => t("tab.memory") },
    { id: "vault", label: () => t("tab.vault") },
    { id: "peers", label: () => t("tab.peers") },
    { id: "notify", label: () => t("tab.notify") },
    { id: "channel", label: () => t("tab.channel") },
    { id: "schedule", label: () => t("tab.schedule") },
    { id: "chain", label: () => t("tab.chain") },
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
          >
            <option value="ko">한국어</option>
            <option value="en">English</option>
          </select>
        </div>
      </header>
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
      <main>
        <Show when={tab() === "onboarding"}>
          <Onboarding onReady={() => setTab("memory")} />
        </Show>
        <Show when={tab() === "memory"}>
          <SearchView />
        </Show>
        <Show when={tab() === "vault"}>
          <VaultView />
        </Show>
        <Show when={tab() === "peers"}>
          <PeersView />
        </Show>
        <Show when={tab() === "notify"}>
          <NotifySetup />
        </Show>
        <Show when={tab() === "channel"}>
          <ChannelDashboard />
        </Show>
        <Show when={tab() === "schedule"}>
          <ScheduleView />
        </Show>
        <Show when={tab() === "chain"}>
          <ChainView />
        </Show>
        <Show when={tab() === "settings"}>
          <PaymentLimitsView />
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
