import { createSignal, createResource, For, Show, onMount } from "solid-js";
import { invoke } from "@/api/client";

// OpenXgram 방(대화) 설정 모달 — GUI Phase P3.
// 대화 모델 spec 항목 11(하네스/방설정): 방마다 하네스·역할·오케스트레이션·시스템
// 프롬프트·이벤트 규칙을 개별 설정. 목업 openRoomModal/toggleSet/ROLE_DEFS/ORCH_STEPS 이식.
//
// ⚠️ P3 = 저장(persistence)만. 턴 시점 강제 적용(prompt 레이어링·orch 실행·이벤트
//    트리거)은 P4. UI 에 그 사실을 honest 배너로 명시한다.
//
// 전역 기본 하네스(⚙️)는 RuntimeTab + /v1/gui/runtime/config (alias 미지정) 재사용.
// 이 모달의 하네스 섹션은 그 위에 얹는 방-스코프 오버라이드. harness=null 이면 전역 상속.

// ── 스타일 (RuntimeTab.tsx 와 동일 토큰 재사용: --kk-line/--kk-ink/--kk-sub) ──
const overlay = "position:fixed; inset:0; background:rgba(0,0,0,0.5); z-index:70; display:flex; align-items:center; justify-content:center;";
const card = "background:var(--kk-card,#fff); color:var(--kk-ink); width:min(560px,94vw); max-height:90vh; overflow:auto; border-radius:14px; padding:0; box-shadow:0 10px 40px rgba(0,0,0,0.3);";
const head = "display:flex; align-items:center; gap:8px; padding:14px 18px; border-bottom:1px solid var(--kk-line); position:sticky; top:0; background:var(--kk-card,#fff); z-index:1;";
const body = "padding:14px 18px; display:flex; flex-direction:column; gap:18px;";
const sect = "border:1px solid var(--kk-line); border-radius:10px; padding:13px 15px;";
const sh = "font-weight:700; font-size:14px; margin:0 0 10px; color:var(--kk-ink);";
const ctl = "padding:6px 8px; background:#f7f8fa; color:var(--kk-ink); border:1px solid var(--kk-line); border-radius:6px; font-size:13px;";
const row = "display:flex; gap:10px; align-items:center; flex-wrap:wrap; margin-top:8px;";
const lbl = "min-width:96px; color:var(--kk-ink); font-size:13px;";
const hint = "color:var(--kk-sub); font-size:11.5px;";
const tog = "font-size:12px; padding:4px 11px; border:1px solid var(--kk-line); border-radius:14px; background:#f1f3f6; color:#54607a; cursor:pointer;";
const togOn = "font-size:12px; padding:4px 11px; border:1px solid #3a6ff0; border-radius:14px; background:#e7efff; color:#2452c8; cursor:pointer; font-weight:600;";
const btn = "font-size:12.5px; padding:5px 12px; border:1px solid var(--kk-line); border-radius:8px; background:#f7f8fa; color:var(--kk-ink); cursor:pointer;";
const btnPri = "font-size:13px; padding:7px 16px; border:1px solid #2452c8; border-radius:8px; background:#3a6ff0; color:#fff; cursor:pointer; font-weight:600;";
const ta = "width:100%; min-height:64px; padding:8px; background:#f7f8fa; color:var(--kk-ink); border:1px solid var(--kk-line); border-radius:8px; font-size:13px; resize:vertical; box-sizing:border-box; font-family:inherit;";

// ── 타입 ──
type Harness = {
  runtime: string; model: string; perm_mode: string; exec_mode: string;
  cwd: string; worktree: boolean; isolation: boolean; mcp: string[]; vault_scope: string;
  // P4a — 턴 모드. "auto"=inbound 즉시 턴(기본, 1:1 무회귀) / "gated"=맥락 누적만, 발언권/@호명/조건으로만 턴.
  turn_mode: string;
};
// 역할 정의: 정본 영속 필드명은 `instructions` (P4 레이어링 consumer 가 읽을 의미명).
// 레거시 데이터는 `inst` 로 저장돼 있을 수 있어 read 시 둘 다 수용(instructions ?? inst), write 는 instructions.
type RoleDef = { name: string; instructions?: string; inst?: string };
type RoleAssign = { role: string; agent: string };
type OrchStep = { label: string; agent: string; role: string };
type EventRule = { trigger: string; action: string };
type RoomConfig = {
  harness: Harness | null;
  roles: { defs: RoleDef[]; assignments: RoleAssign[] };
  orchestration: OrchStep[];
  system_prompt: string;
  event_rules: EventRule[];
};

