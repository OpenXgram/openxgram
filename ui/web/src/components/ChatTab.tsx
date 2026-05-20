import { createSignal, Show } from "solid-js";
import { useI18n } from "../i18n";
import { Messenger } from "./Messenger";
import { SearchView } from "./SearchView";
import { PairingGuide } from "./PairingGuide";

// Chat 탭 — 메신저(친구·스레드)와 메시지 검색을 한 탭에서.
//   디폴트 = Messenger. Search 는 상단 토글로 켠다.
type Mode = "thread" | "search";

export function ChatTab(props: { onJumpToSettings?: () => void } = {}) {
  const { t } = useI18n();
  const [mode, setMode] = createSignal<Mode>("thread");

  return (
    <div>
      <PairingGuide onJumpToSettings={() => props.onJumpToSettings?.()} />
      <nav class="subnav" aria-label={t("chat.section.nav")}>
        <button
          type="button"
          class={mode() === "thread" ? "active" : ""}
          onClick={() => setMode("thread")}
        >
          {t("chat.section.thread")}
        </button>
        <button
          type="button"
          class={mode() === "search" ? "active" : ""}
          onClick={() => setMode("search")}
        >
          {t("chat.section.search")}
        </button>
      </nav>
      <Show when={mode() === "thread"}>
        <Messenger onJumpToSettings={() => props.onJumpToSettings?.()} />
      </Show>
      <Show when={mode() === "search"}>
        <SearchView />
      </Show>
    </div>
  );
}
