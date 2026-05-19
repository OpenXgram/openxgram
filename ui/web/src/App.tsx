import { createResource, createSignal, Show } from "solid-js";
import { invoke } from "@/api/client";
import { isUnlocked, lock } from "@/api/auth";
import { I18nProvider, useI18n } from "./i18n";
import { Onboarding } from "./components/Onboarding";
import { ChatTab } from "./components/ChatTab";
import { MemoryTab } from "./components/MemoryTab";
import { NetworkTab } from "./components/NetworkTab";
import { SettingsTab } from "./components/SettingsTab";
import { LoginView } from "./components/LoginView";

// 4탭 단순화 (PRD-OpenXgram §4.8 v0.9 Beta).
//   - chat / memory / network / settings + onboarding(init 전만)
//   - PRD §1: 1 사람 = 1 메인 daemon + N 머신 attach. multi-user X.
//   - 잠금 = 단일 keystore 비밀번호. RegisterView 폐기.
type Tab = "onboarding" | "chat" | "memory" | "network" | "settings";

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
