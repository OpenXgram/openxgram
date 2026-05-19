import { createSignal, Show } from "solid-js";
import { useI18n } from "../i18n";
import { PeersView } from "./PeersView";
import { NotifySetup } from "./NotifySetup";
import { ChannelDashboard } from "./ChannelDashboard";

// Network 탭 — peer 등록 + 알림 봇 연결 + 채널 대시보드를 한 탭에 모음.
type Section = "peers" | "notify" | "channel";

export function NetworkTab() {
  const { t } = useI18n();
  const [section, setSection] = createSignal<Section>("peers");

  const sections: { id: Section; label: string }[] = [
    { id: "peers", label: t("network.section.peers") },
    { id: "notify", label: t("network.section.notify") },
    { id: "channel", label: t("network.section.channel") },
  ];

  return (
    <div>
      <nav class="subnav" aria-label={t("network.section.nav")}>
        {sections.map((s) => (
          <button
            type="button"
            class={section() === s.id ? "active" : ""}
            onClick={() => setSection(s.id)}
          >
            {s.label}
          </button>
        ))}
      </nav>
      <Show when={section() === "peers"}>
        <PeersView />
      </Show>
      <Show when={section() === "notify"}>
        <NotifySetup />
      </Show>
      <Show when={section() === "channel"}>
        <ChannelDashboard />
      </Show>
    </div>
  );
}
