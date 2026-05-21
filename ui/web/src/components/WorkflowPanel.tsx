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
