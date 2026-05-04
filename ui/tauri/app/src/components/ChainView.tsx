import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

interface Chain {
  id: string;
  name: string;
  description: string | null;
  created_at_kst: number;
  enabled: boolean;
  step_count: number;
}

interface StepRun {
  step_order: number;
  executed: boolean;
  response: string;
  error: string | null;
  skipped_reason: string | null;
}

interface RunResult {
  chain_name: string;
  failed: boolean;
  steps: StepRun[];
}

async function fetchList(): Promise<Chain[]> {
  return await invoke<Chain[]>("chain_list");
}

export function ChainView() {
  const { t } = useI18n();
  const [list, { refetch }] = createResource(fetchList);
  const [yaml, setYaml] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [runResult, setRunResult] = createSignal<RunResult | null>(null);

  const create = async () => {
    setError(null);
    setBusy(true);
    try {
      await invoke<string>("chain_create_yaml", { yaml: yaml() });
      setYaml("");
      refetch();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (name: string) => {
    if (!confirm(`Delete chain "${name}"?`)) return;
    try {
      await invoke("chain_delete", { name });
      refetch();
    } catch (e) {
      alert(String(e));
    }
  };

  const run = async (name: string) => {
    setRunResult(null);
    setBusy(true);
    try {
      const res = await invoke<RunResult>("chain_run", { name });
      setRunResult(res);
    } catch (e) {
      alert(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <div class="card">
        <p class="section-title">{t("chain.new")}</p>
        <div class="form-row">
          <label>{t("chain.yaml.label")}</label>
          <textarea
            placeholder={t("chain.yaml.placeholder")}
            value={yaml()}
            onInput={(e) => setYaml(e.currentTarget.value)}
          />
        </div>
        <div class="row-actions">
          <button
            class="primary"
            type="button"
            onClick={create}
            disabled={busy() || !yaml().trim()}
          >
            {t("chain.create")}
          </button>
        </div>
        <Show when={error()}>
          <p class="error-text">{error()}</p>
        </Show>
      </div>

      <Show when={!list.loading} fallback={<p>{t("common.loading")}</p>}>
        <Show
          when={(list() ?? []).length > 0}
          fallback={<p class="hint">{t("chain.list_empty")}</p>}
        >
          <ul class="plain-list">
            <For each={list()}>
              {(c) => (
                <li>
                  <div>
                    <strong>{c.name}</strong>{" "}
                    <span class="badge">
                      {c.step_count} {t("chain.steps")}
                    </span>
                  </div>
                  <Show when={c.description}>
                    <div class="hint">{c.description}</div>
                  </Show>
                  <div class="row-actions" style="margin-top: 6px;">
                    <button type="button" onClick={() => run(c.name)} disabled={busy()}>
                      {t("chain.run")}
                    </button>
                    <button class="danger" type="button" onClick={() => remove(c.name)}>
                      {t("chain.delete")}
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>

      <Show when={runResult()}>
        <div class="card card-soft">
          <p class="section-title">
            {t("chain.run_result")}: <strong>{runResult()!.chain_name}</strong>{" "}
            <Show when={runResult()!.failed}>
              <span class="badge err">failed</span>
            </Show>
          </p>
          <ul class="plain-list">
            <For each={runResult()!.steps}>
              {(s) => (
                <li>
                  <div>
                    <strong>step {s.step_order}</strong>{" "}
                    <span class={s.executed ? "badge ok" : "badge"}>
                      {s.executed
                        ? t("chain.step_executed")
                        : t("chain.step_skipped")}
                    </span>
                  </div>
                  <Show when={s.skipped_reason}>
                    <div class="hint">{s.skipped_reason}</div>
                  </Show>
                  <Show when={s.error}>
                    <p class="error-text">{s.error}</p>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
    </div>
  );
}
