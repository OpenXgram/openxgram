import { createSignal, createResource, createEffect, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// OpenXgram 런타임(하네스) 설정 탭 — 컴포저↔어댑터 사이 제어/설정/메모리주입 레이어를
// 설정하고, 지금 무엇이 주입되는지(관찰) 확인한다. config 는 백엔드 identity_settings 에 영속.

type RuntimeConfig = {
  perm_default: string;
  model_default: string;
  thinking_default: string;
  inject_memory: boolean;
  memory_count: number;
  inject_wiki: boolean;
};

const DEFAULTS: RuntimeConfig = {
  perm_default: "bypassPermissions",
  model_default: "default",
  thinking_default: "high",
  inject_memory: true,
  memory_count: 8,
  inject_wiki: false,
};

export function RuntimeTab() {
  const [cfg, setCfg] = createSignal<RuntimeConfig>(DEFAULTS);
  const [saved, setSaved] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const [loaded] = createResource(async () => {
    try {
      const r = await invoke<{ config: Partial<RuntimeConfig> }>("runtime_config_get");
      setCfg({ ...DEFAULTS, ...(r?.config ?? {}) });
    } catch { /* keep defaults */ }
    return true;
  });

  // 관찰 패널 — 지금 주입 가능한 메모리/위키 (memory_count 반영).
  const [ctx, { refetch: refetchCtx }] = createResource(
    () => cfg().memory_count,
    async (count) => {
      try {
        return await invoke<{ memories: { kind: string; content: string }[]; wiki: { id: string; title: string }[]; memory_count: number; wiki_count: number }>(
          "runtime_context", { count: String(count) },
        );
      } catch { return { memories: [], wiki: [], memory_count: 0, wiki_count: 0 }; }
    },
  );

  function set<K extends keyof RuntimeConfig>(k: K, v: RuntimeConfig[K]) {
    setCfg({ ...cfg(), [k]: v });
    setSaved(null);
  }

  async function save() {
    setBusy(true);
    try {
      await invoke("runtime_config_set", { config: cfg() });
      setSaved("저장됨 — 새 대화부터 적용");
      await refetchCtx();
    } catch (e) {
      setSaved(`저장 실패: ${(e as Error).message}`);
    } finally { setBusy(false); }
  }

  createEffect(() => { loaded(); });

  return (
    <div class="kk-runtime" style="padding:18px 22px; max-width:760px; color:#cfd5de; overflow:auto;">
      <h2 style="margin:0 0 4px;">⚙️ 런타임 (하네스)</h2>
      <div style="color:#8a92a0; font-size:12.5px; margin-bottom:16px;">
        컴포저↔에이전트 사이 제어 레이어. 권한·모델·effort 기본값과 <b>OpenXgram 메모리·위키 주입</b>을
        설정하고, 지금 무엇이 주입되는지 아래 관찰 패널에서 확인합니다.
      </div>

      <div style="display:grid; grid-template-columns:170px 1fr; gap:10px 14px; align-items:center;">
        <label>기본 권한 모드</label>
        <select value={cfg().perm_default} onInput={(e) => set("perm_default", e.currentTarget.value)} style="padding:6px; background:#1a1f29; color:#cfd5de; border:1px solid #2b303a; border-radius:6px;">
          <option value="bypassPermissions">Bypass Permissions (전체 허용)</option>
          <option value="acceptEdits">Accept Edits</option>
          <option value="default">Default (allowlist)</option>
          <option value="plan">Plan (읽기전용)</option>
        </select>

        <label>기본 모델</label>
        <input value={cfg().model_default} onInput={(e) => set("model_default", e.currentTarget.value)} style="padding:6px; background:#1a1f29; color:#cfd5de; border:1px solid #2b303a; border-radius:6px;" />

        <label>기본 effort</label>
        <select value={cfg().thinking_default} onInput={(e) => set("thinking_default", e.currentTarget.value)} style="padding:6px; background:#1a1f29; color:#cfd5de; border:1px solid #2b303a; border-radius:6px;">
          <For each={["high", "medium", "low", "off"]}>{(o) => <option value={o}>{o}</option>}</For>
        </select>

        <label>메모리 주입</label>
        <div><input type="checkbox" checked={cfg().inject_memory} onChange={(e) => set("inject_memory", e.currentTarget.checked)} /> 대화 시작에 OpenXgram L2 메모리 주입</div>

        <label>주입 메모리 수</label>
        <input type="number" min="0" max="50" value={cfg().memory_count} onInput={(e) => set("memory_count", parseInt(e.currentTarget.value) || 0)} style="padding:6px; width:90px; background:#1a1f29; color:#cfd5de; border:1px solid #2b303a; border-radius:6px;" />

        <label>위키 주입</label>
        <div><input type="checkbox" checked={cfg().inject_wiki} onChange={(e) => set("inject_wiki", e.currentTarget.checked)} /> 위키 제목 목록 주입</div>
      </div>

      <div style="margin-top:16px; display:flex; gap:10px; align-items:center;">
        <button disabled={busy()} onClick={save} style="background:#2f6a3a; color:#fff; border:none; border-radius:7px; padding:8px 16px; cursor:pointer;">{busy() ? "저장 중…" : "설정 저장"}</button>
        <Show when={saved()}><span style="color:#7fc99a; font-size:12.5px;">{saved()}</span></Show>
      </div>

      <h3 style="margin:22px 0 6px;">🔎 관찰 — 지금 이 설정으로 주입될 것</h3>
      <Show when={!ctx.loading} fallback={<div style="color:#8a92a0;">불러오는 중…</div>}>
        <div style="background:#16181d; border:1px solid #23262d; border-radius:8px; padding:12px;">
          <div style="font-size:12.5px; color:#9aa1ad; margin-bottom:6px;">
            메모리 {cfg().inject_memory ? (ctx()?.memory_count ?? 0) : 0}개 · 위키 {cfg().inject_wiki ? (ctx()?.wiki_count ?? 0) : 0}개 주입 예정
            {!cfg().inject_memory && " (메모리 주입 꺼짐)"}
          </div>
          <Show when={cfg().inject_memory}>
            <For each={ctx()?.memories ?? []}>
              {(m) => (
                <div style="font-size:12px; padding:4px 0; border-top:1px solid #23262d;">
                  <span style="color:#fee500; margin-right:6px;">[{m.kind}]</span>
                  <span style="color:#c9d1d9;">{m.content.slice(0, 140)}</span>
                </div>
              )}
            </For>
          </Show>
          <Show when={cfg().inject_wiki && (ctx()?.wiki?.length ?? 0) > 0}>
            <div style="font-size:12px; color:#9ecbff; margin-top:8px;">위키: {(ctx()?.wiki ?? []).map((w) => w.title).join(", ")}</div>
          </Show>
        </div>
      </Show>
    </div>
  );
}
