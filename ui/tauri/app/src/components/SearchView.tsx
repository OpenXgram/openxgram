import { createSignal, createEffect, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

interface Hit {
  id: string;
  layer: "L0" | "L2" | "L4";
  body: string;
  score: number;
}

function debounce<T extends (...args: any[]) => void>(fn: T, ms: number) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return (...args: Parameters<T>) => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

export function SearchView() {
  const { t } = useI18n();
  const [q, setQ] = createSignal("");
  const [layers, setLayers] = createSignal<Record<"L0" | "L2" | "L4", boolean>>({
    L0: true,
    L2: true,
    L4: true,
  });
  const [results, setResults] = createSignal<Hit[]>([]);
  const [loading, setLoading] = createSignal(false);

  const search = debounce(async (query: string) => {
    if (!query) {
      setResults([]);
      return;
    }
    setLoading(true);
    try {
      const enabled = Object.entries(layers())
        .filter(([, v]) => v)
        .map(([k]) => k);
      const hits = await invoke<Hit[]>("memory_search", {
        query,
        layers: enabled,
      });
      setResults(hits);
    } finally {
      setLoading(false);
    }
  }, 300);

  createEffect(() => {
    search(q());
  });

  const toggleLayer = (key: "L0" | "L2" | "L4") => {
    setLayers({ ...layers(), [key]: !layers()[key] });
  };

  return (
    <div>
      <input
        type="text"
        placeholder={t("search.placeholder")}
        value={q()}
        onInput={(e) => setQ(e.currentTarget.value)}
        style="width: 100%; padding: 8px;"
      />
      <div style="display: flex; gap: 12px; margin: 8px 0;">
        <label>
          <input type="checkbox" checked={layers().L0} onChange={() => toggleLayer("L0")} />{" "}
          {t("search.layer.l0")}
        </label>
        <label>
          <input type="checkbox" checked={layers().L2} onChange={() => toggleLayer("L2")} />{" "}
          {t("search.layer.l2")}
        </label>
        <label>
          <input type="checkbox" checked={layers().L4} onChange={() => toggleLayer("L4")} />{" "}
          {t("search.layer.l4")}
        </label>
      </div>
      <Show when={!loading()} fallback={<p>searching…</p>}>
        <Show when={results().length > 0} fallback={<p>{t("search.empty")}</p>}>
          <ul style="list-style: none; padding: 0; max-height: 60vh; overflow-y: auto;">
            <For each={results()}>
              {(hit) => (
                <li style="border-bottom: 1px solid #eee; padding: 8px;">
                  <small style="color: #666;">{hit.layer} · score={hit.score.toFixed(3)}</small>
                  <p style="margin: 4px 0;">{hit.body}</p>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </div>
  );
}
