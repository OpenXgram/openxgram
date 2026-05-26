import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";

// rc.132 — agency-agents 카탈로그 (msitarzewski/agency-agents).
// 카테고리 select + grid + 선택 → preview + 적용 → AGENT.md 생성.

interface TemplateDto {
 id: string;
 source_repo: string;
 source_path: string | null;
 category: string;
 name: string;
 description: string | null;
 color: string | null;
 emoji: string | null;
 vibe: string | null;
 body: string;
 customized: boolean;
 fetched_at: string;
 updated_at: string;
}

export function AgentTemplatesCard(props: { onBack: () => void}) {
 const [templates, { refetch}] = createResource<TemplateDto[]>(async () => {
 try { return await invoke<TemplateDto[]>("agent_templates_list");} catch { return [];}
});
 const [category, setCategory] = createSignal<string>("");
 const [selected, setSelected] = createSignal<TemplateDto | null>(null);
 const [busy, setBusy] = createSignal(false);
 const [msg, setMsg] = createSignal<string | null>(null);

 const categories = () => {
 const set = new Set<string>();
 (templates() ?? []).forEach((t) => set.add(t.category));
 return Array.from(set).sort();
};
 const filtered = () => {
 const c = category();
 const all = templates() ?? [];
 return c ? all.filter((t) => t.category === c) : all;
};

 async function refresh() {
 setBusy(true); setMsg("🔄 GitHub 에서 fetch 중...");
 try {
 const r = await invoke<any>("agent_templates_refresh");
 setMsg(`✓ 갱신: fetched=${r?.fetched} inserted=${r?.inserted} updated=${r?.updated} preserved=${r?.preserved}`);
 await refetch();
} catch (e) { setMsg(`✗ ${e}`);} finally { setBusy(false);}
}

 return (
 <section class="card-section">
 <button type="button" class="link-btn" onClick={props.onBack} style="margin-bottom:8px;">← 홈</button>
 <h3>📚 에이전트 카탈로그</h3>
 <p style="font-size:12px; color:var(--text-3); margin-bottom:10px;">
 <a href="https://github.com/msitarzewski/agency-agents" target="_blank" style="color:#5fa;">msitarzewski/agency-agents</a> 의 분류된 에이전트 템플릿.
 선택 → AGENT.md 자동 생성 → 메신저 등록.
 </p>

 <div style="display:flex; gap:8px; align-items:center; margin-bottom:12px;">
 <button class="link-btn" disabled={busy()} onClick={refresh}
 style="background:#238636; color:white; padding:6px 14px; border:none; border-radius:4px;">
 🔄 카탈로그 갱신
 </button>
 <select value={category()} onChange={(e) => setCategory(e.currentTarget.value)}
 style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="">— 모든 카테고리 ({(templates() ?? []).length}) —</option>
 <For each={categories()}>{(c) => <option value={c}>{c} ({(templates() ?? []).filter((t) => t.category === c).length})</option>}</For>
 </select>
 <Show when={msg()}>
 <span style={`font-size:11px; padding:4px 8px; border-radius:4px; background:${msg()!.startsWith("✓") || msg()!.startsWith("🔄") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>{msg()}</span>
 </Show>
 </div>

 <Show when={(templates() ?? []).length === 0}>
 <div style="padding:20px; text-align:center; color:var(--text-3);">
 카탈로그 비어있음. 위 <strong>🔄 카탈로그 갱신</strong> 클릭으로 fetch.
 </div>
 </Show>

 <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(280px, 1fr)); gap:10px;">
 <For each={filtered()}>
 {(t) => (
 <div onClick={() => setSelected(t)} style={`padding:10px; border:1px solid var(--border); border-radius:6px; cursor:pointer; background:${selected()?.id === t.id ? "rgba(35,134,54,0.15)" : "var(--surface-2)"};`}>
 <div style="display:flex; align-items:center; gap:6px; margin-bottom:4px;">
 <span style="font-size:18px;">{t.emoji || "🤖"}</span>
 <strong style="font-size:13px;">{t.name}</strong>
 {t.customized && <span style="background:#d29922; color:white; padding:1px 6px; border-radius:3px; font-size:9px;">사용자 수정</span>}
 </div>
 <div style="font-size:10px; color:var(--text-3); margin-bottom:4px;">📁 {t.category}</div>
 <Show when={t.vibe}>
 <div style="font-size:11px; color:var(--text-2); font-style:italic; margin-bottom:4px;">"{t.vibe}"</div>
 </Show>
 <Show when={t.description}>
 <div style="font-size:11px; color:var(--text-2); line-height:1.4; max-height:60px; overflow:hidden;">{t.description}</div>
 </Show>
 </div>
 )}
 </For>
 </div>

 {/* 선택 시 detail */}
 <Show when={selected()}>
 <div style="margin-top:14px; padding:12px; background:var(--surface-2); border:2px solid #238636; border-radius:6px;">
 <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:8px;">
 <h4 style="margin:0;">{selected()!.emoji || "🤖"} {selected()!.name}</h4>
 <button class="link-btn" onClick={() => setSelected(null)} style="padding:4px 10px;">닫기</button>
 </div>
 <div style="font-size:11px; color:var(--text-3); margin-bottom:6px;">📁 {selected()!.category} · source: {selected()!.source_path}</div>
 <Show when={selected()!.vibe}>
 <p style="font-style:italic; color:var(--text-2);">"{selected()!.vibe}"</p>
 </Show>
 <Show when={selected()!.description}>
 <p style="font-size:12px; color:var(--text-2);">{selected()!.description}</p>
 </Show>
 <details style="margin-top:8px;">
 <summary style="cursor:pointer; font-size:12px; color:var(--text-3);">📝 본문 보기 ({selected()!.body.length} chars)</summary>
 <pre style="font-size:10px; padding:8px; background:var(--surface); border-radius:4px; max-height:400px; overflow:auto; white-space:pre-wrap;">{selected()!.body}</pre>
 </details>
 <p style="font-size:11px; color:#d29922; margin-top:8px;">
 ⚠️ \"적용\" 기능 (AGENT.md 자동 생성 + 메신저 등록) 은 다음 cycle (rc.133).
 </p>
 </div>
 </Show>
 </section>
);
}
