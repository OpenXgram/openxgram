import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";

// rc.122 — 에이전트 메신저 등록 카드.
// 외부 채널 바인딩(Discord/Telegram)과 별개. LLM↔LLM 협업·발견·소개의 필수 등록 path.
// 등록 시 다른 peer 의 list_peers 결과에 자동 노출 + group fan-out 대상.

interface AgentDto {
 alias: string;
 role: string | null;
 description: string | null;
 capabilities: string | null;
 tool_list: string | null;
 project_path: string | null;
 group_name: string | null;
 messenger_enabled: boolean;
 updated_at: string;
}

export function AgentMessengerCard(props: { onBack: () => void}) {
 const [agents, { refetch}] = createResource<AgentDto[]>(async () => {
 try { return await invoke<AgentDto[]>("agents_list");} catch { return [];}
});
 const [alias, setAlias] = createSignal("");
 const [role, setRole] = createSignal("");
 const [description, setDescription] = createSignal("");
 const [groupName, setGroupName] = createSignal("");
 const [busy, setBusy] = createSignal(false);
 const [msg, setMsg] = createSignal<string | null>(null);

 async function add(enabled: boolean) {
 if (!alias().trim()) { setMsg("alias 필요"); return;}
 setBusy(true); setMsg(null);
 try {
 await invoke("agents_register", {
 alias: alias().trim(),
 role: role().trim() || null,
 description: description().trim() || null,
 group_name: groupName().trim() || null,
 messenger_enabled: enabled,
});
 setMsg(`✓ ${alias()} 등록/갱신 (messenger=${enabled})`);
 setAlias(""); setRole(""); setDescription(""); setGroupName("");
 await refetch();
} catch (e) { setMsg(`✗ ${e}`);} finally { setBusy(false);}
}

 async function toggleMessenger(a: AgentDto) {
 setBusy(true);
 try {
 await invoke("agents_register", {
 alias: a.alias,
 role: a.role, description: a.description, group_name: a.group_name,
 messenger_enabled: !a.messenger_enabled,
});
 await refetch();
} finally { setBusy(false);}
}

 async function del(aliasName: string) {
 if (!confirm(`'${aliasName}' 등록 삭제?`)) return;
 setBusy(true);
 try { await invoke("agents_delete", { alias: aliasName}); await refetch();} finally { setBusy(false);}
}

 return (
 <section class="card-section">
 <button type="button" class="link-btn" onClick={props.onBack} style="margin-bottom:8px;">← 홈</button>
 <h3>에이전트 메신저 등록</h3>
 <p style="font-size:12px; color:var(--text-3); margin-bottom:10px;">
 LLM ↔ LLM 협업·발견·소개. 외부 채널 바인딩(Discord/Telegram)과 별개.
 <strong>messenger_enabled = ON</strong> 인 에이전트는 다른 peer 의 <code>list_peers</code> 응답에 자동 노출 + group_name 으로 <code>peer_send</code> fan-out 대상.
 </p>

 {/* 신규 등록 폼 */}
 <div style="padding:10px; background:var(--surface-2); border:1px solid var(--border); border-radius:6px; margin-bottom:12px;">
 <strong style="font-size:13px; display:block; margin-bottom:6px;">+ 새 에이전트 등록</strong>
 <div style="display:flex; flex-direction:column; gap:6px;">
 <input value={alias()} onInput={(e) => setAlias(e.currentTarget.value)}
 placeholder="alias (필수, 예: pip / eno / qua)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <input value={role()} onInput={(e) => setRole(e.currentTarget.value)}
 placeholder="role (예: PRD 작성, Rust 구현, 테스트)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <textarea value={description()} onInput={(e) => setDescription(e.currentTarget.value)}
 placeholder="description (1~3 문장 — 다른 에이전트에게 소개 메시지)" rows={2}
 style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-family:inherit;" />
 <input value={groupName()} onInput={(e) => setGroupName(e.currentTarget.value)}
 placeholder="group (선택, 예: prd-team / dev-team — peer_send fan-out 단위)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <div style="display:flex; gap:6px;">
 <button class="link-btn" disabled={busy()} onClick={() => add(true)}
 style="background:#238636; color:white; padding:6px 14px; border:none; border-radius:4px;">
 ▶ 등록 + 메신저 활성 (다른 peer 에 노출)
 </button>
 <button class="link-btn" disabled={busy()} onClick={() => add(false)}
 style="background:var(--surface); color:var(--text-1); padding:6px 14px; border:1px solid var(--border); border-radius:4px;">
 등록만 (비활성)
 </button>
 </div>
 <Show when={msg()}><div style={`padding:6px 10px; font-size:11px; border-radius:4px; background:${msg()!.startsWith("✓") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>{msg()}</div></Show>
 </div>
 </div>

 {/* 등록된 에이전트 list */}
 <strong style="font-size:13px; display:block; margin-bottom:6px;">📋 등록된 에이전트 ({(agents() ?? []).length})</strong>
 <Show when={(agents() ?? []).length === 0}>
 <div style="font-size:12px; color:var(--text-3); padding:8px;">아직 등록 없음.</div>
 </Show>
 <For each={agents() ?? []}>
 {(a) => (
 <div style={`padding:10px; margin-bottom:6px; border:1px solid var(--border); border-radius:6px; background:${a.messenger_enabled ? "rgba(35,134,54,0.08)" : "var(--surface-2)"};`}>
 <div style="display:flex; justify-content:space-between; align-items:center;">
 <div>
 <strong style="font-size:13px;">{a.alias}</strong>
 {a.role && <span style="color:var(--text-3); margin-left:8px; font-size:11px;">[{a.role}]</span>}
 {a.group_name && <span style="background:#3a4a6a; color:white; padding:2px 6px; border-radius:3px; margin-left:8px; font-size:10px;">{a.group_name}</span>}
 </div>
 <div style="display:flex; gap:6px;">
 <button class="link-btn" disabled={busy()} onClick={() => toggleMessenger(a)}
 title={a.messenger_enabled ? "메신저 비활성" : "메신저 활성"}
 style={`padding:4px 10px; border:none; border-radius:4px; background:${a.messenger_enabled ? "#238636" : "var(--surface)"}; color:${a.messenger_enabled ? "white" : "var(--text-1)"};`}>
 {a.messenger_enabled ? "● 활성" : "○ 비활성"}
 </button>
 <button class="link-btn" disabled={busy()} onClick={() => del(a.alias)}
 style="padding:4px 8px; background:var(--surface); border:1px solid #f85149; color:#f85149; border-radius:4px;">삭제</button>
 </div>
 </div>
 {a.description && <div style="font-size:11px; color:var(--text-2); margin-top:6px;">{a.description}</div>}
 {a.capabilities && <div style="font-size:10px; color:var(--text-3); margin-top:4px;">tools: <code>{a.capabilities}</code></div>}
 </div>
 )}
 </For>
 </section>
);
}