const HARNESS_DEF: Harness = {
  runtime: "claude-code", model: "default", perm_mode: "bypassPermissions",
  exec_mode: "interactive", cwd: "", worktree: false, isolation: false, mcp: [], vault_scope: "none",
  turn_mode: "auto",
};
const EMPTY: RoomConfig = {
  harness: null, roles: { defs: [], assignments: [] }, orchestration: [], system_prompt: "", event_rules: [],
};

// 역할 지침 read 헬퍼: 정본 instructions 우선, 레거시 inst 폴백.
function roleInst(d: RoleDef | undefined): string {
  return (d?.instructions ?? d?.inst ?? "") as string;
}

// 서버 GET 응답 → 안전한 RoomConfig 로 정규화.
// - 누락 섹션 기본값 보강 (EMPTY 와 병합)
// - harness 가 객체면 HARNESS_DEF 와 병합해 모든 키 보장 (mcp 누락 → .includes 크래시 방지)
// - 역할 defs 의 instructions/inst 표준화 (instructions 로 통일)
function normalizeConfig(raw: unknown): RoomConfig {
  const c = { ...EMPTY, ...(raw && typeof raw === "object" ? (raw as Partial<RoomConfig>) : {}) };
  // harness: null 이면 전역 상속, 객체면 모든 키 보장
  if (c.harness && typeof c.harness === "object") {
    const h = c.harness as Partial<Harness>;
    c.harness = {
      ...HARNESS_DEF,
      ...h,
      mcp: Array.isArray(h.mcp) ? h.mcp : [],
    };
  } else {
    c.harness = null;
  }
  // roles
  const roles = (c.roles && typeof c.roles === "object" ? c.roles : {}) as Partial<RoomConfig["roles"]>;
  c.roles = {
    defs: Array.isArray(roles.defs)
      ? roles.defs.map((d) => ({ name: (d?.name ?? "") as string, instructions: roleInst(d as RoleDef) }))
      : [],
    assignments: Array.isArray(roles.assignments) ? roles.assignments : [],
  };
  c.orchestration = Array.isArray(c.orchestration) ? c.orchestration : [];
  c.event_rules = Array.isArray(c.event_rules) ? c.event_rules : [];
  c.system_prompt = typeof c.system_prompt === "string" ? c.system_prompt : "";
  return c;
}

// 토글 그룹 옵션 (목업 toggleSet idiom)
const RUNTIMES = ["claude-code", "codex", "gemini-cli", "cursor", "aider"];
const MODELS = ["default", "opus", "sonnet", "haiku"];
const PERMS = ["bypassPermissions", "acceptEdits", "plan", "default"];
const EXECS = ["interactive", "headless"];
const VAULT_SCOPES = ["none", "read", "read-write"];
const MCP_OPTS = ["openxgram", "context-mode", "token-savior", "playwright", "github"];

