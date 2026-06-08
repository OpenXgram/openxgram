// UI-MESSENGER-SPEC v1.4 §20 — 워크플로 (W-1~W-10).
// YAML 빌더 + cron/메시지 트리거 + 비용 한도 + 휴먼 in the loop + 실행 이력.

import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";

const SAMPLE_YAML = `# W-1: YAML 워크플로 (시각 빌더 토글은 Phase 2)
name: example-pipeline
description: 외부 호출 → AI 분석 → 메일 발송
steps:
 - id: fetch_data
 agent: researcher
 action: web_search
 input: "오늘 비트코인 가격"
 - id: analyze
 agent: analyst
 depends_on: [fetch_data] # W-2 단계 의존성
 action: llm_call
 input: "{{steps.fetch_data.output}}"
 - id: report
 agent: scribe
 depends_on: [analyze]
 action: email
 to: "ai@kands.kr"
 body: "{{steps.analyze.output}}"
on_error: # W-7 에러 처리
 - notify: telegram
 - abort: true
cost_limit_usdc: 1.0 # W-8 비용 한도
human_approval_at: report # W-3 휴먼 in the loop
`;

// rc.279 — Paperclip 오케스트레이션 GUI (org agents + peer 추가 + invoke).
// 친근한 이름 derivation (Messenger.deriveFriendly 와 동일 규칙 — 중복 회피용 로컬 복제는
// import cycle 방지를 위해 최소 버전만 둠).
function deriveFriendly(raw: string): string {
 const s = (raw || "").trim();
 if (!s) return "(unknown)";
 const svM = s.match(/^sv(?:_aoe)?_([a-zA-Z0-9-]+)/i);
 if (svM) return `Subagent ${svM[1]}`;
 let core = s.toLowerCase();
 core = core.replace(/^\[[^\]]+\]\s*/, "");
 core = core.replace(/^(?:aoe|term)[_-]/, "");
 core = core.replace(/[_-][0-9a-f]{8,}$/i, "");
 core = core.trim() || s;
 return core.charAt(0).toUpperCase() + core.slice(1);
}

interface OrgAgent {
 alias: string;
 role?: string | null;
 orchestration_role?: string | null;
 adapter_type?: string | null;
 reports_to?: string | null;
 company_id?: string | null;
 status?: string | null;
}
interface PeerLite { alias: string; role?: string | null; machine?: string }
interface OrgIssue { id: string; title: string; status: string; assignee_agent_id?: string | null }

function adapterBadgeColor(t?: string | null): string {
 switch (t) {
 case "peer_send": return "rgba(58,130,246,0.25)";
 case "process": return "rgba(34,197,94,0.25)";
 case "http": return "rgba(168,85,247,0.25)";
 default: return "var(--surface-2)";
 }
}

