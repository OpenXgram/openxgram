import { createSignal, createResource, createEffect, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// OpenXgram 런타임(하네스) 설정 — 컴포저↔에이전트 사이 제어 레이어.
// 주입(기억·위키) / 검색 / 제한(권한·모델·예산) / 필수(매 대화 거치는 규칙) 를 설정하고,
// "지금 무엇이 주입되는지" 관찰. config 는 백엔드 identity_settings 에 영속.

type RuntimeConfig = {
  inject_memory: boolean;
  memory_count: number;
  memory_kinds: string[];
  inject_wiki: boolean;
  search_enabled: boolean;
  search_source: string;
  perm_default: string;
  model_default: string;
  thinking_default: string;
  max_inject_chars: number;
  mandatory_note: string;
};

const DEFAULTS: RuntimeConfig = {
  inject_memory: true, memory_count: 8,
  memory_kinds: ["fact", "decision", "rule", "reference"], inject_wiki: false,
  search_enabled: false, search_source: "last_message",
  perm_default: "bypassPermissions", model_default: "default", thinking_default: "high",
  max_inject_chars: 6000, mandatory_note: "",
};

const KIND_LABELS: Record<string, string> = { fact: "사실", decision: "결정", rule: "규칙", reference: "참조" };

const inputStyle = "padding:6px 8px; background:var(--bg-soft); color:inherit; border:1px solid var(--border); border-radius:6px; font-size:13px;";

export function RuntimeTab() {
  const [cfg, setCfg] = createSignal<RuntimeConfig>(DEFAULTS);
  const [saved, setSaved] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const [loaded] = createResource(async () => {
    try {
      const r = await invoke<{ config: Partial<RuntimeConfig> }>("runtime_config_get");
      setCfg({ ...DEFAULTS, ...(r?.config ?? {}) });
    } catch { /* defaults */ }
    return true;
  });
  createEffect(() => { loaded(); });

  const [ctx, { refetch: refetchCtx }] = createResource(
    () => cfg().memory_count,
    async (count) => {
      try {
        return await invoke<{ memories: { kind: string; content: string }[]; wiki: { id: string; title: string }[]; memory_count: number; wiki_count: number }>(
          "runtime_context", { count: String(count) });
      } catch { return { memories: [], wiki: [], memory_count: 0, wiki_count: 0 }; }
    },
  );

  function set<K extends keyof RuntimeConfig>(k: K, v: RuntimeConfig[K]) { setCfg({ ...cfg(), [k]: v }); setSaved(null); }
  function toggleKind(k: string) {
    const cur = cfg().memory_kinds;
    set("memory_kinds", cur.includes(k) ? cur.filter((x) => x !== k) : [...cur, k]);
  }
  async function save() {
    setBusy(true);
    try { await invoke("runtime_config_set", { config: cfg() }); setSaved("저장됨 — 새 대화부터 적용"); await refetchCtx(); }
    catch (e) { setSaved(`저장 실패: ${(e as Error).message}`); }
    finally { setBusy(false); }
  }

  // 주입 미리보기 — memory_kinds 필터 반영.
  const injMems = () => (ctx()?.memories ?? []).filter((m) => cfg().memory_kinds.includes(m.kind));

  return (
    <div class="kk-set">
      <div class="board">
        <div class="bh">
          <h2>🧠 런타임 (하네스)</h2>
          <span class="sub">이벤트별 훅 — 입력 전 / 에이전트 호출 / 도구 실행 / 응답 후마다 무엇을 할지</span>
        </div>
        <div class="bb">
          {/* 이벤트: 프롬프트 전송 전 (session/prompt 직전, 우리가 합성) */}
          <div class="wsec">🟩 프롬프트 전송 전 (session/prompt 직전) — 무엇을 주입할지</div>
          <div style="display:flex; flex-direction:column; gap:10px;">
            <label style="display:flex; gap:8px; align-items:center;">
              <input type="checkbox" checked={cfg().inject_memory} onChange={(e) => set("inject_memory", e.currentTarget.checked)} />
              기억(사실·결정·규칙) 주입
            </label>
            <div style="display:flex; gap:8px; align-items:center; flex-wrap:wrap;">
              <span style="min-width:110px;">주입 종류</span>
              <For each={["fact", "decision", "rule", "reference"]}>
                {(k) => (
                  <label style="display:flex; gap:4px; align-items:center;">
                    <input type="checkbox" checked={cfg().memory_kinds.includes(k)} onChange={() => toggleKind(k)} /> {KIND_LABELS[k]}
                  </label>
                )}
              </For>
            </div>
            <div style="display:flex; gap:8px; align-items:center;">
              <span style="min-width:110px;">주입 개수</span>
              <input type="number" min="0" max="50" value={cfg().memory_count} onInput={(e) => set("memory_count", parseInt(e.currentTarget.value) || 0)} style={`${inputStyle} width:90px;`} />
            </div>
            <label style="display:flex; gap:8px; align-items:center;">
              <input type="checkbox" checked={cfg().inject_wiki} onChange={(e) => set("inject_wiki", e.currentTarget.checked)} /> 위키 제목 목록 주입
            </label>
          </div>

          {/* 🟩 전송 전 — 검색 주입 */}
          <div class="wsec">🟩 ↳ 검색 주입 — 관련 기억 자동 검색</div>
          <div style="display:flex; flex-direction:column; gap:10px;">
            <label style="display:flex; gap:8px; align-items:center;">
              <input type="checkbox" checked={cfg().search_enabled} onChange={(e) => set("search_enabled", e.currentTarget.checked)} />
              관련성 검색 활성 (최근 메시지로 관련 기억 검색)
            </label>
            <div style="display:flex; gap:8px; align-items:center;">
              <span style="min-width:110px;">검색 기준</span>
              <select value={cfg().search_source} onInput={(e) => set("search_source", e.currentTarget.value)} style={inputStyle}>
                <option value="last_message">마지막 사용자 메시지</option>
                <option value="goal">대화 목표</option>
              </select>
            </div>
          </div>

          {/* 이벤트: 세션 생성 (session/new + agent spawn) */}
          <div class="wsec">🟦 세션 생성 (session/new + spawn) — 권한·모델·effort</div>
          <div style="display:flex; flex-direction:column; gap:10px;">
            <div style="display:flex; gap:8px; align-items:center;">
              <span style="min-width:110px;">기본 권한</span>
              <select value={cfg().perm_default} onInput={(e) => set("perm_default", e.currentTarget.value)} style={inputStyle}>
                <option value="bypassPermissions">전체 허용 (Bypass)</option>
                <option value="acceptEdits">편집 허용 (Accept Edits)</option>
                <option value="default">허용목록 (Default)</option>
                <option value="plan">읽기전용 (Plan)</option>
              </select>
            </div>
            <div style="display:flex; gap:8px; align-items:center;">
              <span style="min-width:110px;">기본 모델</span>
              <input value={cfg().model_default} onInput={(e) => set("model_default", e.currentTarget.value)} style={`${inputStyle} width:180px;`} />
            </div>
            <div style="display:flex; gap:8px; align-items:center;">
              <span style="min-width:110px;">기본 effort</span>
              <select value={cfg().thinking_default} onInput={(e) => set("thinking_default", e.currentTarget.value)} style={inputStyle}>
                <For each={["high", "medium", "low", "off"]}>{(o) => <option value={o}>{o}</option>}</For>
              </select>
            </div>
            <div style="display:flex; gap:8px; align-items:center;">
              <span style="min-width:110px;">주입 최대 글자</span>
              <input type="number" min="0" max="100000" step="1000" value={cfg().max_inject_chars} onInput={(e) => set("max_inject_chars", parseInt(e.currentTarget.value) || 0)} style={`${inputStyle} width:110px;`} />
              <span class="sub" style="font-size:12px;">토큰 절감 — 주입 총량 상한</span>
            </div>
          </div>

          {/* 🟩 전송 전 — 필수 규칙 */}
          <div class="wsec">🟩 ↳ 필수 규칙 — 전송 전 반드시 주입 (게이트)</div>
          <textarea
            rows="3"
            placeholder="예: 답변 전 반드시 안전 규칙 확인. 코드 수정 전 영향범위 보고."
            value={cfg().mandatory_note}
            onInput={(e) => set("mandatory_note", e.currentTarget.value)}
            style={`${inputStyle} width:100%; resize:vertical;`}
          />

          {/* 이벤트: 도구 승인 (session/request_permission) */}
          <div class="wsec">🟨 도구 승인 (session/request_permission) — 허용/차단</div>
          <div class="sub" style="font-size:12.5px; line-height:1.6;">
            도구 권한 정책은 위 <b>세션 생성 → 기본 권한</b>으로 결정(어댑터 settings 적용).
            패턴별 allow/deny(예: <code>rm -rf</code> 차단)는 <code>session/request_permission</code>
            응답 라우팅이 필요 — 다음 단계 추가 예정.
          </div>

          {/* 이벤트: 턴 종료 (stopReason) */}
          <div class="wsec">🟪 턴 종료 (stopReason) — 무엇을 할지</div>
          <div class="sub" style="font-size:12.5px; line-height:1.6;">
            현재: 응답 영속 기록 + 읽음 처리 + 대기열 발신(자동).
            향후: 핵심 기억 자동 추출(L0→L2 승격, <code>claude_ingest_tick</code> 연계)·다음 트리거.
          </div>

          {/* 도메인 tick (자동·주기) */}
          <div class="wsec">⚙️ 도메인 tick (자동·주기) — 백그라운드 이벤트</div>
          <div class="sub" style="font-size:12.5px; line-height:1.6;">
            <code>claude_ingest_tick</code>(L0→L2 메모리 추출) · <code>patterns_mistakes_extract_tick</code>(L3 패턴) ·
            <code>m2_merge_candidates_tick</code>(메모리 병합) · <code>heartbeat_wake</code>(30분) ·
            <code>workflow_cron_tick</code>(워크플로우) · <code>self_trigger_fire_tick</code>(자가 트리거). 향후 주기·on/off 설정.
          </div>

          <div style="margin-top:16px; display:flex; gap:10px; align-items:center;">
            <button class="qbtn" disabled={busy()} onClick={save}>{busy() ? "저장 중…" : "설정 저장"}</button>
            <Show when={saved()}><span class="sub">{saved()}</span></Show>
          </div>

          {/* 관찰 */}
          <div class="wsec">🔎 관찰 — 지금 이 설정으로 주입될 것</div>
          <Show when={!ctx.loading} fallback={<div class="sub">불러오는 중…</div>}>
            <div style="border:1px solid var(--border); border-radius:8px; padding:12px; background:var(--bg-soft);">
              <div class="sub" style="margin-bottom:6px;">
                기억 {cfg().inject_memory ? injMems().length : 0}개 · 위키 {cfg().inject_wiki ? (ctx()?.wiki_count ?? 0) : 0}개 · 필수규칙 {cfg().mandatory_note.trim() ? "있음" : "없음"}
                {!cfg().inject_memory && " (기억 주입 꺼짐)"}
              </div>
              <Show when={cfg().inject_memory}>
                <For each={injMems()}>
                  {(m) => (
                    <div style="font-size:12.5px; padding:4px 0; border-top:1px solid var(--border);">
                      <b>[{KIND_LABELS[m.kind] ?? m.kind}]</b> {m.content.slice(0, 140)}
                    </div>
                  )}
                </For>
              </Show>
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
}
