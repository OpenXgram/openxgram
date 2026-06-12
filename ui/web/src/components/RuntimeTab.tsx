import { createSignal, createResource, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// OpenXgram 런타임(하네스) — 컴포저↔에이전트 사이 제어 레이어. 에이전트별/전역.
// 검증된 실제 이벤트마다 "할 일"을 설정. 전체폭. 기억은 개별 선택(핀), 위키는 검색+선택.

type Mem = { id: string; kind: string; content: string };
type Wiki = { id: string; title: string };

type RuntimeConfig = {
  inject_memory: boolean; memory_count: number; memory_kinds: string[]; memory_pins: string[];
  inject_wiki: boolean; wiki_pins: string[]; search_enabled: boolean;
  perm_default: string; model_default: string; thinking_default: string; max_inject_chars: number;
  mandatory_note: string; extract_on_end: boolean; deny_patterns: string;
};
const DEF: RuntimeConfig = {
  inject_memory: true, memory_count: 8, memory_kinds: ["fact", "decision", "rule", "reference"], memory_pins: [],
  inject_wiki: false, wiki_pins: [], search_enabled: false,
  perm_default: "bypassPermissions", model_default: "default", thinking_default: "high", max_inject_chars: 6000,
  mandatory_note: "", extract_on_end: false, deny_patterns: "",
};
const KIND: Record<string, string> = { fact: "사실", decision: "결정", rule: "규칙", reference: "참조" };
// 도구 승인 — 클릭하면 차단 패턴에 들어가는 예시(자주 쓰는 위험 명령).
const DENY_EXAMPLES = ["rm -rf", "git push --force", "git push -f", "sudo", "chmod 777", "DROP TABLE", "DELETE FROM", "> /dev/sd", "mkfs", "curl | sh"];

const ctl = "padding:6px 8px; background:#f7f8fa; color:var(--kk-ink); border:1px solid var(--kk-line); border-radius:6px; font-size:13px;";
const ev = "border:1px solid var(--kk-line); border-radius:10px; padding:14px 16px; margin-bottom:12px; color:var(--kk-ink);";
const evh = "font-weight:700; font-size:14px; margin-bottom:4px; color:var(--kk-ink);";
const hint = "color:var(--kk-sub); font-size:11.5px;";
const row = "display:flex; gap:10px; align-items:center; flex-wrap:wrap; margin-top:9px;";
const lbl = "min-width:84px; color:var(--kk-ink); font-size:13px;";
const chip = "font-size:11.5px; padding:3px 9px; border:1px solid var(--kk-line); border-radius:14px; background:#f1f3f6; color:#54607a; cursor:pointer;";

export function RuntimeTab() {
  const [target, setTarget] = createSignal("");
  const [cfg, setCfg] = createSignal<RuntimeConfig>(DEF);
  const [inherited, setInherited] = createSignal(false);
  const [saved, setSaved] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [wikiFilter, setWikiFilter] = createSignal("");
  const [memFilter, setMemFilter] = createSignal("");
  const [agents] = createResource<any[]>(() => invoke("agents_list"));

  createResource(target, async (t) => {
    try {
      const r = await invoke<{ config: Partial<RuntimeConfig>; inherited?: boolean }>("runtime_config_get", t ? { alias: t } : {});
      setCfg({ ...DEF, ...(r?.config ?? {}) }); setInherited(!!r?.inherited);
    } catch { setCfg(DEF); setInherited(false); }
    setSaved(null); return t;
  });
  // 후보 풀은 넉넉히(최대 50) 가져와 선택지로 보여준다 — 주입은 핀/종류/개수로 결정.
  const [ctx, { refetch }] = createResource(async () => {
    try { return await invoke<{ memories: Mem[]; wiki: Wiki[]; wiki_count: number }>("runtime_context", { count: "50" }); }
    catch { return { memories: [], wiki: [], wiki_count: 0 }; }
  });

  function set<K extends keyof RuntimeConfig>(k: K, v: RuntimeConfig[K]) { setCfg({ ...cfg(), [k]: v }); setSaved(null); }
  function tk(k: string) { const c = cfg().memory_kinds; set("memory_kinds", c.includes(k) ? c.filter((x) => x !== k) : [...c, k]); }
  function pinMem(id: string) { const p = cfg().memory_pins; set("memory_pins", p.includes(id) ? p.filter((x) => x !== id) : [...p, id]); }
  function pinWiki(id: string) { const p = cfg().wiki_pins; set("wiki_pins", p.includes(id) ? p.filter((x) => x !== id) : [...p, id]); }
  function addDeny(ex: string) {
    const cur = cfg().deny_patterns.split("\n").map((s) => s.trim()).filter(Boolean);
    if (!cur.includes(ex)) set("deny_patterns", [...cur, ex].join("\n"));
  }
  async function save() {
    setBusy(true);
    try { await invoke("runtime_config_set", { config: cfg(), alias: target() || null }); setSaved(target() ? `'${target()}' 저장됨 — 새 대화부터` : "전역 기본값 저장"); setInherited(false); }
    catch (e) { setSaved(`저장 실패: ${(e as Error).message}`); } finally { setBusy(false); }
  }

  const allMems = () => ctx()?.memories ?? [];
  const allWiki = () => ctx()?.wiki ?? [];
  // 종류 필터 + 텍스트 필터 적용된 후보 기억.
  const memCands = () => allMems().filter((m) => cfg().memory_kinds.includes(m.kind) && (!memFilter() || m.content.toLowerCase().includes(memFilter().toLowerCase())));
  const wikiCands = () => allWiki().filter((w) => !wikiFilter() || w.title.toLowerCase().includes(wikiFilter().toLowerCase()));
  // 실제 주입될 기억: 핀이 있으면 핀만, 없으면 종류 필터 후 최근 N개.
  const injMems = () => {
    const pins = allMems().filter((m) => cfg().memory_pins.includes(m.id));
    if (pins.length) return pins;
    return allMems().filter((m) => cfg().memory_kinds.includes(m.kind)).slice(0, cfg().memory_count);
  };
  const injWiki = () => allWiki().filter((w) => cfg().wiki_pins.includes(w.id));

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

          {/* 🟩 프롬프트 전송 전 — 기억/위키/필수규칙 합성 */}
          <div style={ev}>
            <div style={evh}>🟩 프롬프트 전송 전 <span style={hint}>(session/prompt 직전)</span></div>
            <div style={hint}>이 이벤트에 할 일: 프롬프트 앞에 무엇을 합성해 보낼지</div>

            <label style="display:flex; gap:8px; align-items:center; margin-top:11px; font-size:13px; font-weight:600;">
              <input type="checkbox" checked={cfg().inject_memory} onChange={(e) => set("inject_memory", e.currentTarget.checked)} /> 기억(L2) 주입
            </label>
            <Show when={cfg().inject_memory}>
              <div style="margin:8px 0 0 4px;">
                <div style={row}>
                  <span style={lbl}>종류 필터</span>
                  <For each={["fact", "decision", "rule", "reference"]}>{(k) => <label style="display:flex; gap:3px; align-items:center; font-size:12.5px;"><input type="checkbox" checked={cfg().memory_kinds.includes(k)} onChange={() => tk(k)} />{KIND[k]}</label>}</For>
                  <input placeholder="🔍 기억 내용 검색" value={memFilter()} onInput={(e) => setMemFilter(e.currentTarget.value)} style={`${ctl} flex:1; min-width:160px;`} />
                </div>
                <div style="margin-top:6px; font-size:12px; color:var(--kk-sub);">주입할 기억 선택 — 체크한 것만 주입(핀). <b>하나도 안 고르면</b> 위 종류의 최근 <input type="number" min="0" max="50" value={cfg().memory_count} onInput={(e) => set("memory_count", parseInt(e.currentTarget.value) || 0)} style={`${ctl} width:56px; padding:2px 6px;`} />개.</div>
                <div style="max-height:240px; overflow:auto; border:1px solid var(--kk-line); border-radius:8px; margin-top:6px;">
                  <Show when={memCands().length} fallback={<div style="padding:12px; font-size:12px; color:var(--kk-sub);">해당 기억 없음.</div>}>
                    <For each={memCands()}>{(m) => (
                      <label style={`display:flex; gap:8px; padding:7px 10px; border-top:1px solid var(--kk-line); cursor:pointer; font-size:12.5px; ${cfg().memory_pins.includes(m.id) ? "background:#eef6ef;" : ""}`}>
                        <input type="checkbox" checked={cfg().memory_pins.includes(m.id)} onChange={() => pinMem(m.id)} style="margin-top:2px;" />
                        <span><b style="color:#54607a;">[{KIND[m.kind] ?? m.kind}]</b> {m.content.slice(0, 180)}</span>
                      </label>
                    )}</For>
                  </Show>
                </div>
                <div style={hint}>선택 {cfg().memory_pins.length}개 핀 · 후보 {memCands().length}개</div>
              </div>
            </Show>

            <label style="display:flex; gap:8px; align-items:center; margin-top:14px; font-size:13px; font-weight:600;">
              <input type="checkbox" checked={cfg().inject_wiki} onChange={(e) => set("inject_wiki", e.currentTarget.checked)} /> 위키(LLM Wiki) 주입
            </label>
            <Show when={cfg().inject_wiki}>
              <div style="margin:8px 0 0 4px;">
                <input placeholder="🔍 위키 제목 검색" value={wikiFilter()} onInput={(e) => setWikiFilter(e.currentTarget.value)} style={`${ctl} width:100%;`} />
                <div style="max-height:200px; overflow:auto; border:1px solid var(--kk-line); border-radius:8px; margin-top:6px;">
                  <Show when={wikiCands().length} fallback={<div style="padding:12px; font-size:12px; color:var(--kk-sub);">위키 페이지 없음.</div>}>
                    <For each={wikiCands()}>{(w) => (
                      <label style={`display:flex; gap:8px; padding:7px 10px; border-top:1px solid var(--kk-line); cursor:pointer; font-size:12.5px; ${cfg().wiki_pins.includes(w.id) ? "background:#eef6ef;" : ""}`}>
                        <input type="checkbox" checked={cfg().wiki_pins.includes(w.id)} onChange={() => pinWiki(w.id)} />
                        <span>📄 {w.title}</span>
                      </label>
                    )}</For>
                  </Show>
                </div>
                <div style={hint}>선택 {cfg().wiki_pins.length}개</div>
              </div>
            </Show>

            <label style="display:flex; gap:8px; align-items:center; margin-top:14px; font-size:13px;">
              <input type="checkbox" checked={cfg().search_enabled} onChange={(e) => set("search_enabled", e.currentTarget.checked)} /> 관련성 검색해 주입 (프롬프트와 유사한 기억 자동 추가)
            </label>

            <div style="margin-top:12px;"><div style="color:var(--kk-ink); font-size:13px; font-weight:600; margin-bottom:4px;">❗ 사전 주입 필수 규칙 (전송 전 반드시 — 게이트)</div>
              <textarea rows="2" placeholder="예: 코드 수정 전 영향범위 보고. DB 변경은 승인 후." value={cfg().mandatory_note} onInput={(e) => set("mandatory_note", e.currentTarget.value)} style={`${ctl} width:100%; resize:vertical;`} /></div>
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

          {/* 🟨 도구 승인 — 예시 칩 클릭으로 쉽게 입력 */}
          <div style={ev}>
            <div style={evh}>🟨 도구 승인 <span style={hint}>(session/request_permission)</span></div>
            <div style={hint}>이 이벤트에 할 일: 도구 호출 허용/차단</div>
            <div style="margin-top:9px; color:var(--kk-ink); font-size:13px;">차단 패턴 (한 줄에 하나 — 포함 시 거부)</div>
            <div style="display:flex; gap:6px; flex-wrap:wrap; margin-top:6px;">
              <span style={hint}>클릭해 추가:</span>
              <For each={DENY_EXAMPLES}>{(ex) => <span style={chip} onClick={() => addDeny(ex)} title="클릭하면 차단 패턴에 추가">+ {ex}</span>}</For>
            </div>
            <textarea rows="3" placeholder={"위 칩을 클릭하거나 직접 입력\n예:\nrm -rf\ngit push --force"} value={cfg().deny_patterns} onInput={(e) => set("deny_patterns", e.currentTarget.value)} style={`${ctl} width:100%; resize:vertical; margin-top:6px;`} />
            <div style={hint}>※ 기본 정책은 위 '기본 권한'. 여기 패턴에 걸리면 그 도구 호출은 거부.</div>
          </div>

          {/* 🟪 턴 종료 */}
          <div style={ev}>
            <div style={evh}>🟪 턴 종료 <span style={hint}>(stopReason)</span></div>
            <div style={hint}>이 이벤트에 할 일: 응답 끝나면</div>
            <label style="display:flex; gap:8px; align-items:center; margin-top:9px; font-size:13px;">
              <input type="checkbox" checked={cfg().extract_on_end} onChange={(e) => set("extract_on_end", e.currentTarget.checked)} /> 핵심 기억 자동 추출(L0→L2 승격)
            </label>
            <div style={hint}>현재: 응답 영속 기록 + 읽음 처리 + 대기열 발신(자동).</div>
          </div>

          <div style="display:flex; gap:10px; align-items:center; margin:6px 0 18px;">
            <button class="qbtn" disabled={busy()} onClick={save}>{busy() ? "저장 중…" : "💾 설정 저장"}</button>
            <Show when={saved()}><span style="color:var(--kk-ink); font-size:13px;">{saved()}</span></Show>
          </div>

          {/* 관찰 — 실제 주입될 것 미리보기 */}
          <div style={evh}>🔎 관찰 — 지금 이 설정으로 주입될 것</div>
          <Show when={!ctx.loading} fallback={<div style={hint}>불러오는 중…</div>}>
            <div style={ev}>
              <div style="color:var(--kk-ink); font-size:13px; margin-bottom:6px;">기억 {cfg().inject_memory ? injMems().length : 0}개{cfg().memory_pins.length ? " (핀 선택)" : " (최근 N개)"} · 위키 {cfg().inject_wiki ? injWiki().length : 0}개 · 필수규칙 {cfg().mandatory_note.trim() ? "있음" : "없음"} · 차단패턴 {cfg().deny_patterns.split("\n").filter((s) => s.trim()).length}개</div>
              <Show when={cfg().inject_memory}><For each={injMems()}>{(m) => <div style="font-size:12.5px; padding:4px 0; border-top:1px solid var(--kk-line); color:var(--kk-ink);"><b>[{KIND[m.kind] ?? m.kind}]</b> {m.content.slice(0, 160)}</div>}</For></Show>
              <Show when={cfg().inject_wiki && injWiki().length}><For each={injWiki()}>{(w) => <div style="font-size:12.5px; padding:4px 0; border-top:1px solid var(--kk-line); color:var(--kk-ink);">📄 {w.title}</div>}</For></Show>
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
}