// 오케스트레이션 섹션 — org 에이전트 목록 + peer 추가 + 에이전트별 invoke + 이슈 목록.
function OrchestrationSection() {
 const [agents, { refetch: refetchAgents }] = createResource<OrgAgent[]>(
 async () => { try { return await invoke<OrgAgent[]>("orchestration_agents"); } catch { return []; } });
 const [peers] = createResource<PeerLite[]>(
 async () => { try { return await invoke<PeerLite[]>("peers_list"); } catch { return []; } });
 const [issues] = createResource<OrgIssue[]>(
 async () => { try { return await invoke<OrgIssue[]>("orchestration_issues"); } catch { return []; } });

 const [peerToAdd, setPeerToAdd] = createSignal("");
 const [adding, setAdding] = createSignal(false);
 // invoke 상태 — alias 별.
 const [promptByAlias, setPromptByAlias] = createSignal<Record<string, string>>({});
 const [busyAlias, setBusyAlias] = createSignal<string | null>(null);
 const [resultByAlias, setResultByAlias] = createSignal<Record<string, { summary: string; timed_out: boolean } | { error: string }>>({});

 // 이미 org agent 인 alias 는 드롭다운에서 제외.
 const addablePeers = () => {
 const have = new Set((agents() ?? []).map((a) => a.alias));
 return (peers() ?? []).filter((p) => !have.has(p.alias));
 };

 async function addPeer() {
 const alias = peerToAdd();
 if (!alias) { alert("추가할 peer 선택"); return; }
 setAdding(true);
 try {
 await invoke("orchestration_add_from_peer", { alias });
 setPeerToAdd("");
 await refetchAgents();
 } catch (e) { alert("peer 추가 실패: " + String(e)); }
 finally { setAdding(false); }
 }

 async function runInvoke(alias: string) {
 const prompt = (promptByAlias()[alias] || "").trim();
 if (!prompt) { alert("작업 내용(prompt) 입력"); return; }
 setBusyAlias(alias);
 setResultByAlias((m) => { const n = { ...m }; delete n[alias]; return n; });
 try {
 const r = await invoke<any>("orchestration_agent_invoke", { alias, prompt });
 const res = r?.result || {};
 setResultByAlias((m) => ({ ...m, [alias]: { summary: res.summary ?? "(빈 응답)", timed_out: !!res.timed_out } }));
 } catch (e) {
 setResultByAlias((m) => ({ ...m, [alias]: { error: String(e) } }));
 } finally { setBusyAlias(null); }
 }

 return (
 <div style="margin-bottom:14px; padding-bottom:12px; border-bottom:2px solid var(--border);">
 <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:8px;">
 <strong style="font-size:13px;"> 오케스트레이션 — Org 에이전트 ({(agents() ?? []).length})</strong>
 </div>

 {/* + peer 추가 */}
 <div style="display:flex; gap:6px; margin-bottom:10px; align-items:center;">
 <select value={peerToAdd()} onChange={(e) => setPeerToAdd(e.currentTarget.value)}
 style="flex:1; padding:4px; font-size:12px;">
 <option value="">+ peer 를 org 에이전트로 추가…</option>
 <For each={addablePeers()}>
 {(p) => <option value={p.alias}>{deriveFriendly(p.alias)} ({p.alias}{p.machine ? " · " + p.machine : ""})</option>}
 </For>
 </select>
 <button class="link-btn" disabled={adding() || !peerToAdd()} onClick={addPeer}>
 {adding() ? "추가 중…" : "추가"}
 </button>
 </div>

 {/* org 에이전트 목록 */}
 <Show when={(agents() ?? []).length > 0} fallback={
 <p style="font-size:12px; color:var(--text-3); padding:8px;">
 org 에이전트 없음. 위 드롭다운에서 peer 를 추가하면 작업을 보낼 수 있는 에이전트가 됩니다.
 </p>
 }>
 <For each={agents() ?? []}>
 {(a) => (
 <div style="padding:8px; border:1px solid var(--border); border-radius:4px; margin-bottom:6px; font-size:12px;">
 <div style="display:flex; justify-content:space-between; align-items:center; gap:6px;">
 <strong>{deriveFriendly(a.alias)}</strong>
 <span style={`font-size:10px; padding:1px 6px; border-radius:3px; background:${adapterBadgeColor(a.adapter_type)}; color:var(--text-1);`}>
 {a.adapter_type || "peer_send"}
 </span>
 </div>
 <div style="color:var(--text-3); font-size:11px; margin-top:2px;">
 {a.orchestration_role || a.role || "agent"}
 {a.reports_to && <span> · ↰ {deriveFriendly(a.reports_to)}</span>}
 {a.status && <span> · {a.status}</span>}
 </div>
 {/* invoke */}
 <div style="display:flex; gap:4px; margin-top:6px;">
 <input
 value={(promptByAlias()[a.alias] || "")}
 onInput={(e) => { const v = e.currentTarget.value; setPromptByAlias((m) => ({ ...m, [a.alias]: v })); }}
 placeholder="작업 보내기 (prompt)…"
 style="flex:1; padding:4px; font-size:11px;"
 onKeyDown={(e) => { if (e.key === "Enter") runInvoke(a.alias); }}
 />
 <button class="link-btn" disabled={busyAlias() === a.alias} onClick={() => runInvoke(a.alias)}>
 {busyAlias() === a.alias ? "보내는 중…" : "보내기"}
 </button>
 </div>
 <Show when={resultByAlias()[a.alias]}>
 {(r) => {
 const v = r() as any;
 return (
 <div style={`margin-top:6px; padding:6px; font-size:11px; border-radius:4px; background:var(--surface-2); border-left:3px solid ${v.error ? "var(--danger, #f85149)" : v.timed_out ? "#d29922" : "#238636"}; white-space:pre-wrap; max-height:160px; overflow:auto;`}>
 {v.error
 ? "오류: " + v.error
 : (v.timed_out ? "[타임아웃] " : "") + v.summary}
 </div>
 );
 }}
 </Show>
 </div>
 )}
 </For>
 </Show>

 {/* 이슈 목록 */}
 <div style="margin-top:10px;">
 <strong style="font-size:12px; color:var(--text-2);"> 이슈 ({(issues() ?? []).length})</strong>
 <Show when={(issues() ?? []).length > 0} fallback={
 <p style="font-size:11px; color:var(--text-3); padding:4px 0;">이슈 없음 (issue board 는 Phase 3 확장 예정).</p>
 }>
 <For each={issues() ?? []}>
 {(i) => (
 <div style="padding:4px 6px; font-size:11px; border-bottom:1px solid var(--border); display:flex; justify-content:space-between; gap:6px;">
 <span style="flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">{i.title}</span>
 <span style="color:var(--text-3);">{i.status}{i.assignee_agent_id ? " · " + deriveFriendly(i.assignee_agent_id) : ""}</span>
 </div>
 )}
 </For>
 </Show>
 </div>
 </div>
 );
}

