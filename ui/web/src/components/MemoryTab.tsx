import { createSignal, Show } from "solid-js";
import { invoke } from "@/api/client";
import { useI18n } from "../i18n";
import { VaultView } from "./VaultView";

// Memory 탭 — 4탭 단순화 시 다음을 통합한다.
//   - Vault    (VaultView = Pending + Reveal) : 자격증명 승인 + 30초 reveal
//   - Wiki     (stub)                          : memory_wiki_status API 호출 → 카드 표시
//   - Mistakes (stub)                          : memory_mistakes_status
//   - Patterns (stub)                          : memory_patterns_status
//
// Wiki/Mistakes/Patterns 는 PRD-OpenXgram §4.1 wiki 격상(Karpathy 패턴) 정합.
// 본격 UI 는 후속 PR. 지금은 API 호출 + 빈 응답 표기까지.
// 검색(SearchView)은 Chat 탭으로 이동 — L0 메시지 검색은 대화 흐름의 일부.

type Section = "vault" | "wiki" | "mistakes" | "patterns";

interface StubStatus {
  count: number;
  note: string;
}

async function fetchStub(cmd: string): Promise<StubStatus> {
  // 아직 백엔드에 명령이 없을 수 있어 안전한 fallback.
  try {
    return await invoke<StubStatus>(cmd);
  } catch (e) {
    return {
      count: 0,
      note: `${cmd} 미구현 — PRD-OpenXgram §4.1 후속 (${String(e)})`,
    };
  }
}

function StubSection(props: { cmd: string; titleKey: string; descKey: string }) {
  const { t } = useI18n();
  const [data, setData] = createSignal<StubStatus | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const refresh = async () => {
    setLoaded(false);
    const v = await fetchStub(props.cmd);
    setData(v);
    setLoaded(true);
  };
  // 첫 진입 시 1회 시도.
  void refresh();
  return (
    <div class="card">
      <h3>{t(props.titleKey)}</h3>
      <p class="hint">{t(props.descKey)}</p>
      <Show when={loaded()} fallback={<p>{t("common.loading")}</p>}>
        <p>
          <span class="badge">{data()?.count ?? 0}</span>{" "}
          <span class="hint">{data()?.note}</span>
        </p>
      </Show>
      <div class="row-actions">
        <button type="button" onClick={() => void refresh()}>
          {t("common.refresh")}
        </button>
      </div>
    </div>
  );
}

export function MemoryTab() {
  const { t } = useI18n();
  const [section, setSection] = createSignal<Section>("vault");

  const sections: { id: Section; label: string }[] = [
    { id: "vault", label: t("memory.section.vault") },
    { id: "wiki", label: t("memory.section.wiki") },
    { id: "mistakes", label: t("memory.section.mistakes") },
    { id: "patterns", label: t("memory.section.patterns") },
  ];

  return (
    <div>
      <nav class="subnav" aria-label={t("memory.section.nav")}>
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
      <Show when={section() === "vault"}>
        <VaultView />
      </Show>
      <Show when={section() === "wiki"}>
        <StubSection
          cmd="memory_wiki_status"
          titleKey="memory.section.wiki"
          descKey="memory.wiki.desc"
        />
      </Show>
      <Show when={section() === "mistakes"}>
        <StubSection
          cmd="memory_mistakes_status"
          titleKey="memory.section.mistakes"
          descKey="memory.mistakes.desc"
        />
      </Show>
      <Show when={section() === "patterns"}>
        <StubSection
          cmd="memory_patterns_status"
          titleKey="memory.section.patterns"
          descKey="memory.patterns.desc"
        />
      </Show>
    </div>
  );
}
