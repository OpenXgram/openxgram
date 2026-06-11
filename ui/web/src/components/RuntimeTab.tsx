import { createSignal, createResource, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// OpenXgram 런타임(하네스) — 컴포저↔에이전트 사이 제어 레이어. 에이전트별 또는 전역 기본값으로
// 설정. 검증된 실제 이벤트(프롬프트 전송 전 / 세션 생성 / 도구 승인 / 턴 종료 / 도메인 tick)별 액션.

type RuntimeConfig = {
  inject_memory: boolean; memory_count: number; memory_kinds: string[]; inject_wiki: boolean;
  search_enabled: boolean; search_source: string;
  perm_default: string; model_default: string; thinking_default: string; max_inject_chars: number;
  mandatory_note: string;
};
const DEFAULTS: RuntimeConfig = {
  inject_memory: true, memory_count: 8, memory_kinds: ["fact", "decision", "rule", "reference"], inject_wiki: false,
  search_enabled: false, search_source: "last_message",
  perm_default: "bypassPermissions", model_default: "default", thinking_default: "high", max_inject_chars: 6000,
  mandatory_note: "",
};
const KIND: Record<string, string> = { fact: "사실", decision: "결정", rule: "규칙", reference: "참조" };
const ctl = "padding:6px 8px; background:var(--bg-soft); color:inherit; border:1px solid var(--border); border-radius:6px; font-size:13px;";
const card = "background:var(--bg-soft); border:1px solid var(--border); border-radius:10px; padding:14px 16px;";

export function RuntimeTab() {
  const [target, setTarget] = createSignal(""); // "" = 전역 기본값, 아니면 에이전트 alias
  const [cfg, setCfg] = createSignal<RuntimeConfig>(DEFAULTS);
  const [inherited, setInherited] = createSignal(false);
  const [saved, setSaved] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const [agents] = createResource<any[]>(() => invoke("agents_list"));

  // target 바뀌면 그 에이전트(또는 전역) config 로드.
  createResource(target, async (t) => {
    try {
      const r = await invoke<{ config: Partial<RuntimeConfig>; inherited?: boolean }>("runtime_config_get", t ? { alias: t } : {});
      setCfg({ ...DEFAULTS, ...(r?.config ?? {}) });
      setInherited(!!r?.inherited);
    } catch { setCfg(DEFAULTS); setInherited(false); }
    setSaved(null);
    return t;
  });

  const [ctx, { refetch: refetchCtx }] = createResource(
    () => cfg().memory_count,
    async (count) => {
      try { return await invoke<{ memories: { kind: string; content: string }[]; wiki: { id: string; title: string }[]; wiki_count: number }>("runtime_context", { count: String(count) }); }
      catch { return { memories: [], wiki: [], wiki_count: 0 }; }
    },
  );

  function set<K extends keyof RuntimeConfig>(k: K, v: RuntimeConfig[K]) { setCfg({ ...cfg(), [k]: v }); setSaved(null); }
  function toggleKind(k: string) { const c = cfg().memory_kinds; set("memory_kinds", c.includes(k) ? c.filter((x) => x !== k) : [...c, k]); }
  async function save() {
    setBusy(true);
    try { await invoke("runtime_config_set", { config: cfg(), alias: target() || null }); setSaved(target() ? `'${target()}' 에이전트에 저장 — 새 대화부터 적용` : "전역 기본값 저장"); setInherited(false); await refetchCtx(); }
    catch (e) { setSaved(`저장 실패: ${(e as Error).message}`); } finally { setBusy(false); }
  }
  const injMems = () => (ctx()?.memories ?? []).filter((m) => cfg().memory_kinds.includes(m.kind));

  return (
    <div class="kk-set" style="height:100%; overflow:auto;">
      <div class="board" style="max-width:none;">
        <div class="bh">
          <h2>🧠 런타임 (하네스)</h2>
          <span class="sub">이벤트별 훅 — 무엇을 주입·제한·필수로 할지. 에이전트별 또는 전역 기본값.</span>
        </div>
        <div class="bb">
          {/* 대상 선택 — per-agent */}
          <div style="display:flex; gap:10px; align-items:center; margin-bottom:6px; flex-wrap:wrap;">
            <span style="font-weight:600;">설정 대상</span>
            <select value={target()} onInput={(e) => setTarget(e.currentTarget.value)} style={`${ctl} min-width:220px;`}>
              <option value="">🌐 전역 기본값 (모든 에이전트)</option>
              <For each={agents() ?? []}>{(a) => <option value={a.alias}>{a.display_name || a.alias}</option>}</For>
            </select>
            <Show when={target() && inherited()}><span class="sub" style="font-size:12px;">↳ 전역 기본값 상속 중 (저장 시 이 에이전트 전용)</span></Show>
          </div>

          {/* 2열 그리드 — 폭 활용 */}
          <div style="display:grid; grid-template-columns:repeat(auto-fit, minmax(330px, 1fr)); gap:14px; align-items:start;">
            {/* 🟩 프롬프트 전송 전 */}
            <div style={card}>
              <div style="font-weight:600; margin-bottom:10px;">🟩 프롬프트 전송 전 <span class="sub" style="font-weight:400;">(session/prompt 직전)</span></div>
              <div style="display:flex; flex-direction:column; gap:10px;">
                <label style="display:flex; gap:8px; align-items:center;"><input type="checkbox" checked={cfg().inject_memory} onChange={(e) => set("inject_memory", e.currentTarget.checked)} /> 기억(사실·결정·규칙) 주입</label>
                <div style="display:flex; gap:6px; align-items:center; flex-wrap:wrap;">
                  <span class="sub" style="min-width:64px;">종류</span>
                  <For each={["fact", "decision", "rule", "reference"]}>{(k) => <label style="display:flex; gap:3px; align-items:center;"><input type="checkbox" checked={cfg().memory_kinds.includes(k)} onChange={() => toggleKind(k)} />{KIND[k]}</label>}</For>
                </div>
                <div style="display:flex; gap:8px; align-items:center;">
                  <span class="sub" style="min-width:64px;">개수</span>
                  <input type="number" min="0" max="50" value={cfg().memory_count} onInput={(e) => set("memory_count", parseInt(e.currentTarget.value) || 0)} style={`${ctl} width:80px;`} />
                  <span class="sub" style="font-size:11.5px;">최근 기억 N개까지 (조절 가능 · 많을수록 토큰↑)</span>
                </div>
                <label style="display:flex; gap:8px; align-items:center;"><input type="checkbox" checked={cfg().search_enabled} onChange={(e) => set("search_enabled", e.currentTarget.checked)} /> 관련성 검색 주입</label>
                <label style="display:flex; gap:8px; align-items:center;"><input type="checkbox" checked={cfg().inject_wiki} onChange={(e) => set("inject_wiki", e.currentTarget.checked)} /> 위키 제목 주입</label>
                <div>
                  <div class="sub" style="margin-bottom:4px;">❗ 필수 규칙 (전송 전 반드시 주입 — 게이트)</div>
                  <textarea rows="2" placeholder="예: 코드 수정 전 영향범위 보고. 답변 전 안전 규칙 확인." value={cfg().mandatory_note} onInput={(e) => set("mandatory_note", e.currentTarget.value)} style={`${ctl} width:100%; resize:vertical;`} />
                </div>
              </div>
            </div>

            {/* 🟦 세션 생성 + 제한 */}
            <div style={card}>
              <div style="font-weight:600; margin-bottom:10px;">🟦 세션 생성 <span class="sub" style="font-weight:400;">(session/new + spawn)</span></div>
              <div style="display:flex; flex-direction:column; gap:10px;">
                <div style="display:flex; gap:8px; align-items:center;"><span class="sub" style="min-width:80px;">기본 권한</span>
                  <select value={cfg().perm_default} onInput={(e) => set("perm_default", e.currentTarget.value)} style={ctl}>
                    <option value="bypassPermissions">전체 허용 (Bypass)</option><option value="acceptEdits">편집 허용</option><option value="default">허용목록</option><option value="plan">읽기전용</option>
                  </select>
                </div>
                <div style="display:flex; gap:8px; align-items:center;"><span class="sub" style="min-width:80px;">기본 모델</span><input value={cfg().model_default} onInput={(e) => set("model_default", e.currentTarget.value)} style={`${ctl} width:160px;`} /></div>
                <div style="display:flex; gap:8px; align-items:center;"><span class="sub" style="min-width:80px;">기본 effort</span>
                  <select value={cfg().thinking_default} onInput={(e) => set("thinking_default", e.currentTarget.value)} style={ctl}><For each={["high", "medium", "low", "off"]}>{(o) => <option value={o}>{o}</option>}</For></select>
                </div>
                <div style="display:flex; gap:8px; align-items:center;"><span class="sub" style="min-width:80px;">주입 상한</span><input type="number" min="0" step="1000" value={cfg().max_inject_chars} onInput={(e) => set("max_inject_chars", parseInt(e.currentTarget.value) || 0)} style={`${ctl} width:100px;`} /><span class="sub" style="font-size:11.5px;">글자 (토큰 절감)</span></div>
              </div>
              <div style="font-weight:600; margin:14px 0 6px;">🟨 도구 승인 <span class="sub" style="font-weight:400;">(request_permission)</span> · 🟪 턴 종료 <span class="sub" style="font-weight:400;">(stopReason)</span></div>
              <div class="sub" style="font-size:12px; line-height:1.6;">도구 권한은 위 기본 권한으로 결정. 패턴별 allow/deny는 다음 단계. 턴 종료 시 응답 기록·읽음·큐 발신(자동), 향후 기억 자동추출.</div>
            </div>
          </div>

          <div style="margin-top:14px; display:flex; gap:10px; align-items:center;">
            <button class="qbtn" disabled={busy()} onClick={save}>{busy() ? "저장 중…" : "설정 저장"}</button>
            <Show when={saved()}><span class="sub">{saved()}</span></Show>
          </div>

          {/* 관찰 */}
          <div style="font-weight:600; margin:18px 0 8px;">🔎 관찰 — 지금 이 설정으로 주입될 것</div>
          <Show when={!ctx.loading} fallback={<div class="sub">불러오는 중…</div>}>
            <div style={card}>
              <div class="sub" style="margin-bottom:6px;">기억 {cfg().inject_memory ? injMems().length : 0}개 · 위키 {cfg().inject_wiki ? (ctx()?.wiki_count ?? 0) : 0}개 · 필수규칙 {cfg().mandatory_note.trim() ? "있음" : "없음"}{!cfg().inject_memory && " (기억 주입 꺼짐)"}</div>
              <Show when={cfg().inject_memory}>
                <For each={injMems()}>{(m) => <div style="font-size:12.5px; padding:4px 0; border-top:1px solid var(--border);"><b>[{KIND[m.kind] ?? m.kind}]</b> {m.content.slice(0, 160)}</div>}</For>
              </Show>
            </div>
          </Show>
          <div style="height:24px;" />
        </div>
      </div>
    </div>
  );
}