export function RoomModal(props: { roomKey: string; roomLabel?: string; onClose: () => void }) {
  const [cfg, setCfg] = createSignal<RoomConfig>(EMPTY);
  const [busy, setBusy] = createSignal(false);
  const [saved, setSaved] = createSignal<string | null>(null);
  const [err, setErr] = createSignal<string | null>(null);
  // 역할 인라인 아코디언 편집 인덱스(-1 = 새 역할 추가 폼 열림, null = 닫힘)
  const [editRole, setEditRole] = createSignal<number | null>(null);
  const [reName, setReName] = createSignal("");
  const [reInst, setReInst] = createSignal("");

  // 배정 가능 에이전트 목록 (역할→에이전트 select 용). 동적 — agents_list 재사용.
  const [agents] = createResource<any[]>(() => invoke("agents_list").catch(() => []));
  const agentNames = () => {
    const a = agents() || [];
    return ["— 미지정", ...a.map((x: any) => x.alias || x.name).filter(Boolean), "⭐ 나"];
  };

  onMount(async () => {
    try {
      const r = await invoke<{ config: RoomConfig }>("room_config_get", { key: props.roomKey });
      if (r?.config) setCfg(normalizeConfig(r.config));
    } catch (e: any) {
      setErr(`로드 실패: ${e?.message || e}`);
    }
  });

  function touch() { setSaved(null); }
  function patch(p: Partial<RoomConfig>) { setCfg({ ...cfg(), ...p }); touch(); }

  // ── 하네스 ──
  const harness = (): Harness => cfg().harness ?? HARNESS_DEF;
  const harnessOn = () => cfg().harness != null;
  function setHarness<K extends keyof Harness>(k: K, v: Harness[K]) {
    patch({ harness: { ...harness(), [k]: v } });
  }
  function toggleMcp(m: string) {
    const list = Array.isArray(harness().mcp) ? harness().mcp : [];
    setHarness("mcp", list.includes(m) ? list.filter((x) => x !== m) : [...list, m]);
  }
  function enableHarnessOverride(on: boolean) {
    patch({ harness: on ? { ...HARNESS_DEF } : null });
  }

  // ── 역할 정의 (인라인 아코디언: 이름 + 지침) ──
  function openRoleEd(i: number) {
    setEditRole(i);
    if (i === -1) { setReName(""); setReInst(""); }
    else { const d = cfg().roles.defs[i]; setReName(d?.name || ""); setReInst(roleInst(d)); }
  }
  function saveRoleDef() {
    const nm = reName().trim(); if (!nm) return;
    const defs = [...cfg().roles.defs];
    const i = editRole();
    if (i === -1 || i == null) defs.push({ name: nm, instructions: reInst() });
    else defs[i] = { name: nm, instructions: reInst() };
    patch({ roles: { ...cfg().roles, defs } });
    setEditRole(null);
  }
  function delRoleDef(i: number) {
    const defs = cfg().roles.defs.filter((_, j) => j !== i);
    patch({ roles: { ...cfg().roles, defs } });
    if (editRole() === i) setEditRole(null);
  }

  // ── 역할 배정 (역할 → 에이전트) ──
  function addAssign() {
    patch({ roles: { ...cfg().roles, assignments: [...cfg().roles.assignments, { role: "", agent: "— 미지정" }] } });
  }
  function setAssign(i: number, k: keyof RoleAssign, v: string) {
    const assignments = [...cfg().roles.assignments];
    assignments[i] = { ...assignments[i], [k]: v };
    patch({ roles: { ...cfg().roles, assignments } });
  }
  function delAssign(i: number) {
    patch({ roles: { ...cfg().roles, assignments: cfg().roles.assignments.filter((_, j) => j !== i) } });
  }

  // ── 오케스트레이션 단계 (순서 보존) ──
  function addStep() { patch({ orchestration: [...cfg().orchestration, { label: "새 단계", agent: "— 미지정", role: "작업" }] }); }
  function setStep(i: number, k: keyof OrchStep, v: string) {
    const o = [...cfg().orchestration]; o[i] = { ...o[i], [k]: v }; patch({ orchestration: o });
  }
  function moveStep(i: number, dir: number) {
    const j = i + dir; const o = [...cfg().orchestration];
    if (j < 0 || j >= o.length) return;
    [o[i], o[j]] = [o[j], o[i]]; patch({ orchestration: o });
  }
  function delStep(i: number) { patch({ orchestration: cfg().orchestration.filter((_, j) => j !== i) }); }

  // ── 이벤트 규칙 ──
  function addEvent() { patch({ event_rules: [...cfg().event_rules, { trigger: "", action: "" }] }); }
  function setEvent(i: number, k: keyof EventRule, v: string) {
    const e = [...cfg().event_rules]; e[i] = { ...e[i], [k]: v }; patch({ event_rules: e });
  }
  function delEvent(i: number) { patch({ event_rules: cfg().event_rules.filter((_, j) => j !== i) }); }

  // ── 저장 ──
  async function save() {
    setBusy(true); setErr(null);
    try {
      const c = cfg();
      await invoke("room_config_set", {
        key: props.roomKey,
        harness: c.harness, roles: c.roles, orchestration: c.orchestration,
        system_prompt: c.system_prompt, event_rules: c.event_rules,
      });
      setSaved("저장됨 ✓ (설정 보존 — 턴 시점 적용은 P4)");
    } catch (e: any) {
      setErr(`저장 실패: ${e?.message || e}`);
    } finally { setBusy(false); }
  }

  return (
    <div style={overlay} onClick={props.onClose}>
      <div style={card} onClick={(e) => e.stopPropagation()}>
        <div style={head}>
          <span style="font-size:18px">⚙️</span>
          <b style="font-size:15px">방 설정 — {props.roomLabel || props.roomKey}</b>
          <button style={`${btn}; margin-left:auto`} onClick={props.onClose}>✕</button>
        </div>

        {/* 정직성 배너 — P4a/P4c 배선 완료, 이벤트 트리거만 후속 */}
        <div style="margin:12px 18px 0; padding:8px 12px; background:#fff7e6; border:1px solid #f0c674; border-radius:8px; font-size:12px; color:#8a6d1a;">
          ⓘ 이 설정은 <b>저장</b>되며 <b>적용</b>됩니다. 프롬프트 레이어링·턴 제어(P4a)와 오케스트레이션 실행(P4c)은 <b>배선·강제 완료</b>. 이벤트 트리거(이벤트 규칙)만 후속 단계로 남아 있습니다.
        </div>

        <div style={body}>
          {/* ── 하네스 ── */}
          <div style={sect}>
            <h4 style={sh}>🧠 하네스 (방 오버라이드)</h4>
            <div style={hint}>전역 기본 하네스는 설정 ▸ 런타임 탭(⚙️)에서. 이 방만 다르게 하려면 오버라이드를 켜세요. 끄면 전역 상속.</div>
            <div style={row}>
              <button style={harnessOn() ? togOn : tog} onClick={() => enableHarnessOverride(!harnessOn())}>
                {harnessOn() ? "방 오버라이드 켜짐" : "전역 상속 (오버라이드 꺼짐)"}
              </button>
            </div>
            <Show when={harnessOn()}>
              <div style={row}>
                <span style={lbl}>런타임</span>
                <For each={RUNTIMES}>{(r) => (
                  <button style={harness().runtime === r ? togOn : tog} onClick={() => setHarness("runtime", r)}>{r}</button>
                )}</For>
              </div>
              <div style={row}>
                <span style={lbl}>모델</span>
                <For each={MODELS}>{(m) => (
                  <button style={harness().model === m ? togOn : tog} onClick={() => setHarness("model", m)}>{m}</button>
                )}</For>
              </div>
              <div style={row}>
                <span style={lbl}>권한 모드</span>
                <For each={PERMS}>{(p) => (
                  <button style={harness().perm_mode === p ? togOn : tog} onClick={() => setHarness("perm_mode", p)}>{p}</button>
                )}</For>
              </div>
              <div style={row}>
                <span style={lbl}>실행 모드</span>
                <For each={EXECS}>{(x) => (
                  <button style={harness().exec_mode === x ? togOn : tog} onClick={() => setHarness("exec_mode", x)}>{x}</button>
                )}</For>
              </div>
              <div style={row}>
                <span style={lbl}>작업 디렉토리</span>
                <input style={`${ctl}; flex:1; min-width:180px`} value={harness().cwd}
                  placeholder="예: /home/llm/projects/…" onInput={(e) => setHarness("cwd", e.currentTarget.value)} />
              </div>
              <div style={row}>
                <button style={harness().worktree ? togOn : tog} onClick={() => setHarness("worktree", !harness().worktree)}>
                  worktree {harness().worktree ? "ON" : "OFF"}
                </button>
                <button style={harness().isolation ? togOn : tog} onClick={() => setHarness("isolation", !harness().isolation)}>
                  격리(isolation) {harness().isolation ? "ON" : "OFF"}
                </button>
              </div>
              <div style={row}>
                <span style={lbl}>MCP</span>
                <For each={MCP_OPTS}>{(m) => (
                  <button style={(harness().mcp ?? []).includes(m) ? togOn : tog} onClick={() => toggleMcp(m)}>{m}</button>
                )}</For>
              </div>
              <div style={row}>
                <span style={lbl}>vault 범위</span>
                <For each={VAULT_SCOPES}>{(v) => (
                  <button style={harness().vault_scope === v ? togOn : tog} onClick={() => setHarness("vault_scope", v)}>{v}</button>
                )}</For>
              </div>
              {/* P4a — 턴 모드. auto=받으면 즉시 응답(기본) / gated=맥락만 누적, 발언권/@/조건으로만 발언(관찰자). */}
              <div style={row}>
                <span style={lbl}>턴 모드</span>
                <button style={harness().turn_mode === "auto" ? togOn : tog} onClick={() => setHarness("turn_mode", "auto")}>auto (즉시 응답)</button>
                <button style={harness().turn_mode === "gated" ? togOn : tog} onClick={() => setHarness("turn_mode", "gated")}>gated (관찰자 · 발언권 필요)</button>
              </div>
            </Show>
          </div>

          {/* ── 역할 정의 (인라인 아코디언) ── */}
          <div style={sect}>
            <h4 style={sh}>🎭 역할 정의 (이름 + 지침)</h4>
            <div style={hint}>📚 지침 레이어링: 방 시스템 프롬프트 + 역할 지침 + 에이전트 base = 방 내 행동.</div>
            <For each={cfg().roles.defs}>{(d, i) => (
              <div style="margin-top:8px; border:1px solid var(--kk-line); border-radius:8px; overflow:hidden;">
                <div style="display:flex; align-items:center; gap:8px; padding:8px 10px; background:#f7f8fa; cursor:pointer;"
                  onClick={() => (editRole() === i() ? setEditRole(null) : openRoleEd(i()))}>
                  <b style="font-size:13px">{d.name}</b>
                  <span style={hint}>{editRole() === i() ? "▾" : "▸"} 클릭하면 편집</span>
                  <button style={`${btn}; margin-left:auto; padding:2px 8px`} onClick={(e) => { e.stopPropagation(); delRoleDef(i()); }}>🗑</button>
                </div>
                <Show when={editRole() === i()}>
                  <div style="padding:10px; display:flex; flex-direction:column; gap:6px;">
                    <input style={ctl} value={reName()} onInput={(e) => setReName(e.currentTarget.value)} placeholder="역할명" />
                    <textarea style={ta} value={reInst()} onInput={(e) => setReInst(e.currentTarget.value)}
                      placeholder="지침 (role-level 시스템 프롬프트). 예: 테스트만 돌리고 통과/실패 보고. 코드 수정 금지." />
                    <div style="display:flex; gap:8px;">
                      <button style={btnPri} onClick={saveRoleDef}>💾 역할 저장</button>
                      <button style={btn} onClick={() => setEditRole(null)}>취소</button>
                    </div>
                  </div>
                </Show>
              </div>
            )}</For>
            <Show when={editRole() === -1}>
              <div style="margin-top:8px; padding:10px; border:1px dashed var(--kk-line); border-radius:8px; display:flex; flex-direction:column; gap:6px;">
                <input style={ctl} value={reName()} onInput={(e) => setReName(e.currentTarget.value)} placeholder="예: 검증자" />
                <textarea style={ta} value={reInst()} onInput={(e) => setReInst(e.currentTarget.value)} placeholder="지침 (role-level 시스템 프롬프트)" />
                <div style="display:flex; gap:8px;">
                  <button style={btnPri} onClick={saveRoleDef}>💾 역할 저장</button>
                  <button style={btn} onClick={() => setEditRole(null)}>취소</button>
                </div>
              </div>
            </Show>
            <div style={row}>
              <button style={btn} onClick={() => openRoleEd(-1)}>＋ 새 역할 정의 (커스텀: 이름 + 지침)</button>
            </div>

            <h4 style={`${sh}; margin-top:14px`}>👤 역할 배정 (역할 → 에이전트)</h4>
            <div style={hint}>각 행 = 역할 → 담당 에이전트. 배정한 역할의 지침이 그 에이전트에게 함께 적용(P4).</div>
            <For each={cfg().roles.assignments}>{(a, i) => (
              <div style={row}>
                <input style={`${ctl}; flex:1; min-width:120px`} value={a.role} placeholder="역할명"
                  onInput={(e) => setAssign(i(), "role", e.currentTarget.value)} />
                <span>→</span>
                <select style={`${ctl}; min-width:130px`} value={a.agent} onChange={(e) => setAssign(i(), "agent", e.currentTarget.value)}>
                  <For each={agentNames()}>{(n) => <option value={n} selected={n === a.agent}>{n}</option>}</For>
                </select>
                <button style={`${btn}; padding:2px 8px`} onClick={() => delAssign(i())}>🗑</button>
              </div>
            )}</For>
            <div style={row}>
              <button style={btn} onClick={addAssign}>＋ 역할 배정 추가</button>
            </div>
          </div>

          {/* ── 오케스트레이션 단계 ── */}
          <div style={sect}>
            <h4 style={sh}>🔀 오케스트레이션 단계 (순서)</h4>
            <div style={hint}>순서대로 실행될 단계 — 라벨 + 담당 에이전트 + 역할/액션. (실행 엔진은 P4)</div>
            <For each={cfg().orchestration}>{(s, i) => (
              <div style={`${row}; align-items:flex-start`}>
                <span style="min-width:18px; color:var(--kk-sub); font-size:13px; margin-top:6px;">{i() + 1}.</span>
                <input style={`${ctl}; flex:1; min-width:100px`} value={s.label} placeholder="단계 라벨"
                  onInput={(e) => setStep(i(), "label", e.currentTarget.value)} />
                <select style={`${ctl}; min-width:120px`} value={s.agent} onChange={(e) => setStep(i(), "agent", e.currentTarget.value)}>
                  <For each={agentNames()}>{(n) => <option value={n} selected={n === s.agent}>{n}</option>}</For>
                </select>
                <input style={`${ctl}; width:90px`} value={s.role} placeholder="역할/액션"
                  onInput={(e) => setStep(i(), "role", e.currentTarget.value)} />
                <button style={`${btn}; padding:2px 7px`} title="위로" onClick={() => moveStep(i(), -1)}>▲</button>
                <button style={`${btn}; padding:2px 7px`} title="아래로" onClick={() => moveStep(i(), 1)}>▼</button>
                <button style={`${btn}; padding:2px 7px`} title="삭제" onClick={() => delStep(i())}>🗑</button>
              </div>
            )}</For>
            <div style={row}>
              <button style={btn} onClick={addStep}>＋ 단계 추가</button>
            </div>
          </div>

          {/* ── 방 시스템 프롬프트 ── */}
          <div style={sect}>
            <h4 style={sh}>📝 방 시스템 프롬프트</h4>
            <div style={hint}>이 방의 모든 참가자에게 공통으로 얹히는 지침(레이어링 최상위).</div>
            <textarea style={`${ta}; min-height:80px; margin-top:8px`} value={cfg().system_prompt}
              onInput={(e) => patch({ system_prompt: e.currentTarget.value })}
              placeholder="예: 이 방은 환불 정책 논의방. 결정 전 반드시 근거를 제시할 것." />
          </div>

          {/* ── 이벤트 규칙 ── */}
          <div style={sect}>
            <h4 style={sh}>⚡ 이벤트 규칙</h4>
            <div style={hint}>트리거 → 액션 규칙. (트리거 엔진은 P4)</div>
            <For each={cfg().event_rules}>{(r, i) => (
              <div style={row}>
                <input style={`${ctl}; flex:1; min-width:120px`} value={r.trigger} placeholder="트리거 (예: @mention, idle, conclusion)"
                  onInput={(e) => setEvent(i(), "trigger", e.currentTarget.value)} />
                <span>→</span>
                <input style={`${ctl}; flex:1; min-width:120px`} value={r.action} placeholder="액션 (예: grant-turn, summarize)"
                  onInput={(e) => setEvent(i(), "action", e.currentTarget.value)} />
                <button style={`${btn}; padding:2px 8px`} onClick={() => delEvent(i())}>🗑</button>
              </div>
            )}</For>
            <div style={row}>
              <button style={btn} onClick={addEvent}>＋ 이벤트 규칙 추가</button>
            </div>
          </div>
        </div>

        {/* 푸터 — 저장 */}
        <div style="display:flex; align-items:center; gap:12px; padding:14px 18px; border-top:1px solid var(--kk-line); position:sticky; bottom:0; background:var(--kk-card,#fff);">
          <Show when={saved()}><span style="color:#1a8a3a; font-size:12.5px;">{saved()}</span></Show>
          <Show when={err()}><span style="color:#c0392b; font-size:12.5px;">{err()}</span></Show>
          <button style={`${btnPri}; margin-left:auto`} disabled={busy()} onClick={save}>{busy() ? "저장 중…" : "💾 방 설정 저장"}</button>
          <button style={btn} onClick={props.onClose}>닫기</button>
        </div>
      </div>
    </div>
  );
}
