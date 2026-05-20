import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// UI-MESSENGER-SPEC v1.3 V11 — RoutingRule 모달 (메신저 헤더 [🔀]).
// 에이전트 ↔ 에이전트 internal scope. 인간 ↔ 에이전트는 📱 채널 카드.

interface RoutingRule {
  id: string;
  scope: string;
  from_pattern: string;
  to_pattern: string;
  action: string;
  created_at: string;
  active: boolean;
}

async function fetchRules(): Promise<RoutingRule[]> {
  try {
    return await invoke<RoutingRule[]>("routing_rules_list");
  } catch {
    return [];
  }
}

export function RoutingRulesModal(props: { onClose: () => void }) {
  const [rules, { refetch }] = createResource(fetchRules);
  const [from, setFrom] = createSignal("");
  const [to, setTo] = createSignal("");
  const [action, setAction] = createSignal("forward");
  const [busy, setBusy] = createSignal(false);

  async function add() {
    if (!from() || !to()) return;
    setBusy(true);
    try {
      await invoke("routing_rule_add", { from_pattern: from(), to_pattern: to(), action: action() });
      setFrom("");
      setTo("");
      await refetch();
    } finally {
      setBusy(false);
    }
  }
  async function del(id: string) {
    setBusy(true);
    try {
      await invoke("routing_rule_delete", { id });
      await refetch();
    } finally {
      setBusy(false);
    }
  }
  return (
    <div
      onClick={props.onClose}
      style="position:fixed; inset:0; background:rgba(0,0,0,0.5); z-index:60; display:flex; align-items:center; justify-content:center;"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={
          "background:var(--surface-1); color:var(--text-1); border:1px solid var(--border);" +
          " border-radius:8px; padding:18px; min-width:540px; max-width:90vw; max-height:80vh; overflow:auto;"
        }
      >
        <h3 style="margin:0 0 12px;">🔀 RoutingRule — 사양 V11 (Internal scope: agent↔agent)</h3>
        <p style="font-size:11px; color:var(--text-3);">
          인간 ↔ 에이전트 라우팅 = 📱 채널 카드 (다른 마스터). 본 모달은 에이전트끼리만.
        </p>
        <div style="display:flex; gap:6px; margin:10px 0;">
          <input
            type="text"
            placeholder="from_pattern (예: claude-*)"
            value={from()}
            onInput={(e) => setFrom(e.currentTarget.value)}
            style="flex:1; padding:6px; background:var(--surface-2); border:1px solid var(--border); color:var(--text-1); border-radius:4px;"
          />
          <input
            type="text"
            placeholder="to_pattern (예: reviewer)"
            value={to()}
            onInput={(e) => setTo(e.currentTarget.value)}
            style="flex:1; padding:6px; background:var(--surface-2); border:1px solid var(--border); color:var(--text-1); border-radius:4px;"
          />
          <select
            value={action()}
            onChange={(e) => setAction(e.currentTarget.value)}
            style="padding:6px; background:var(--surface-2); border:1px solid var(--border); color:var(--text-1); border-radius:4px;"
          >
            <option value="forward">forward</option>
            <option value="summarize_and_send">summarize_and_send</option>
            <option value="block">block</option>
          </select>
          <button class="link-btn" type="button" onClick={add} disabled={busy()}>+ 추가</button>
        </div>
        <h4 style="margin:10px 0 6px;">활성 규칙 ({rules()?.length ?? 0})</h4>
        <Show when={(rules() ?? []).length === 0}>
          <p style="color:var(--text-3); font-size:12px;">규칙 없음.</p>
        </Show>
        <For each={rules() ?? []}>
          {(r) => (
            <div style="display:flex; gap:8px; padding:6px 0; border-bottom:1px solid var(--border); font-size:12px; align-items:center;">
              <code>{r.from_pattern}</code>
              <span>→</span>
              <code>{r.to_pattern}</code>
              <span style="color:var(--text-3);">[{r.action}]</span>
              <span style="margin-left:auto;">
                <button class="link-btn" type="button" onClick={() => del(r.id)} disabled={busy()}>삭제</button>
              </span>
            </div>
          )}
        </For>
        <button class="link-btn" type="button" onClick={props.onClose} style="margin-top:12px;">
          닫기
        </button>
      </div>
    </div>
  );
}
