import { createSignal, createResource, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// OpenXgram 런타임(하네스) — 컴포저↔에이전트 사이 제어 레이어. 에이전트별/전역.
// 검증된 실제 이벤트(코드 기준)마다 "할 일"을 설정. 전체폭 단일 컬럼.

type RuntimeConfig = {
  inject_memory: boolean; memory_count: number; memory_kinds: string[]; inject_wiki: boolean;
  search_enabled: boolean;
  perm_default: string; model_default: string; thinking_default: string; max_inject_chars: number;
  mandatory_note: string;
  extract_on_end: boolean;
  deny_patterns: string;
};
const DEF: RuntimeConfig = {
  inject_memory: true, memory_count: 8, memory_kinds: ["fact", "decision", "rule", "reference"], inject_wiki: false,
  search_enabled: false,
  perm_default: "bypassPermissions", model_default: "default", thinking_default: "high", max_inject_chars: 6000,
  mandatory_note: "", extract_on_end: false, deny_patterns: "",
};
const KIND: Record<string, string> = { fact: "사실", decision: "결정", rule: "규칙", reference: "참조" };
const ctl = "padding:6px 8px; background:#f7f8fa; color:var(--kk-ink); border:1px solid var(--kk-line); border-radius:6px; font-size:13px;";
const ev = "border:1px solid var(--kk-line); border-radius:10px; padding:14px 16px; margin-bottom:12px; color:var(--kk-ink);";
const evh = "font-weight:700; font-size:14px; margin-bottom:4px; color:var(--kk-ink);";
const hint = "color:var(--kk-sub); font-size:11.5px;";
const row = "display:flex; gap:10px; align-items:center; flex-wrap:wrap; margin-top:9px;";
const lbl = "min-width:84px; color:var(--kk-ink); font-size:13px;";

export function RuntimeTab() {
  const [target, setTarget] = createSignal("");
  const [cfg, setCfg] = createSignal<RuntimeConfig>(DEF);
  const [inherited, setInherited] = createSignal(false);
  const [saved, setSaved] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [agents] = createResource<any[]>(() => invoke("agents_list"));

  createResource(target, async (t) => {
    try {
      const r = await invoke<{ config: Partial<RuntimeConfig>; inherited?: boolean }>("runtime_config_get", t ? { alias: t } : {});
      setCfg({ ...DEF, ...(r?.config ?? {}) }); setInherited(!!r?.inherited);
    } catch { setCfg(DEF); setInherited(false); }
    setSaved(null); return t;
  });
  const [ctx, { refetch }] = createResource(() => cfg().memory_count, async (count) => {
    try { return await invoke<{ memories: { kind: string; content: string }[]; wiki_count: number }>("runtime_context", { count: String(count) }); }
    catch { return { memories: [], wiki_count: 0 }; }
  });

  function set<K extends keyof RuntimeConfig>(k: K, v: RuntimeConfig[K]) { setCfg({ ...cfg(), [k]: v }); setSaved(null); }
  function tk(k: string) { const c = cfg().memory_kinds; set("memory_kinds", c.includes(k) ? c.filter((x) => x !== k) : [...c, k]); }
  async function save() {
    setBusy(true);
    try { await invoke("runtime_config_set", { config: cfg(), alias: target() || null }); setSaved(target() ? `'${target()}' 저장됨 — 새 대화부터` : "전역 기본값 저장"); setInherited(false); await refetch(); }
    catch (e) { setSaved(`저장 실패: ${(e as Error).message}`); } finally { setBusy(false); }
  }
  const injMems = () => (ctx()?.memories ?? []).filter((m) => cfg().memory_kinds.includes(m.kind));
  const cb = (k: keyof RuntimeConfig, label: string) => (
    <label style="display:flex; gap:8px; align-items:center; margin-top:9px; color:var(--kk-ink); font-size:13px;">
      <input type="checkbox" checked={cfg()[k] as boolean} onChange={(e) => set(k, e.currentTarget.checked as any)} /> {label}
    </label>
  );

  return (
    <div class="kk-set" style="height:100%; overflow:auto;">
      <div class="board" style="max-width:none;">
        <div class="bh"><h2>🧠 런타임 (하네스)</h2><span class="sub">이벤트별 훅 — 각 이벤트에 무엇을 할지. 에이전트별/전역.</span></div>
        <div class="bb" style="color:var(--kk-ink); max-width:none;">
          <div style="display:flex; gap:10px; align-items:center; margin-bottom:14px; flex-wrap:wrap;">
            <span style="font-weight:700;">설정 대상</span>
            <select value={target()} onInput={(e) => setTarget(e.currentTarget.value)} style={`${ctl} min-width:230px;`}>
              <option value="">🌐 전역 기본값 (모든 에이전트)</option>
              <For each={agents() ?? []}>{(a) => <option value={a.alias}>{a.display_name || a.alias}</option>}</For>
            </select>
            <Show when={target() && inherited()}><span style={hint}>↳ 전역 상속 중 (저장 시 이 에이전트 전용)</span></Show>
          </div>

          {/* 🟩 프롬프트 전송 전 */}
          <div style={ev}>
            <div style={evh}>🟩 프롬프트 전송 전 <span style={hint}>(session/prompt 직전)</span></div>
            <div style={hint}>이 이벤트에 할 일: 프롬프트 앞에 무엇을 합성해 보낼지</div>
            {cb("inject_memory", "기억(사실·결정·규칙) 주입")}
            <div style={row}><span style={lbl}>주입 종류</span><For each={["fact", "decision", "rule", "reference"]}>{(k) => <label style="display:flex; gap:3px; align-items:center; font-size:13px;"><input type="checkbox" checked={cfg().memory_kinds.includes(k)} onChange={() => tk(k)} />{KIND[k]}</label>}</For></div>
            <div style={row}><span style={lbl}>주입 개수</span><input type="number" min="0" max="50" value={cfg().memory_count} onInput={(e) => set("memory_count", parseInt(e.currentTarget.value) || 0)} style={`${ctl} width:80px;`} /><span style={hint}>최근 기억 N개 (조절 가능 · 많을수록 토큰↑)</span></div>
            {cb("search_enabled", "관련성 검색해 주입")}
            {cb("inject_wiki", "위키 제목 주입")}
            <div style="margin-top:10px;"><div style="color:var(--kk-ink); font-size:13px; margin-bottom:4px;">❗ 필수 규칙 (전송 전 반드시 주입 — 게이트)</div>
              <textarea rows="2" placeholder="예: 코드 수정 전 영향범위 보고." value={cfg().mandatory_note} onInput={(e) => set("mandatory_note", e.currentTarget.value)} style={`${ctl} width:100%; resize:vertical;`} /></div>
            <div style={row}><span style={lbl}>주입 상한</span><input type="number" min="0" step="1000" value={cfg().max_inject_chars} onInput={(e) => set("max_inject_chars", parseInt(e.currentTarget.value) || 0)} style={`${ctl} width:100px;`} /><span style={hint}>글자 (토큰 절감)</span></div>
          </div>

          {/* 🟦 세션 생성 */}
          <div style={ev}>
            <div style={evh}>🟦 세션 생성 <span style={hint}>(session/new + spawn)</span></div>
            <div style={hint}>이 이벤트에 할 일: 에이전트를 어떻게 띄울지</div>
            <div style={row}><span style={lbl}>기본 권한</span><select value={cfg().perm_default} onInput={(e) => set("perm_default", e.currentTarget.value)} style={ctl}><option value="bypassPermissions">전체 허용 (Bypass)</option><option value="acceptEdits">편집 허용</option><option value="default">허용목록</option><option value="plan">읽기전용</option></select></div>
            <div style={row}><span style={lbl}>기본 모델</span><input value={cfg().model_default} onInput={(e) => set("model_default", e.currentTarget.value)} style={`${ctl} width:170px;`} /></div>
            <div style={row}><span style={lbl}>기본 effort</span><select value={cfg().thinking_default} onInput={(e) => set("thinking_default", e.currentTarget.value)} style={ctl}><For each={["high", "medium", "low", "off"]}>{(o) => <option value={o}>{o}</option>}</For></select></div>
          </div>

          {/* 🟨 도구 승인 */}
          <div style={ev}>
            <div style={evh}>🟨 도구 승인 <span style={hint}>(session/request_permission)</span></div>
            <div style={hint}>이 이벤트에 할 일: 도구 호출 허용/차단</div>
            <div style="margin-top:9px; color:var(--kk-ink); font-size:13px;">차단 패턴 (한 줄에 하나 — 해당 명령 거부)</div>
            <textarea rows="2" placeholder={"예:\nrm -rf\ngit push --force"} value={cfg().deny_patterns} onInput={(e) => set("deny_patterns", e.currentTarget.value)} style={`${ctl} width:100%; resize:vertical; margin-top:4px;`} />
            <div style={hint}>※ 정책 기본은 위 '기본 권한'. 패턴 차단 enforcement 는 다음 단계 연결.</div>
          </div>

          {/* 🟪 턴 종료 */}
          <div style={ev}>
            <div style={evh}>🟪 턴 종료 <span style={hint}>(stopReason)</span></div>
            <div style={hint}>이 이벤트에 할 일: 응답 끝나면</div>
            {cb("extract_on_end", "핵심 기억 자동 추출(L0→L2 승격) — 다음 단계 연결")}
            <div style={hint} class="">현재: 응답 영속 기록 + 읽음 + 대기열 발신(자동).</div>
          </div>

          <div style="display:flex; gap:10px; align-items:center; margin:6px 0 18px;">
            <button class="qbtn" disabled={busy()} onClick={save}>{busy() ? "저장 중…" : "설정 저장"}</button>
            <Show when={saved()}><span style="color:var(--kk-ink); font-size:13px;">{saved()}</span></Show>
          </div>

          {/* 관찰 */}
          <div style={evh}>🔎 관찰 — 지금 이 설정으로 주입될 것</div>
          <Show when={!ctx.loading} fallback={<div style={hint}>불러오는 중…</div>}>
            <div style={ev}>
              <div style="color:var(--kk-ink); font-size:13px; margin-bottom:6px;">기억 {cfg().inject_memory ? injMems().length : 0}개 · 위키 {cfg().inject_wiki ? (ctx()?.wiki_count ?? 0) : 0}개 · 필수규칙 {cfg().mandatory_note.trim() ? "있음" : "없음"}</div>
              <Show when={cfg().inject_memory}><For each={injMems()}>{(m) => <div style="font-size:12.5px; padding:4px 0; border-top:1px solid var(--kk-line); color:var(--kk-ink);"><b>[{KIND[m.kind] ?? m.kind}]</b> {m.content.slice(0, 160)}</div>}</For></Show>
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
}
