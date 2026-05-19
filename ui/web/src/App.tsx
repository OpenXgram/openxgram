import { createResource, createSignal, Show } from "solid-js";
import { getBearer, invoke } from "@/api/client";
import { isAuthenticated, logout as apiLogout } from "@/api/auth";
import { I18nProvider, useI18n } from "./i18n";
import { Onboarding } from "./components/Onboarding";
import { ChatTab } from "./components/ChatTab";
import { MemoryTab } from "./components/MemoryTab";
import { NetworkTab } from "./components/NetworkTab";
import { SettingsTab } from "./components/SettingsTab";
import { LoginView } from "./components/LoginView";
import { RegisterView } from "./components/RegisterView";

// 4탭 단순화 (PRD-OpenXgram §4.8 v0.9 Beta).
//   - chat     : Messenger + 검색 (SearchView)
//   - memory   : Vault(=Pending+Reveal) + Wiki·Mistakes·Patterns 진입 stub
//   - network  : Peers + Notify(Telegram·Discord) + Channel 대시보드
//   - settings : Schedule + Chain + PaymentLimits + Locale
//   - onboarding 은 init 전에만 표시.
//   - 인증되지 않은 사용자 → LoginView / RegisterView (4탭 GUI 잠금).
type Tab = "onboarding" | "chat" | "memory" | "network" | "settings";
type AuthScreen = "login" | "register";

async function checkInitialized(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_initialized");
  } catch {
    return false;
  }
}

async function checkAuth(): Promise<boolean> {
  // Bearer 가 아예 없으면 즉시 false (네트워크 호출 생략).
  if (!getBearer()) return false;
  return await isAuthenticated();
}

function AppInner() {
  const { t, setLocale, locale } = useI18n();
  const [authed, { refetch: refetchAuth }] = createResource(checkAuth);
  const [authScreen, setAuthScreen] = createSignal<AuthScreen>("login");
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
    await apiLogout();
    refetchAuth();
    setAuthScreen("login");
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
          <Show when={authScreen() === "login"}>
            <LoginView
              onSuccess={() => refetchAuth()}
              onSwitchToRegister={() => setAuthScreen("register")}
            />
          </Show>
          <Show when={authScreen() === "register"}>
            <RegisterView
              onSuccess={() => refetchAuth()}
              onSwitchToLogin={() => setAuthScreen("login")}
            />
          </Show>
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
