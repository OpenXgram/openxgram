import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";

// rc.126 — 워크플로우 오케스트레이션 카드.
// UI-MESSENGER-SPEC v1.4 §20 W-1~W-10. backend handler 기존 (gui_workflow_*).
// 각 워크플로우 = yaml_body 안에 steps (sequential / parallel / DAG) 정의.

interface WorkflowDto {
 id: string;
 name: string;
 description: string | null;
 yaml_body: string;
 orchestrator: string | null;
 cron_expr: string | null;
 message_trigger: string | null;
 cost_limit: number | null;
 enabled: boolean;
 created_at: string;
 updated_at: string;
}

interface WorkflowRunDto {
 id: string;
 workflow_id: string;
 started_at: string;
 finished_at: string | null;
 status: string;
 current_step: string | null;
 error: string | null;
 total_cost: number;
 trigger_source: string | null;
}

const SAMPLE_YAML = `# 워크플로우 YAML 예시 (sequential)
steps:
 - name: prd
   agent: pip
   prompt: "이번 작업의 PRD 작성"
 - name: implement
   agent: eno
   prompt: "위 PRD 따라 Rust 구현"
   depends_on: [prd]
 - name: test
   agent: qua
   prompt: "구현 검증 + 테스트"
   depends_on: [implement]
`;

