import { createSignal, onMount, Show } from "solid-js";

// UI-MESSENGER-SPEC v1.3 C5 + §6 — Cross-card 점프 breadcrumb + 자동 복귀.
//
// 다른 카드 (자율 행동·신원·Vault 등) 가 메신저에서 진입했을 때 상단에 breadcrumb 표시.
// localStorage 키 `xgram_from_card` 가 있으면 표시 + "← 메신저로 돌아가기" 버튼.
// 자동 복귀 옵션 (체크박스).

const STORAGE_KEY = "xgram_from_card";

export interface FromMarker {
  card: string; // "messenger" | "memory" | ...
  agent_id?: string; // peer.alias / session id
  ts: number;
  auto_return?: boolean;
}

export function setFromMarker(m: FromMarker) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(m));
  } catch {}
}

export function clearFromMarker() {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {}
}

export function getFromMarker(): FromMarker | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as FromMarker;
  } catch {
    return null;
  }
}

export function Breadcrumb(props: { onReturn: () => void; cardName: string }) {
  const [marker, setMarker] = createSignal<FromMarker | null>(null);
  onMount(() => setMarker(getFromMarker()));

  return (
    <Show when={marker()}>
      {(m) => (
        <div
          style={
            "background:var(--accent-soft); color:var(--text-1); padding:8px 12px;" +
            " border-bottom:1px solid var(--border); display:flex;" +
            " justify-content:space-between; align-items:center; font-size:12px;"
          }
        >
          <div>
            🧭 진입 경로: <strong>💬 메신저</strong>
            {m().agent_id ? <> / <code>{m().agent_id}</code></> : null}
            {" → "}
            <strong>{props.cardName}</strong>
          </div>
          <div style="display:flex; gap:8px; align-items:center;">
            <label style="font-size:11px;">
              <input
                type="checkbox"
                checked={m().auto_return !== false}
                onChange={(e) => {
                  const next = { ...m(), auto_return: e.currentTarget.checked };
                  setFromMarker(next);
                  setMarker(next);
                }}
              />{" "}
              완료 시 자동 복귀
            </label>
            <button
              type="button"
              class="link-btn"
              onClick={() => {
                clearFromMarker();
                props.onReturn();
              }}
            >
              ← 메신저로 돌아가기
            </button>
          </div>
        </div>
      )}
    </Show>
  );
}
