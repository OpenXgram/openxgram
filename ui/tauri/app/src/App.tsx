import { createSignal, Show } from "solid-js";
import { I18nProvider, useI18n } from "./i18n";
import { PendingList } from "./components/PendingList";
import { SearchView } from "./components/SearchView";
import { PeersView } from "./components/PeersView";
import { VaultRevealView } from "./components/VaultRevealView";
import { PaymentLimitsView } from "./components/PaymentLimitsView";

type Tab =
  | "pending"
  | "search"
  | "peers"
  | "vault-reveal"
  | "payment-limits";

function Inner() {
  const { t, setLocale, locale } = useI18n();
  const [tab, setTab] = createSignal<Tab>("pending");
  const tabs: { id: Tab; label: () => string }[] = [
    { id: "pending", label: () => t("tab.pending") },
    { id: "search", label: () => t("tab.search") },
    { id: "peers", label: () => t("tab.peers") },
    { id: "vault-reveal", label: () => t("tab.vault_reveal") },
    { id: "payment-limits", label: () => t("tab.payment_limits") },
  ];
  return (
    <div style="font-family: system-ui, sans-serif; padding: 16px;">
      <header
        style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px;"
      >
        <h1 style="margin: 0; font-size: 18px;">OpenXgram</h1>
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
      <nav style="display: flex; gap: 8px; margin-bottom: 16px;">
        {tabs.map((entry) => (
          <button
            type="button"
            onClick={() => setTab(entry.id)}
            style={{
              padding: "6px 12px",
              border: tab() === entry.id ? "2px solid #333" : "1px solid #ccc",
              background: tab() === entry.id ? "#eee" : "#fff",
              cursor: "pointer",
            }}
          >
            {entry.label()}
          </button>
        ))}
      </nav>
      <main>
        <Show when={tab() === "pending"}>
          <PendingList />
        </Show>
        <Show when={tab() === "search"}>
          <SearchView />
        </Show>
        <Show when={tab() === "peers"}>
          <PeersView />
        </Show>
        <Show when={tab() === "vault-reveal"}>
          <VaultRevealView />
        </Show>
        <Show when={tab() === "payment-limits"}>
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
