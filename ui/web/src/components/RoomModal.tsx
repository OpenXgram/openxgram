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

// 스타일은 mockup.css(.oxg-ovl .mset …) 정본 목업 verbatim 포팅으로 대체됨.

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
// 정본 목업 ORCH flow 원형 숫자.
const CIRC = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"];

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

  // 정본 목업 toggle idiom — 켜짐(default tone) / .off(꺼짐).
  const togCls = (on: boolean) => (on ? "tog" : "tog off");

  return (
    <div class="oxg-ovl show" onClick={props.onClose}>
      <div class="modal" onClick={(e) => e.stopPropagation()}>
        <div class="mhead">
          <span class="ttl">⚙️ 방 설정 — {props.roomLabel || props.roomKey}</span>
          <span class="x" onClick={props.onClose}>✕</span>
        </div>

        {/* 정본 .mset — 방 설정 뷰(하네스/시스템프롬프트/역할정의/역할배정/순서/이벤트규칙) */}
        <div class="mset show">
          <div class="layhint" style="background:#fff7e6;border-color:#f0c674;color:#8a6d1a">
            ⓘ 이 설정은 <b>저장</b>되며 <b>적용</b>됩니다. 프롬프트 레이어링·턴 제어(P4a)·오케스트레이션 실행(P4c) <b>배선 완료</b>. 이벤트 트리거만 후속.
          </div>

          {/* 🧩 런타임 하네스 (방 오버라이드) */}
          <h5>🧩 런타임 하네스 <span style="font-weight:600;color:var(--muted);font-size:11px">— 편집 후 저장 시 이 방에 적용</span></h5>
          <div class="fld">
            <span class={togCls(harnessOn())} onClick={() => enableHarnessOverride(!harnessOn())}>
              {harnessOn() ? "방 오버라이드 켜짐" : "전역 상속 (꺼짐)"}
            </span>
          </div>
          <Show when={harnessOn()}>
            <div class="fld" style="margin-top:8px">
              <select class="sel" value={harness().runtime} onChange={(e) => setHarness("runtime", e.currentTarget.value)}>
                <For each={RUNTIMES}>{(r) => <option value={r} selected={r === harness().runtime}>런타임 · {r}</option>}</For>
              </select>
              <select class="sel" value={harness().model} onChange={(e) => setHarness("model", e.currentTarget.value)}>
                <For each={MODELS}>{(m) => <option value={m} selected={m === harness().model}>모델 · {m}</option>}</For>
              </select>
              <select class="sel" value={harness().perm_mode} onChange={(e) => setHarness("perm_mode", e.currentTarget.value)}>
                <For each={PERMS}>{(p) => <option value={p} selected={p === harness().perm_mode}>권한 · {p}</option>}</For>
              </select>
              <span class={togCls(harness().worktree)} onClick={() => setHarness("worktree", !harness().worktree)}>🌳 worktree</span>
              <span class={togCls(harness().isolation)} onClick={() => setHarness("isolation", !harness().isolation)}>🔒 격리</span>
              <select class="sel" value={harness().exec_mode} onChange={(e) => setHarness("exec_mode", e.currentTarget.value)}>
                <For each={EXECS}>{(x) => <option value={x} selected={x === harness().exec_mode}>실행 · {x}</option>}</For>
              </select>
              <select class="sel" value={harness().vault_scope} onChange={(e) => setHarness("vault_scope", e.currentTarget.value)}>
                <For each={VAULT_SCOPES}>{(v) => <option value={v} selected={v === harness().vault_scope}>🔐 vault · {v}</option>}</For>
              </select>
            </div>
            <div class="fld" style="margin-top:8px">
              <span style="font-size:11.5px;color:var(--muted)">MCP</span>
              <For each={MCP_OPTS}>{(m) => <span class={togCls((harness().mcp ?? []).includes(m))} onClick={() => toggleMcp(m)}>🔌 {m}</span>}</For>
            </div>
            <div class="fld" style="margin-top:8px">
              <span style="font-size:11.5px;color:var(--muted)">턴 모드</span>
              <span class={togCls(harness().turn_mode === "auto")} onClick={() => setHarness("turn_mode", "auto")}>auto (즉시 응답)</span>
              <span class={togCls(harness().turn_mode === "gated")} onClick={() => setHarness("turn_mode", "gated")}>gated (발언권 필요)</span>
            </div>
            <div class="fld" style="margin-top:8px">
              <span style="font-size:11.5px;color:var(--muted)">cwd</span>
              <input class="rname" style="flex:1;min-width:180px;margin-bottom:0" value={harness().cwd}
                placeholder="📁 /home/llm/projects/…" onInput={(e) => setHarness("cwd", e.currentTarget.value)} />
            </div>
          </Show>

          {/* 📝 방 시스템 프롬프트 */}
          <h5>📝 방 시스템 프롬프트 (이 방 전용)</h5>
          <textarea class="ta-edit" value={cfg().system_prompt}
            onInput={(e) => patch({ system_prompt: e.currentTarget.value })}
            placeholder="예: 이 방은 환불 정책 논의방. 결정 전 반드시 근거를 제시할 것." />

          {/* 🎭 역할 정의 (인라인 아코디언) */}
          <h5>🎭 역할 정의 (이름 + 지침) <span style="font-weight:600;color:var(--muted);font-size:11px">— 클릭하면 역할명·지침 편집</span></h5>
          <div class="layhint">📚 <b>지침 레이어링</b>: 방 시스템 프롬프트(방 전체) + <b>역할 지침</b>(배정 역할별) + 에이전트 base → 방 내 행동 = 셋의 합.</div>
          <For each={cfg().roles.defs}>{(d, i) => (
            <div class="roleitem" classList={{ open: editRole() === i() }}>
              <div class="defrole" onClick={() => (editRole() === i() ? setEditRole(null) : openRoleEd(i()))}>
                <span class="dn">🎭 {d.name}</span>
                <span class="dd">— {roleInst(d)}</span>
                <span class="edt">편집</span>
              </div>
              <div class="roleed">
                <label>역할명</label>
                <input class="rname" value={reName()} onInput={(e) => setReName(e.currentTarget.value)} />
                <label>지침 (role-level 시스템 프롬프트)</label>
                <textarea class="rinst" value={reInst()} onInput={(e) => setReInst(e.currentTarget.value)} />
                <div class="rerow">
                  <button class="rsave" onClick={saveRoleDef}>💾 역할 저장</button>
                  <button class="rcancel" onClick={() => setEditRole(null)}>취소</button>
                  <button class="rcancel" onClick={() => delRoleDef(i())}>🗑 삭제</button>
                </div>
              </div>
            </div>
          )}</For>
          <div class="roleed standalone" classList={{ show: editRole() === -1 }}>
            <label>역할명</label>
            <input class="rname" value={reName()} onInput={(e) => setReName(e.currentTarget.value)} placeholder="예: 검증자" />
            <label>지침 (role-level 시스템 프롬프트)</label>
            <textarea class="rinst" value={reInst()} onInput={(e) => setReInst(e.currentTarget.value)} placeholder="예: 테스트만 돌리고 통과/실패 보고. 코드 수정 금지." />
            <div class="rerow">
              <button class="rsave" onClick={saveRoleDef}>💾 역할 저장</button>
              <button class="rcancel" onClick={() => setEditRole(null)}>취소</button>
            </div>
          </div>
          <button class="addrole" onClick={() => openRoleEd(-1)}>＋ 새 역할 정의 (커스텀: 이름 + 지침)</button>

          {/* 👤 역할 배정 (역할 → 에이전트) */}
          <h5>👤 역할 배정 (역할 → 어떤 에이전트가 맡는지)</h5>
          <div style="font-size:11px;color:var(--muted);margin:0 0 6px">각 행 = <b>역할 → 담당 에이전트</b>. 배정한 역할의 <b>지침</b>이 그 에이전트에게 함께 적용됩니다.</div>
          <For each={cfg().roles.assignments}>{(a, i) => (
            <div class="role">
              <input class="rname" style="width:auto;flex:1;min-width:110px;margin-bottom:0" value={a.role} placeholder="역할명"
                onInput={(e) => setAssign(i(), "role", e.currentTarget.value)} />
              <span class="arr" style="color:#5a7fb0;font-weight:700">→</span>
              <select class="rsel" value={a.agent} onChange={(e) => setAssign(i(), "agent", e.currentTarget.value)}>
                <For each={agentNames()}>{(n) => <option value={n} selected={n === a.agent}>{n}</option>}</For>
              </select>
              <button class="rb" style="border:0;cursor:pointer" onClick={() => delAssign(i())}>🗑</button>
            </div>
          )}</For>
          <button class="addrole" onClick={addAssign}>＋ 역할 배정 — 역할 → 에이전트 지정</button>

          {/* 🔢 순서 (오케스트레이션) */}
          <h5>🔢 순서 (오케스트레이션) <span style="font-weight:600;color:var(--muted);font-size:11px">— 단계 추가·재정렬·삭제 가능</span></h5>
          <div class="orchflow fld">
            <Show when={cfg().orchestration.length > 0} fallback={<span style="color:var(--muted);font-size:11px">단계 없음 — ＋ 단계 추가</span>}>
              <For each={cfg().orchestration}>{(s, i) => (
                <>
                  <span class="pill">{CIRC[i()] || i() + 1} {s.label}{s.agent && s.agent !== "— 미지정" ? ` ${s.agent}` : ""}</span>
                  <Show when={i() < cfg().orchestration.length - 1}><span class="arr" style="color:#5a7fb0;font-weight:700">→</span></Show>
                </>
              )}</For>
            </Show>
          </div>
          <For each={cfg().orchestration}>{(s, i) => (
            <div class="orchstep">
              <span class="num">{CIRC[i()] || i() + 1}</span>
              <input class="slbl" value={s.label} onInput={(e) => setStep(i(), "label", e.currentTarget.value)} />
              <select class="ssel" value={s.agent} onChange={(e) => setStep(i(), "agent", e.currentTarget.value)}>
                <For each={agentNames()}>{(n) => <option value={n} selected={n === s.agent}>{n}</option>}</For>
              </select>
              <input class="ssel" style="width:80px" value={s.role} onInput={(e) => setStep(i(), "role", e.currentTarget.value)} />
              <span class="sp" />
              <button class="ob" title="위로" onClick={() => moveStep(i(), -1)}>▲</button>
              <button class="ob" title="아래로" onClick={() => moveStep(i(), 1)}>▼</button>
              <button class="ob del" title="삭제" onClick={() => delStep(i())}>🗑</button>
            </div>
          )}</For>
          <button class="addrole" onClick={addStep}>＋ 단계 추가</button>

          {/* 🔀 워크플로우 / 이벤트 규칙 */}
          <h5>🔀 워크플로우 / 이벤트 규칙 (이 방 적용)</h5>
          <For each={cfg().event_rules}>{(r, i) => (
            <div class="ev" style="display:flex;gap:7px;align-items:center">
              <input class="slbl" style="width:auto;flex:1" value={r.trigger} placeholder="트리거 (예: @mention, idle)"
                onInput={(e) => setEvent(i(), "trigger", e.currentTarget.value)} />
              <span class="arr">→</span>
              <input class="slbl" style="width:auto;flex:1" value={r.action} placeholder="액션 (예: grant-turn, summarize)"
                onInput={(e) => setEvent(i(), "action", e.currentTarget.value)} />
              <button class="ob del" onClick={() => delEvent(i())}>🗑</button>
            </div>
          )}</For>
          <button class="addrole" onClick={addEvent}>＋ 이벤트 규칙 추가</button>
          <div style="font-size:11px;color:var(--muted);margin-top:8px">OpenXgram 워크플로우 엔진(cron/메시지/이벤트) 연결</div>

          <Show when={err()}><div class="errln">{err()}</div></Show>
          <div class="savedhint" classList={{ show: !!saved() }}>✓ {saved()}</div>

          <div class="msetfoot">
            <button class="save" disabled={busy()} onClick={save}>{busy() ? "저장 중…" : "💾 저장"}</button>
            <button class="cancel" onClick={props.onClose}>취소</button>
          </div>
        </div>
      </div>
    </div>
  );
}