export function WorkflowsCard(props: { onBack: () => void}) {
 const [workflows, { refetch}] = createResource<WorkflowDto[]>(async () => {
 try { return await invoke<WorkflowDto[]>("workflows_list");} catch { return [];}
});
 const [selectedId, setSelectedId] = createSignal<string | null>(null);
 const [showForm, setShowForm] = createSignal(false);
 const [name, setName] = createSignal("");
 const [description, setDescription] = createSignal("");
 const [yamlBody, setYamlBody] = createSignal(SAMPLE_YAML);
 const [orchestrator, setOrchestrator] = createSignal("");
 const [cronExpr, setCronExpr] = createSignal("");
 const [costLimit, setCostLimit] = createSignal("");
 const [msg, setMsg] = createSignal<string | null>(null);
 const [busy, setBusy] = createSignal(false);

 const [runs] = createResource(selectedId, async (id) => {
 if (!id) return [];
 try { return await invoke<WorkflowRunDto[]>("workflow_runs", { id});} catch { return [];}
});

 function startNew() {
 setName(""); setDescription(""); setYamlBody(SAMPLE_YAML);
 setOrchestrator(""); setCronExpr(""); setCostLimit(""); setMsg(null);
 setShowForm(true);
}

 function loadForEdit(w: WorkflowDto) {
 setName(w.name); setDescription(w.description || ""); setYamlBody(w.yaml_body);
 setOrchestrator(w.orchestrator || ""); setCronExpr(w.cron_expr || "");
 setCostLimit(w.cost_limit?.toString() || ""); setMsg(null);
 setShowForm(true);
}

 async function save() {
 if (!name().trim()) { setMsg("name 필요"); return;}
 setBusy(true); setMsg(null);
 try {
 const r = await invoke<any>("workflow_upsert", {
 name: name().trim(),
 description: description().trim() || null,
 yaml_body: yamlBody(),
 orchestrator: orchestrator().trim() || null,
 cron_expr: cronExpr().trim() || null,
 cost_limit: costLimit() ? Number(costLimit()) : null,
 enabled: true,
});
 setMsg(`✓ 저장: ${r?.id || name()}`);
 setShowForm(false);
 await refetch();
} catch (e) { setMsg(`✗ ${e}`);} finally { setBusy(false);}
}

 async function run(id: string) {
 setBusy(true);
 try {
 const r = await invoke<any>("workflow_run", { id});
 setMsg(`▶ 실행: run_id=${r?.run_id || "?"}`);
 setSelectedId(id);
} catch (e) { setMsg(`✗ ${e}`);} finally { setBusy(false);}
}

 async function del(id: string) {
 if (!confirm("이 워크플로우 삭제?")) return;
 setBusy(true);
 try { await invoke("workflow_delete", { id}); await refetch();} finally { setBusy(false);}
}

 async function approveRun(runId: string) {
 setBusy(true);
 try { await invoke("workflow_run_approve", { run_id: runId}); setMsg(`✓ approve: ${runId}`);} finally { setBusy(false);}
}

 return (
 <section class="card-section">
 <button type="button" class="link-btn" onClick={props.onBack} style="margin-bottom:8px;">← 홈</button>
 <h3> 오케스트레이션 (워크플로우)</h3>
 <p style="font-size:12px; color:var(--text-3); margin-bottom:10px;">
 여러 에이전트를 순서대로 / 병렬 / DAG 로 구성. yaml 안 steps 정의.
 cron / 메시지 trigger / 수동 실행. 진행 monitoring.
 </p>

 <Show when={!showForm()} fallback={
 <div style="padding:10px; background:var(--surface-2); border:1px solid var(--border); border-radius:6px; margin-bottom:12px;">
 <strong style="font-size:13px; display:block; margin-bottom:6px;">워크플로우 편집</strong>
 <div style="display:flex; flex-direction:column; gap:6px;">
 <input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="name (필수)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <input value={description()} onInput={(e) => setDescription(e.currentTarget.value)} placeholder="description (선택)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <label style="font-size:11px; color:var(--text-3);">YAML body (steps / parallel / depends_on)</label>
 <textarea value={yamlBody()} onInput={(e) => setYamlBody(e.currentTarget.value)} rows={10}
 style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-family:monospace; font-size:11px;" />
 <input value={orchestrator()} onInput={(e) => setOrchestrator(e.currentTarget.value)} placeholder="orchestrator alias (선택)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <input value={cronExpr()} onInput={(e) => setCronExpr(e.currentTarget.value)} placeholder="cron expr (선택, 예: 0 9 * * *)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <input value={costLimit()} onInput={(e) => setCostLimit(e.currentTarget.value)} placeholder="cost limit USDC (선택)" style="padding:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <div style="display:flex; gap:6px;">
 <button class="link-btn" disabled={busy()} onClick={save} style="background:#238636; color:white; padding:6px 14px; border:none; border-radius:4px;">저장</button>
 <button class="link-btn" disabled={busy()} onClick={() => setShowForm(false)} style="padding:6px 14px;">취소</button>
 </div>
 <Show when={msg()}><div style={`padding:6px 10px; font-size:11px; border-radius:4px; background:${msg()!.startsWith("✓") || msg()!.startsWith("▶") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>{msg()}</div></Show>
 </div>
 </div>
 }>
 <button class="link-btn" onClick={startNew} style="background:#238636; color:white; padding:6px 14px; border:none; border-radius:4px; margin-bottom:12px;">+ 새 워크플로우</button>
 <Show when={msg()}><div style={`padding:6px 10px; font-size:11px; border-radius:4px; margin-bottom:8px; background:${msg()!.startsWith("✓") || msg()!.startsWith("▶") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>{msg()}</div></Show>
 </Show>

 <strong style="font-size:13px; display:block; margin-bottom:6px;">목록 ({(workflows() ?? []).length})</strong>
 <Show when={(workflows() ?? []).length === 0}>
 <div style="font-size:12px; color:var(--text-3); padding:8px;">워크플로우 없음. 위 "+ 새 워크플로우" 클릭.</div>
 </Show>
 <For each={workflows() ?? []}>
 {(w) => (
 <div style={`padding:10px; margin-bottom:6px; border:1px solid var(--border); border-radius:6px; background:${selectedId() === w.id ? "rgba(58,74,106,0.2)" : "var(--surface-2)"};`}>
 <div style="display:flex; justify-content:space-between; align-items:center;">
 <div onClick={() => setSelectedId(w.id)} style="cursor:pointer; flex:1;">
 <strong style="font-size:13px;">{w.name}</strong>
 {w.orchestrator && <span style="background:#3a4a6a; color:white; padding:2px 6px; border-radius:3px; margin-left:8px; font-size:10px;">orch:{w.orchestrator}</span>}
 {w.cron_expr && <span style="color:var(--text-3); margin-left:8px; font-size:11px;">cron: {w.cron_expr}</span>}
 </div>
 <div style="display:flex; gap:4px;">
 <button class="link-btn" disabled={busy()} onClick={() => run(w.id)} style="background:#238636; color:white; padding:4px 10px; border:none; border-radius:4px;">▶ 실행</button>
 <button class="link-btn" disabled={busy()} onClick={() => loadForEdit(w)} style="padding:4px 10px;">수정</button>
 <button class="link-btn" disabled={busy()} onClick={() => del(w.id)} style="padding:4px 8px; background:var(--surface); border:1px solid #f85149; color:#f85149; border-radius:4px;">삭제</button>
 </div>
 </div>
 {w.description && <div style="font-size:11px; color:var(--text-2); margin-top:6px;">{w.description}</div>}
 <Show when={selectedId() === w.id && (runs() ?? []).length > 0}>
 <div style="margin-top:8px; padding-top:8px; border-top:1px solid var(--border);">
 <strong style="font-size:11px; color:var(--text-3);">실행 이력 ({(runs() ?? []).length})</strong>
 <For each={(runs() ?? []).slice(0, 10)}>
 {(r) => (
 <div style="font-size:10px; padding:4px 0; display:flex; gap:8px; align-items:center;">
 <span style={`padding:2px 6px; border-radius:3px; background:${r.status === "success" ? "#238636" : r.status === "failed" ? "#f85149" : r.status === "waiting_human" ? "#d29922" : "#3a4a6a"}; color:white;`}>{r.status}</span>
 <span style="color:var(--text-3);">{r.started_at.slice(0,19)}</span>
 {r.current_step && <span>step: {r.current_step}</span>}
 {r.error && <span style="color:#f85149;">{r.error.slice(0,60)}</span>}
 <Show when={r.status === "waiting_human"}>
 <button class="link-btn" disabled={busy()} onClick={() => approveRun(r.id)} style="background:#d29922; color:white; padding:2px 8px; border:none; border-radius:3px;">approve</button>
 </Show>
 </div>
 )}
 </For>
 </div>
 </Show>
 </div>
 )}
 </For>
 </section>
);
}