export function WorkflowPanel(props: { onSelectWorkflow?: (id: string) => void}) {
 const [list, { refetch}] = createResource<any[]>(async () => { try { return await invoke<any[]>("workflows_list");} catch { return [];}});
 const [showEditor, setShowEditor] = createSignal(false);
 const [editingId, setEditingId] = createSignal<string | null>(null);
 const [name, setName] = createSignal("");
 const [yamlBody, setYamlBody] = createSignal(SAMPLE_YAML);
 const [orchestrator, setOrchestrator] = createSignal("");
 const [cronExpr, setCronExpr] = createSignal("");
 const [costLimit, setCostLimit] = createSignal(1.0);

 async function save() {
 if (!name() || !yamlBody()) { alert("name + yaml 필수"); return;}
 try {
 await invoke("workflow_upsert", {
 id: editingId() || undefined, name: name(), yaml_body: yamlBody(),
 orchestrator: orchestrator() || null, cron_expr: cronExpr() || null,
 cost_limit: costLimit() || null,
});
 setShowEditor(false); setEditingId(null);
 await refetch();
} catch (e) { alert(String(e));}
}

 async function runNow(id: string) {
 try { const r = await invoke<any>("workflow_run", { id}); alert(`실행 시작: ${r.run_id}`); await refetch();} catch (e) { alert(String(e));}
}

 async function del(id: string) {
 if (!confirm(`${id} 삭제?`)) return;
 try { await invoke("workflow_delete", { id}); await refetch();} catch (e) { alert(String(e));}
}

 async function edit(id: string) {
 try {
 const w = await invoke<any>("workflow_get", { id});
 setEditingId(w.id); setName(w.name); setYamlBody(w.yaml_body);
 setOrchestrator(w.orchestrator || ""); setCronExpr(w.cron_expr || ""); setCostLimit(w.cost_limit || 1);
 setShowEditor(true);
} catch (e) { alert(String(e));}
}

 function newWorkflow() {
 setEditingId(null); setName(""); setYamlBody(SAMPLE_YAML); setOrchestrator(""); setCronExpr(""); setCostLimit(1);
 setShowEditor(true);
}

 return (
 <div style="padding:8px; flex:1; overflow:auto;">
 <Show when={!showEditor()}>
 <OrchestrationSection />
 <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:8px;">
 <strong style="font-size:13px;"> 워크플로 ({list()?.length ?? 0})</strong>
 <button class="link-btn" onClick={newWorkflow}>+ 새 워크플로</button>
 </div>
 <For each={list() ?? []}>
 {(w: any) => (
 <div style="padding:8px; border:1px solid var(--border); border-radius:4px; margin-bottom:6px; font-size:12px;">
 <div style="display:flex; justify-content:space-between; align-items:center;">
 <strong>{w.name}</strong>
 <span style="font-size:10px; color:var(--text-3);">{w.enabled ? " 활성" : "○ 비활성"}</span>
 </div>
 <div style="color:var(--text-3); font-size:11px; margin-top:2px;">
 {w.description || "(설명 없음)"}
 </div>
 <div style="font-size:10px; color:var(--text-3); margin-top:4px;">
 {w.orchestrator && <span>오케스트레이터: {w.orchestrator} · </span>}
 {w.cron_expr && <span>cron: <code>{w.cron_expr}</code> · </span>}
 {w.cost_limit && <span>한도: {w.cost_limit} USDC</span>}
 </div>
 <div style="margin-top:6px; display:flex; gap:4px;">
 <button class="link-btn" onClick={() => runNow(w.id)}>▶ 실행</button>
 <button class="link-btn" onClick={() => edit(w.id)}> 편집</button>
 <button class="link-btn" onClick={() => del(w.id)}> 삭제</button>
 <Show when={props.onSelectWorkflow}>
 <button class="link-btn" onClick={() => props.onSelectWorkflow!(w.id)}> 이력</button>
 </Show>
 </div>
 </div>
)}
 </For>
 <Show when={(list() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3); padding:12px;">
 워크플로 없음. [+ 새 워크플로] 클릭 → YAML 작성 → cron/수동 실행.<br/>
 <br/>
 UI-MESSENGER-SPEC v1.4 §20 (W-1 ~ W-10):
 <br/>YAML 빌더 · 단계 의존성 · 휴먼 in the loop · cron/메시지 트리거 · 비용 한도 · 에러 처리.
 </p>
 </Show>
 </Show>

 <Show when={showEditor()}>
 <div style="display:flex; flex-direction:column; gap:6px;">
 <strong style="font-size:13px;">{editingId() ? ` 편집: ${name()}` : "+ 새 워크플로"}</strong>
 <input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="이름 (예: daily-report)" style="padding:4px;" />
 <input value={orchestrator()} onInput={(e) => setOrchestrator(e.currentTarget.value)} placeholder="W-10 오케스트레이터 에이전트 (옵션)" style="padding:4px;" />
 <input value={cronExpr()} onInput={(e) => setCronExpr(e.currentTarget.value)} placeholder="W-4/W-9 cron 표현식 (옵션, 예: 0 9 * * 1-5)" style="padding:4px;" />
 <input type="number" step="0.1" value={costLimit()} onInput={(e) => setCostLimit(parseFloat(e.currentTarget.value) || 1)} placeholder="W-8 비용 한도 USDC" style="padding:4px;" />
 <textarea value={yamlBody()} onInput={(e) => setYamlBody(e.currentTarget.value)}
 rows={20} style="padding:6px; font-family:monospace; font-size:11px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;"
 />
 <div style="display:flex; gap:6px;">
 <button class="link-btn" onClick={save}> 저장</button>
 <button class="link-btn" onClick={() => setShowEditor(false)}> 취소</button>
 </div>
 <p style="font-size:10px; color:var(--text-3);">
 W-1 YAML + 시각 빌더 토글 = Phase 2. 현재 YAML 텍스트만.
 </p>
 </div>
 </Show>
 </div>
);
}
