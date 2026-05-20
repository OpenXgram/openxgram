import { createEffect, createResource, createSignal, Show } from "solid-js";
import { invoke } from "@/api/client";
import { isUnlocked, lock } from "@/api/auth";
import { I18nProvider, useI18n } from "./i18n";
import { Onboarding } from "./components/Onboarding";
import { ChatTab } from "./components/ChatTab";
import { MemoryTab } from "./components/MemoryTab";
import { NetworkTab } from "./components/NetworkTab";
import { SettingsTab } from "./components/SettingsTab";
import { LoginView } from "./components/LoginView";
import { HomeDashboard, type CardId } from "./components/HomeDashboard";

// PRD-OpenXgram v1.4 §0 + UI-CARDS-IDENTITY v1.1: 홈 대시보드 = 8 카드 (4 가치 + 4 토대).
// unlock 후 첫 화면 = HomeDashboard. 카드 클릭 시 해당 탭/뷰 진입.
// 기존 4탭 (chat·memory·network·settings) 은 카드와 매핑 (placeholder 카드는 settings 로 fallback).
type Tab = "onboarding" | "home" | "chat" | "memory" | "network" | "settings";

async function checkInitialized(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_initialized");
  } catch {
    return false;
  }
}

async function checkUnlocked(): Promise<boolean> {
  return await isUnlocked();
}

function AppInner() {
  const { t, setLocale, locale } = useI18n();
  const [authed, { refetch: refetchAuth }] = createResource(checkUnlocked);
  const [initialized] = createResource(
    () => authed() === true,
    async (ok) => (ok ? await checkInitialized() : false),
  );
  // 기본 = home (8 카드 대시보드). Onboarding 은 daemon 이 명시적으로 false 일 때만.
  const [tab, setTab] = createSignal<Tab>("home");

  // initialized 가 false 로 확정되면 onboarding 강제. true 면 home 으로 복귀.
  createEffect(() => {
    const init = initialized();
    if (init === false && tab() !== "onboarding") setTab("onboarding");
    if (init === true && tab() === "onboarding") setTab("home");
  });

  // 카드 클릭 → 탭 매핑. 미구현 placeholder 는 settings 로.
  function openCard(id: CardId) {
    switch (id) {
      case "messenger": setTab("chat"); break;
      case "memory": setTab("memory"); break;
      case "channel": setTab("settings"); break;       // Settings → 알림 채널
      case "autonomy": setTab("settings"); break;      // Settings → 예약 (cron)
      case "vault": setTab("settings"); break;         // Settings → Vault·MCP
      case "external":                                 // placeholder
      case "identity":                                 // placeholder
      case "ops":                                      // placeholder
      default: setTab("settings"); break;
    }
  }

  const tabs: { id: Exclude<Tab, "onboarding" | "home">; label: () => string }[] = [
    { id: "chat", label: () => t("tab.chat") },
    { id: "memory", label: () => t("tab.memory") },
    { id: "network", label: () => t("tab.network") },
    { id: "settings", label: () => t("tab.settings") },
  ];

  const onLogout = async () => {
    lock();
    refetchAuth();
  };

  return (
    <div class="app-shell">
      <header class="app-header">
        <h1 class="app-title">OpenXgram</h1>
        <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
          <select
            value={locale()}
            onChange={(e) => setLocale(e.currentTarget.value as "ko" | "en")}
            aria-label="Locale"
          >
            <option value="ko">한국어</option>
            <option value="en">English</option>
          </select>
          <Show when={authed() === true}>
            <button type="button" onClick={onLogout}>
              {t("auth.logout")}
            </button>
          </Show>
        </div>
      </header>

      {/* 인증 화면 — Bearer 없음/만료 */}
      <Show when={authed.loading}>
        <main>
          <p class="hint">{t("common.loading")}</p>
        </main>
      </Show>
      <Show when={!authed.loading && authed() !== true}>
        <main>
          <LoginView onUnlock={() => refetchAuth()} />
        </main>
      </Show>

      {/* 메인 GUI — 인증된 사용자만 */}
      <Show when={authed() === true}>
        {/* tabnav — onboarding/home 일 때는 숨김 */}
        <Show when={tab() !== "onboarding" && tab() !== "home"}>
          <nav class="tabnav" aria-label="OpenXgram tabs">
            <button
              type="button"
              onClick={() => setTab("home")}
              style="margin-right:8px;"
              title="홈 대시보드"
            >
              🏠 홈
            </button>
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
            <Onboarding onReady={() => setTab("home")} />
          </Show>
          <Show when={tab() === "home"}>
            <HomeDashboard onOpen={openCard} />
          </Show>
          <Show when={tab() === "chat"}>
            <ChatTab onJumpToSettings={() => setTab("settings")} />
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
      </Show>
    </div>
  );
}

export function App() {
  return (
    <I18nProvider>
      <AppInner />
    </I18nProvider>
  );
}
