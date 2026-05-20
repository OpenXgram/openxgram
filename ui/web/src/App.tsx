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
import { IdentityCard } from "./components/IdentityCard";
import { VaultMcpCard } from "./components/VaultMcpCard";
import { ChannelCard } from "./components/ChannelCard";
import { MemoryCard } from "./components/MemoryCard";
import { AutonomyCard } from "./components/AutonomyCard";
import { ExternalAgentCard } from "./components/ExternalAgentCard";
import { OpsCard } from "./components/OpsCard";
import { ApprovalQueueBell } from "./components/ApprovalQueueBell";
import { GlobalSearchModal } from "./components/GlobalSearchModal";

// PRD-OpenXgram v1.4 §0 + UI-CARDS-IDENTITY v1.1: 홈 대시보드 = 8 카드 (4 가치 + 4 토대).
// unlock 후 첫 화면 = HomeDashboard. 카드 클릭 시 해당 카드 전용 페이지 진입.
type Tab =
  | "onboarding"
  | "home"
  | "chat"
  | "memory"
  | "network"
  | "settings"
  | "card-identity"
  | "card-vault"
  | "card-channel"
  | "card-memory"
  | "card-autonomy"
  | "card-external"
  | "card-ops";

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

  // 카드 클릭 → 카드 전용 페이지. 메신저만 기존 ChatTab 전체 화면 사용 (실시간 + 시각화 무대).
  function openCard(id: CardId) {
    switch (id) {
      case "messenger": setTab("chat"); break;
      case "memory": setTab("card-memory"); break;
      case "channel": setTab("card-channel"); break;
      case "autonomy": setTab("card-autonomy"); break;
      case "vault": setTab("card-vault"); break;
      case "external": setTab("card-external"); break;
      case "identity": setTab("card-identity"); break;
      case "ops": setTab("card-ops"); break;
      default: setTab("home"); break;
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
        <h1 class="app-title">
          OpenXgram <span class="app-version" title={`build ${__BUILD_TIME__}`}>v{__APP_VERSION__}</span>
        </h1>
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
            <SearchButton />
            <ApprovalQueueBell />
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
        {/* tabnav — onboarding/home/카드 전용 페이지에서는 숨김 (카드 페이지는 자체 ← 홈 버튼) */}
        <Show when={tab() !== "onboarding" && tab() !== "home" && !tab().startsWith("card-")}>
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
          <Show when={tab() === "card-identity"}>
            <IdentityCard onBack={() => setTab("home")} />
          </Show>
          <Show when={tab() === "card-vault"}>
            <VaultMcpCard onBack={() => setTab("home")} />
          </Show>
          <Show when={tab() === "card-channel"}>
            <ChannelCard onBack={() => setTab("home")} />
          </Show>
          <Show when={tab() === "card-memory"}>
            <MemoryCard onBack={() => setTab("home")} />
          </Show>
          <Show when={tab() === "card-autonomy"}>
            <AutonomyCard onBack={() => setTab("home")} />
          </Show>
          <Show when={tab() === "card-external"}>
            <ExternalAgentCard onBack={() => setTab("home")} />
          </Show>
          <Show when={tab() === "card-ops"}>
            <OpsCard onBack={() => setTab("home")} />
          </Show>
        </main>
      </Show>
    </div>
  );
}

function SearchButton() {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title="글로벌 검색 (N4)"
        style="background:transparent; border:1px solid var(--border); border-radius:4px; padding:4px 10px; cursor:pointer; color:var(--text-1); font-size:13px;"
      >
        🔍
      </button>
      <Show when={open()}>
        <GlobalSearchModal onClose={() => setOpen(false)} />
      </Show>
    </>
  );
}

export function App() {
  return (
    <I18nProvider>
      <AppInner />
    </I18nProvider>
  );
}
