import { createSignal, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// UI-MESSENGER-SPEC v1.3 §7.5 + N4 — 글로벌 검색 (FTS5).
// sqlite-vec (시멘틱) 은 별 단계 — Phase 2.

interface SearchHit {
  kind: string;
  ref_id: string;
  title: string;
  body: string;
  rank: number;
}
interface SearchResult {
  query: string;
  hits: SearchHit[];
  total: number;
}

export function GlobalSearchModal(props: { onClose: () => void }) {
  const [q, setQ] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [r, setR] = createSignal<SearchResult | null>(null);
  const [err, setErr] = createSignal<string | null>(null);

  async function run() {
    if (!q().trim()) return;
    setBusy(true);
    setErr(null);
    try {
      const res = await invoke<SearchResult>("global_search", { q: q(), limit: 30 });
      setR(res);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      onClick={props.onClose}
      style="position:fixed; inset:0; background:rgba(0,0,0,0.55); z-index:65; display:flex; justify-content:center; padding-top:80px;"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={
          "background:var(--surface-1); color:var(--text-1); border:1px solid var(--border);" +
          " border-radius:8px; padding:18px; width:640px; max-width:90vw; max-height:75vh; overflow:auto;"
        }
      >
        <h3 style="margin:0 0 10px;">🔍 글로벌 검색 — UI-MESSENGER-SPEC §7.5 + N4 (FTS5)</h3>
        <div style="display:flex; gap:6px; margin-bottom:10px;">
          <input
            type="text"
            placeholder="message · wiki · pattern · mistake · trait 통합 검색"
            value={q()}
            onInput={(e) => setQ(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && run()}
            style="flex:1; padding:8px; background:var(--surface-2); border:1px solid var(--border); color:var(--text-1); border-radius:4px;"
          />
          <button class="link-btn" type="button" onClick={run} disabled={busy()}>
            검색
          </button>
        </div>
        <Show when={err()}>
          <p style="color:#f88; font-size:12px;">⚠ {err()}</p>
        </Show>
        <Show when={r()}>
          <p style="font-size:11px; color:var(--text-3);">
            {r()!.total} 건 ({r()!.query})
          </p>
          <For each={r()!.hits}>
            {(h) => (
              <div style="padding:8px 0; border-bottom:1px solid var(--border); font-size:12px;">
                <div style="color:var(--text-3); font-size:10px;">
                  [{h.kind}] {h.ref_id} · rank {h.rank.toFixed(2)}
                </div>
                <div style="font-weight:600;">{h.title || "(제목 없음)"}</div>
                <div style="margin-top:2px;">{h.body.slice(0, 200)}</div>
              </div>
            )}
          </For>
        </Show>
        <p style="font-size:10px; color:var(--text-3); margin-top:12px;">
          FTS5 + sqlite-vec hybrid 의 sqlite-vec 통합은 Phase 2. 현재 키워드 검색만.
        </p>
        <button class="link-btn" type="button" onClick={props.onClose} style="margin-top:8px;">
          닫기
        </button>
      </div>
    </div>
  );
}
