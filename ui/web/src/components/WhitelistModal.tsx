import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// UI-MESSENGER-SPEC v1.3 §3.6 — 화이트리스트 패턴 (M-5 + N1 + N3 + V4).
// command > tmux > cwd 우선순위. auto_register + auto_approve_pending 토글.

interface Pattern {
  priority: number;
  pattern_type: string;
  pattern: string;
  default_role: string;
  auto_register: boolean;
  auto_approve_pending: boolean;
}

export function WhitelistModal(props: { onClose: () => void }) {
  const [list, { refetch }] = createResource(async () => {
    try { return await invoke<Pattern[]>("whitelist_patterns_list"); } catch { return []; }
  });
  const [policy] = createResource(async () => {
    try { return await invoke<any>("whitelist"); } catch { return null; }
  });
  const [priority, setPriority] = createSignal(1);
  const [type, setType] = createSignal("command");
  const [pat, setPat] = createSignal("");
  const [role, setRole] = createSignal("researcher");
  const [reg, setReg] = createSignal(true);
  const [appr, setAppr] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  async function add() {
    if (!pat()) return;
    setBusy(true);
    try {
      await invoke("whitelist_pattern_add", {
        priority: priority(), pattern_type: type(), pattern: pat(),
        default_role: role(), auto_register: reg(), auto_approve_pending: appr(),
      });
      setPat("");
      await refetch();
    } finally { setBusy(false); }
  }
  return (
    <div onClick={props.onClose}
      style="position:fixed; inset:0; background:rgba(0,0,0,0.5); z-index:60; display:flex; align-items:center; justify-content:center;">
      <div onClick={(e) => e.stopPropagation()}
        style="background:var(--surface-1); color:var(--text-1); border:1px solid var(--border); border-radius:8px; padding:18px; min-width:560px; max-width:90vw; max-height:80vh; overflow:auto;">
        <h3 style="margin:0 0 12px;">🛡️ 화이트리스트 패턴 — M-5 + N1 + N3 + V4</h3>
        <p style="font-size:11px; color:var(--text-3);">우선순위: command &gt; tmux &gt; cwd (N1). auto_register 매칭 시 자동 등록. auto_approve_pending 매칭 시 만료 큐도 자동 승인 (단 결제·위험은 절대 자동 X — V4).</p>
        <Show when={policy()}>
          <div style="background:var(--surface-2); padding:8px; border-radius:4px; margin-bottom:8px; font-size:12px;">
            <strong>정책 (V4)</strong>: 우선순위 = {(policy()?.priority_order ?? []).join(" &gt; ")}.
            절대 자동 승인 X = {(policy()?.never_auto_approve ?? []).join(", ")}.
          </div>
        </Show>
        <div style="display:flex; gap:6px; margin:10px 0; flex-wrap:wrap;">
          <input type="number" value={priority()} onInput={(e) => setPriority(parseInt(e.currentTarget.value) || 1)}
            style="width:60px; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" placeholder="prio" />
          <select value={type()} onChange={(e) => setType(e.currentTarget.value)}
            style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
            <option value="command">command</option>
            <option value="tmux">tmux</option>
            <option value="cwd">cwd</option>
          </select>
          <input value={pat()} onInput={(e) => setPat(e.currentTarget.value)}
            placeholder='pattern (예: "claude *" or "xgram-*")'
            style="flex:1; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
          <input value={role()} onInput={(e) => setRole(e.currentTarget.value)}
            placeholder="role" style="width:120px; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
        </div>
        <div style="display:flex; gap:12px; font-size:12px; margin-bottom:8px;">
          <label><input type="checkbox" checked={reg()} onChange={(e) => setReg(e.currentTarget.checked)} /> auto_register</label>
          <label><input type="checkbox" checked={appr()} onChange={(e) => setAppr(e.currentTarget.checked)} /> auto_approve_pending (V4)</label>
          <button class="link-btn" onClick={add} disabled={busy()}>+ 추가</button>
        </div>
        <h4 style="margin:10px 0 6px;">활성 패턴 ({list()?.length ?? 0})</h4>
        <For each={list() ?? []}>{(p) => (
          <div style="display:flex; gap:8px; padding:6px 0; border-bottom:1px solid var(--border); font-size:12px;">
            <span style="min-width:40px;">P{p.priority}</span>
            <span style="min-width:80px; color:var(--text-3);">{p.pattern_type}</span>
            <code style="flex:1;">{p.pattern}</code>
            <span>→ {p.default_role}</span>
            <span style={p.auto_register ? "color:#5fa;" : "color:var(--text-3);"}>{p.auto_register ? "auto" : "confirm"}</span>
            <span style={p.auto_approve_pending ? "color:#5fa;" : "color:var(--text-3);"}>{p.auto_approve_pending ? "auto-approve" : ""}</span>
          </div>
        )}</For>
        <button class="link-btn" onClick={props.onClose} style="margin-top:12px;">닫기</button>
      </div>
    </div>
  );
}
