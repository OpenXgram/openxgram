import { createResource, createSignal, createMemo, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./flow-extra.css";

// 흐름 탭 — 카카오톡 정본 목업(_mockups/kakao-mockup.html) 충실 이식.
// 정본 #wfOvl(L634-731) 보드 + 빌더(L640-686) + #orgOvl(L802-816) 조직도 + #runOvl(L759-799) 실행이력.
// builder/org/run 마크업·CSS 는 flow-extra.css 로 verbatim 포팅(네임스페이스 .kk-flow). kakao.css 미수정.
//
// 백엔드 contract 재사용 (신규 명령 발명 X):
//   workflows_list        → Workflow[]   (워크플로우 보드)
//   workflow_upsert       → { id }        (빌더 만들기 — name + yaml_body 필수)
//   workflow_run          → { run_id }    (▶ 실행)
//   workflow_runs         → Run[]         (실행이력 #runOvl)
//   workflow_delete       → ()            (삭제)
//   schedule_list/_stats/_cancel          (스케줄/cron 세그먼트)
//   orchestration_agents  → OrgAgent[]    (조직도 #orgOvl — reports_to 계층)
// daemon no-fallback 규칙: 로딩/에러/빈 상태를 모두 명시적으로 표시. 가짜 데이터 X.
//
// 🪄 자동구성(autoplan): 백엔드 auto-plan 라우트가 없으므로 "준비 중" affordance 로만 노출.
//   생성된 가짜 플랜을 보여주지 않는다(정직).

interface Workflow {
  id: string;
  name: string;
  description?: string | null;
  enabled?: boolean | null;
  orchestrator?: string | null;
  cron_expr?: string | null;
  cost_limit?: number | null;
}

interface Schedule {
  id: string;
  target_kind: string;
  target: string;
  payload: string;
  msg_type: string;
  schedule_kind: string;
  schedule_value: string;
  status: string;
  created_at_kst: number;
  next_due_at_kst: number | null;
  last_error: string | null;
}

interface Stats {
  pending: number;
  sent: number;
  failed: number;
  cancelled: number;
}

interface OrgAgent {
  alias: string;
  role?: string | null;
  description?: string | null;
  orchestration_role?: string | null;
  reports_to?: string | null;
  status?: string | null;
}

interface WfRun {
  id: string;
  started_at: string;
  finished_at?: string | null;
  status: string;
  current_step?: string | null;
  total_cost?: number | null;
}

// Phase 3 — A2A(에이전트↔에이전트) DTO.
// a2a_agents → { agents, note }. 현재 OpenXgram 에이전트는 AgentCard 호스팅 전이라
// reachable:false + note(후속 안내)가 정직하게 내려온다 — UI 에서 숨기지 않는다.
interface A2AAgent {
  alias: string;
  reachable: boolean;
  agentCardUrl?: string | null;
}
interface A2AAgentsResp {
  agents: A2AAgent[];
  note?: string | null;
}
interface A2ASendResp {
  taskId: string;
  skill?: string | null;
  fromAgent?: string | null;
  target: string;
  task?: string | null;
}
interface A2ATaskResp {
  taskId: string;
  task: unknown;
}

type Seg = "workflows" | "schedules";

function fmtTs(ts?: number | null): string {
  if (!ts) return "";
  const ms = ts > 1e12 ? ts : ts * 1000;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(
    d.getMinutes(),
  ).padStart(2, "0")}`;
}

function fmtRunTs(s?: string | null): string {
  if (!s) return "";
  const d = new Date(s.includes("T") ? s : s.replace(" ", "T") + "Z");
  if (Number.isNaN(d.getTime())) return s;
  return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(
    d.getMinutes(),
  ).padStart(2, "0")}`;
}

// 목업 .step / .oava 색상 클래스 매핑 — description/role 의 LLM 종류 힌트로 추정(정직: 없으면 회색).
function colorClass(a: OrgAgent): string {
  const t = `${a.description ?? ""} ${a.role ?? ""}`.toLowerCase();
  if (t.includes("claude")) return "c-claude";
  if (t.includes("codex") || t.includes("gpt") || t.includes("openai")) return "c-codex";
  if (t.includes("gemini")) return "c-gemini";
  if (t.includes("ollama")) return "c-ollama";
  if (t.includes("hermes")) return "c-hermes";
  return "c-ollama";
}

export function FlowTab() {
  const [seg, setSeg] = createSignal<Seg>("workflows");

  const [workflows, { refetch: refetchWorkflows }] = createResource<Workflow[]>(() =>
    invoke<Workflow[]>("workflows_list"),
  );
  const [schedules, { refetch: refetchSchedules }] = createResource<Schedule[]>(() =>
    invoke<Schedule[]>("schedule_list"),
  );
  const [stats, { refetch: refetchStats }] = createResource<Stats>(() =>
    invoke<Stats>("schedule_stats"),
  );

  const [busyWf, setBusyWf] = createSignal<string | null>(null);
  const [wfNote, setWfNote] = createSignal<{ id: string; text: string; err: boolean } | null>(null);

  // 빌더 (만들기) 상태 — 목업 #builder.
  const [showBuilder, setShowBuilder] = createSignal(false);

  // 조직도 / 실행이력 / A2A 위임 오버레이 상태.
  const [orgOpen, setOrgOpen] = createSignal(false);
  const [a2aOpen, setA2aOpen] = createSignal(false);
  const [runFor, setRunFor] = createSignal<Workflow | null>(null);

  async function runWorkflow(id: string) {
    setBusyWf(id);
    setWfNote(null);
    try {
      const r = await invoke<{ run_id?: string }>("workflow_run", { id });
      setWfNote({ id, text: `실행 시작 · run ${r?.run_id ?? "?"}`, err: false });
      await refetchWorkflows();
    } catch (e) {
      setWfNote({ id, text: `실행 실패: ${(e as Error).message}`, err: true });
    } finally {
      setBusyWf(null);
    }
  }

  async function deleteWorkflow(id: string, name: string) {
    if (!confirm(`워크플로우 "${name}" 삭제?`)) return;
    setBusyWf(id);
    try {
      await invoke("workflow_delete", { id });
      await refetchWorkflows();
    } catch (e) {
      setWfNote({ id, text: `삭제 실패: ${(e as Error).message}`, err: true });
    } finally {
      setBusyWf(null);
    }
  }

  async function cancelSchedule(id: string) {
    try {
      await invoke("schedule_cancel", { id });
      await refetchSchedules();
      await refetchStats();
    } catch (e) {
      alert(`스케줄 취소 실패: ${(e as Error).message}`);
    }
  }

  return (
    <div class="kk-flow">
      <div class="board">
        <div class="bh">
          <h2>🔀 워크플로우</h2>
          <span class="sub">목표 + 보유 에이전트 단계 + 트리거(시간·webhook)</span>
          <div class="kk-seg kk-flow-seg">
            <div class={`s${seg() === "workflows" ? " on" : ""}`} onClick={() => setSeg("workflows")}>
              🔀 워크플로우
            </div>
            <div class={`s${seg() === "schedules" ? " on" : ""}`} onClick={() => setSeg("schedules")}>
              ⏰ 스케줄
            </div>
          </div>
        </div>

        <div class="bb">
          <Show when={seg() === "workflows"}>
            <Show
              when={showBuilder()}
              fallback={
                <button class="addwf" onClick={() => setShowBuilder(true)}>
                  ＋ 워크플로우 만들기 (프로젝트·목표·트리거·단계)
                </button>
              }
            >
              <Builder
                onClose={() => setShowBuilder(false)}
                onSaved={async () => {
                  setShowBuilder(false);
                  await refetchWorkflows();
                }}
              />
            </Show>

            <WorkflowsView
              workflows={workflows()}
              loading={workflows.loading}
              error={workflows.error}
              busy={busyWf()}
              note={wfNote()}
              onRun={runWorkflow}
              onDelete={deleteWorkflow}
              onOpenOrg={() => setOrgOpen(true)}
              onOpenA2a={() => setA2aOpen(true)}
              onOpenRun={(w) => setRunFor(w)}
            />
          </Show>

          <Show when={seg() === "schedules"}>
            <SchedulesView
              schedules={schedules()}
              loading={schedules.loading}
              error={schedules.error}
              stats={stats()}
              onCancel={cancelSchedule}
            />
          </Show>
        </div>
      </div>

      <Show when={orgOpen()}>
        <OrgOverlay onClose={() => setOrgOpen(false)} colorClass={colorClass} />
      </Show>
      <Show when={a2aOpen()}>
        <A2AOverlay onClose={() => setA2aOpen(false)} />
      </Show>
      <Show when={runFor()}>
        <RunOverlay workflow={runFor()!} onClose={() => setRunFor(null)} />
      </Show>
    </div>
  );
}

// ── 빌더 (만들기 → 목표/트리거/단계/채널) ── 목업 #builder ──
function Builder(props: { onClose: () => void; onSaved: () => void }) {
  const [name, setName] = createSignal("");
  const [goal, setGoal] = createSignal("");
  const [trigIdx, setTrigIdx] = createSignal(1); // 0=수동 1=cron 2=webhook (목업 기본 cron)
  const [cron, setCron] = createSignal("0 8 * * *");
  const [channel, setChannel] = createSignal("연결 안 함");
  const [steps, setSteps] = createSignal<string[]>([]);
  const [stepDraft, setStepDraft] = createSignal("");
  const [autoplan, setAutoplan] = createSignal(false);
  const [dragIdx, setDragIdx] = createSignal<number | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [note, setNote] = createSignal<{ text: string; err: boolean } | null>(null);
  // 보유 에이전트(로스터) — 목표에 투입할 후보. 클릭 시 단계로 추가(스펙: 등록 에이전트 선택).
  const [agents] = createResource<any[]>(() => invoke("agents_list"));
  const [planBusy, setPlanBusy] = createSignal(false);
  const [planNote, setPlanNote] = createSignal<string | null>(null);

  // ops(또는 orchestrator) 에이전트를 ACP 로 구동해 목표→단계 자동 생성 (LLM 플래너).
  async function runOpsPlan() {
    if (!goal().trim()) { setPlanNote("목표를 먼저 입력하세요."); return; }
    setPlanBusy(true); setPlanNote(null);
    try {
      const r = await invoke<any>("workflow_plan", { goal: goal().trim() });
      const ps = (r?.plan?.steps ?? []) as { agent?: string; action?: string }[];
      if (ps.length) {
        setSteps([...steps(), ...ps.map((s) => `${s.agent || "NEW"} · ${s.action || ""}`)]);
      }
      const hire = (r?.plan?.hire ?? []) as { role?: string }[];
      const hireTxt = hire.length ? ` · 고용 추천: ${hire.map((h) => h.role).join(", ")}` : "";
      setPlanNote(ps.length
        ? `🤖 ops가 ${ps.length}단계 제안${hireTxt}`
        : `ops 응답 파싱 실패 — 원문: ${(r?.raw || "").slice(0, 140)}`);
    } catch (e) {
      setPlanNote(`플래너 실패: ${(e as Error).message}`);
    } finally { setPlanBusy(false); }
  }

  function addStep() {
    const v = stepDraft().trim();
    if (!v) return;
    setSteps([...steps(), v]);
    setStepDraft("");
  }
  function removeStep(i: number) {
    setSteps(steps().filter((_, j) => j !== i));
  }
  function onDrop(target: number) {
    const from = dragIdx();
    if (from === null || from === target) return;
    const arr = [...steps()];
    const [m] = arr.splice(from, 1);
    arr.splice(target, 0, m);
    setSteps(arr);
    setDragIdx(null);
  }

  // 목업 단계 칩 → workflow_upsert 의 yaml_body 로 직렬화. 단계가 없으면 단일 noop 단계.
  function buildYaml(): string {
    const lines: string[] = [];
    lines.push(`name: ${name().trim()}`);
    if (goal().trim()) lines.push(`description: ${goal().trim()}`);
    lines.push("steps:");
    const list = steps().length ? steps() : ["단계 1"];
    list.forEach((s, i) => {
      const [agentPart, ...rest] = s.split("·");
      const agent = (agentPart || s).trim().replace(/\s+/g, "_") || `step_${i + 1}`;
      const action = rest.join("·").trim();
      lines.push(`  - id: step_${i + 1}`);
      lines.push(`    agent: ${agent}`);
      if (action) lines.push(`    action: ${action}`);
    });
    return lines.join("\n");
  }

  async function save() {
    if (!name().trim()) {
      setNote({ text: "이름(흐름 이름)을 입력하세요.", err: true });
      return;
    }
    setBusy(true);
    setNote(null);
    try {
      const cronExpr = trigIdx() === 1 ? cron().trim() || null : null;
      const desc =
        channel() !== "연결 안 함" ? `${goal().trim()} · 채널: ${channel()}` : goal().trim();
      await invoke("workflow_upsert", {
        name: name().trim(),
        yaml_body: buildYaml(),
        description: desc || null,
        cron_expr: cronExpr,
        cost_limit: null,
      });
      props.onSaved();
    } catch (e) {
      setNote({ text: `저장 실패: ${(e as Error).message}`, err: true });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="builder">
      <div class="bl">🏷 흐름 이름</div>
      <input
        class="ctl2"
        placeholder="예: 배포 후 SNS 공유"
        value={name()}
        onInput={(e) => setName(e.currentTarget.value)}
      />

      <div class="bl">🎯 목표 (이 흐름이 달성할 것)</div>
      <input
        class="ctl2"
        placeholder="예: 배포 결과를 정리해 SNS 게시하고 보고"
        value={goal()}
        onInput={(e) => setGoal(e.currentTarget.value)}
      />

      <button class="autoplan-btn" onClick={() => setAutoplan(!autoplan())}>
        🧩 보유 에이전트로 구성 — 클릭해서 단계 추가 {autoplan() ? "▲" : "▼"}
      </button>
      <Show when={autoplan()}>
        <div class="autoplan">
          <button
            class="autoplan-btn"
            style="background:#3a2f6a; margin-bottom:6px;"
            disabled={planBusy()}
            onClick={runOpsPlan}
          >
            {planBusy() ? "🤖 ops 플래너 분석 중… (수십 초 소요)" : "🤖 ops 플래너로 목표 분석 → 단계 자동 생성"}
          </button>
          <Show when={planNote()}>
            <div class="ap-head" style="color:#9ecbff;">{planNote()}</div>
          </Show>
          <div class="ap-head">
            목표에 투입할 <b>보유 에이전트</b>를 클릭하면 아래 단계로 추가됩니다. 더 필요하면
            <b> 에이전트 탭 → 템플릿으로 고용</b> 후 다시 선택하세요. (LLM 자동 분석은 ops 에이전트 연동 시 추가)
          </div>
          <div style="display:flex; flex-wrap:wrap; gap:6px; margin-top:8px;">
            <For each={agents() ?? []}>
              {(a) => (
                <button
                  type="button"
                  title={`${a.role || a.classification || ""} · @${a.alias}`}
                  style="background:#1f6f43; color:#fff; border:none; border-radius:7px; padding:5px 9px; font-size:12px; cursor:pointer;"
                  onClick={() => setSteps([...steps(), `${a.alias} · `])}
                >
                  ＋ {a.display_name || a.alias}
                </button>
              )}
            </For>
            <Show when={(agents() ?? []).length === 0}>
              <span class="miss">보유 에이전트 없음 — 에이전트 탭에서 먼저 생성/고용하세요.</span>
            </Show>
          </div>
          <div class="aprow" style="margin-top:8px;">
            <span class="have">보유 {(agents() ?? []).length}개</span> · 고용 필요 시 에이전트 탭 템플릿 사용
          </div>
        </div>
      </Show>

      <div class="bl">⏱ 트리거 — 언제 실행할까</div>
      <div class="trigseg">
        <div class={`ts${trigIdx() === 0 ? " on" : ""}`} onClick={() => setTrigIdx(0)}>
          수동 실행
        </div>
        <div class={`ts${trigIdx() === 1 ? " on" : ""}`} onClick={() => setTrigIdx(1)}>
          ⏰ 시간 (cron)
        </div>
        <div class={`ts${trigIdx() === 2 ? " on" : ""}`} onClick={() => setTrigIdx(2)}>
          🔗 Webhook
        </div>
      </div>
      <Show when={trigIdx() === 1}>
        <div class="cronrow">
          <input
            class="ctl2"
            style={{ "font-family": "ui-monospace,Menlo,monospace" }}
            value={cron()}
            onInput={(e) => setCron(e.currentTarget.value)}
          />
        </div>
      </Show>
      <Show when={trigIdx() === 2}>
        <div class="builder-hint">🔗 Webhook 트리거는 저장 후 워크플로우 카드에서 발급됩니다.</div>
      </Show>

      <div class="bl">📣 외부 채널 (선택) — 채널에서 실행·상태조회</div>
      <select class="ctl2" value={channel()} onInput={(e) => setChannel(e.currentTarget.value)}>
        <option>연결 안 함</option>
        <option>Telegram</option>
        <option>Discord</option>
        <option>Slack</option>
      </select>
      <div class="builder-hint">
        연결 시 채널에서 <b>/run</b> 실행 · <b>/status</b> 진행상태 조회 · 완료 결과 자동 게시.
      </div>

      <div class="bl">🧩 단계 — 어떤 에이전트가 어떤 순서로 (위→아래 실행)</div>
      <div class="stepchips">
        <For each={steps()}>
          {(s, i) => (
            <span
              class={`schip${dragIdx() === i() ? " dragging" : ""}`}
              draggable={true}
              onDragStart={() => setDragIdx(i())}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => onDrop(i())}
            >
              <span class="grip">⠿</span>
              <span class="num">{i() + 1}</span>
              {s}
              <button class="schip-x" onClick={() => removeStep(i())}>
                ✕
              </button>
            </span>
          )}
        </For>
        <span class="schip" style={{ "border-style": "dashed" }}>
          <select
            class="ctl2"
            style={{ border: "none", background: "transparent", "max-width": "130px" }}
            title="등록 에이전트 선택 → 단계 앞에 채움"
            onInput={(e) => {
              const v = e.currentTarget.value;
              if (v) setStepDraft(`${v} · `);
              e.currentTarget.value = "";
            }}
          >
            <option value="">에이전트 선택…</option>
            <For each={agents() ?? []}>
              {(a) => <option value={a.alias}>{a.display_name || a.alias}</option>}
            </For>
          </select>
          <input
            class="ctl2"
            style={{ border: "none", padding: "0", background: "transparent" }}
            placeholder="에이전트 · 작업 (예: akashic · 요약)"
            value={stepDraft()}
            onInput={(e) => setStepDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") addStep();
            }}
          />
          <button class="addstep" onClick={addStep}>
            ＋ 추가
          </button>
        </span>
      </div>
      <div class="draghint">
        ⠿ 손잡이를 잡고 <b>드래그</b>하면 순서가 바뀝니다.
      </div>

      <Show when={note()}>
        <div class={`builder-note${note()!.err ? " err" : ""}`}>{note()!.text}</div>
      </Show>

      <div style={{ display: "flex", gap: "10px", "align-items": "center" }}>
        <button class="savewf" disabled={busy()} onClick={save}>
          {busy() ? "저장 중…" : "워크플로우 저장"}
        </button>
        <button class="addstep" style={{ "margin-top": "14px" }} onClick={props.onClose}>
          취소
        </button>
      </div>
    </div>
  );
}

function trigInfo(w: Workflow): { cls: string; icon: string; label: string } {
  if (w.cron_expr) return { cls: "cron", icon: "⏰", label: `cron · ${w.cron_expr}` };
  return { cls: "", icon: "🎯", label: "수동 / 트리거 실행" };
}

function WorkflowsView(props: {
  workflows: Workflow[] | undefined;
  loading: boolean;
  error: unknown;
  busy: string | null;
  note: { id: string; text: string; err: boolean } | null;
  onRun: (id: string) => void;
  onDelete: (id: string, name: string) => void;
  onOpenOrg: () => void;
  onOpenA2a: () => void;
  onOpenRun: (w: Workflow) => void;
}) {
  const list = createMemo(() => props.workflows ?? []);
  return (
    <Show when={!props.loading} fallback={<div class="kk-flow-empty">불러오는 중…</div>}>
      <Show
        when={!props.error}
        fallback={
          <div class="kk-flow-empty err">⚠ 워크플로우를 불러오지 못했습니다. 데몬 연결을 확인하세요.</div>
        }
      >
        {/* A2A 위임 진입 — 워크플로우 유무와 무관하게 항상 노출.
            ACP(나↔에이전트)와 구분되는 에이전트↔에이전트 레이어. */}
        <div class="a2a-entry">
          <span class="a2a-entry-tit">🔗 A2A 위임</span>
          <span class="a2a-entry-sub">에이전트↔에이전트 — 다른 A2A 에이전트에게 작업 위임</span>
          <button class="a2abtn" onClick={props.onOpenA2a}>
            열기
          </button>
        </div>
        <Show
          when={list().length > 0}
          fallback={
            <div class="kk-flow-empty">
              아직 등록된 워크플로우가 없습니다.<br />
              <span class="sub">위 ＋ 만들기로 목표·트리거·단계를 구성해 보세요.</span>
            </div>
          }
        >
          {/* 목업 .wfproj 프로젝트 그룹 헤더 + 🗂 조직도 진입. DTO 에 project 필드가 없어
              단일 그룹으로 묶고 조직도는 orchestration_agents(실데이터)로 연다. */}
          <div class="wfproj">
            🔀 워크플로우 <span class="cnt">· 흐름 {list().length}</span>
            <button class="orgbtn" onClick={props.onOpenOrg}>
              🗂 조직도
            </button>
          </div>
          <For each={list()}>
            {(w) => {
              const tg = trigInfo(w);
              const on = () => w.enabled !== false;
              return (
                <div class="wfcard flat" onClick={() => props.onOpenRun(w)}>
                  <div class="wftop">
                    <b>{w.name || w.id}</b>
                    <span class={`trig${tg.cls ? " " + tg.cls : ""}`}>
                      {tg.icon} {tg.label}
                    </span>
                    <span class={`onoff${on() ? " on" : " off"}`} onClick={(e) => e.stopPropagation()}>
                      {on() ? "ON" : "OFF"}
                    </span>
                  </div>
                  <Show when={w.description}>
                    <div class="goal">🎯 목표: {w.description}</div>
                  </Show>
                  <div class="kk-wfmeta">
                    <Show when={w.orchestrator}>
                      <span class="kk-wfchip">🗂 {w.orchestrator}</span>
                    </Show>
                    <Show when={w.cost_limit != null}>
                      <span class="kk-wfchip">👛 {w.cost_limit} USDC</span>
                    </Show>
                  </div>
                  <Show when={props.note && props.note.id === w.id}>
                    <div class={`kk-wfnote${props.note!.err ? " err" : ""}`}>{props.note!.text}</div>
                  </Show>
                  <div class="runline" onClick={(e) => e.stopPropagation()}>
                    <span class="rdot" classList={{ off: !on() }} />
                    <span class="kk-wftime" style={{ cursor: "pointer" }} onClick={() => props.onOpenRun(w)}>
                      📜 실행 이력 보기
                    </span>
                    <button
                      class="kk-wfbtn run"
                      disabled={props.busy === w.id}
                      onClick={() => props.onRun(w.id)}
                    >
                      {props.busy === w.id ? "실행 중…" : "▶ 실행"}
                    </button>
                    <button
                      class="kk-wfbtn"
                      disabled={props.busy === w.id}
                      onClick={() => props.onDelete(w.id, w.name || w.id)}
                    >
                      🗑 삭제
                    </button>
                  </div>
                </div>
              );
            }}
          </For>
        </Show>
      </Show>
    </Show>
  );
}

// ── 프로젝트 조직도 ── 목업 #orgOvl — orchestration_agents 의 reports_to 계층 ──
function OrgOverlay(props: { onClose: () => void; colorClass: (a: OrgAgent) => string }) {
  const [agents] = createResource<OrgAgent[]>(() => invoke<OrgAgent[]>("orchestration_agents"));
  const roots = createMemo(() => (agents() ?? []).filter((a) => !a.reports_to));
  const children = (parent: string) => (agents() ?? []).filter((a) => a.reports_to === parent);

  function badge(a: OrgAgent) {
    const r = (a.orchestration_role ?? "").toLowerCase();
    if (r.includes("lead") || r.includes("primary"))
      return <span class="obadge ob-lead">프로젝트 리드</span>;
    if (r.includes("special") || r.includes("특수"))
      return <span class="obadge ob-special">특수 기능</span>;
    return null;
  }

  function node(a: OrgAgent) {
    return (
      <div class="orgnode">
        <div class={`oava ${props.colorClass(a)}`}>{(a.alias || "?").charAt(0).toUpperCase()}</div>
        <div>
          <div class="on2">{a.alias}</div>
          <div class="or">{a.role || a.description || a.orchestration_role || "에이전트"}</div>
        </div>
        {badge(a)}
      </div>
    );
  }

  return (
    <div class="ovl" onClick={props.onClose}>
      <div class="board" onClick={(e) => e.stopPropagation()}>
        <div class="bh">
          <h2>🗂 조직도</h2>
          <span class="sub">이 워크스페이스에서 일하는 에이전트</span>
          <span class="bx" onClick={props.onClose}>
            ✕
          </span>
        </div>
        <div class="bb">
          <Show when={!agents.loading} fallback={<div class="org-empty">불러오는 중…</div>}>
            <Show
              when={!agents.error}
              fallback={<div class="org-empty">⚠ 조직도를 불러오지 못했습니다. 데몬 연결을 확인하세요.</div>}
            >
              <Show
                when={(agents() ?? []).length > 0}
                fallback={<div class="org-empty">등록된 에이전트가 없습니다.</div>}
              >
                <For each={roots()}>
                  {(r) => (
                    <>
                      {node(r)}
                      <Show when={children(r.alias).length > 0}>
                        <div class="orgindent">
                          <For each={children(r.alias)}>{(c) => node(c)}</For>
                        </div>
                      </Show>
                    </>
                  )}
                </For>
              </Show>
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
}

// ── A2A 위임 (에이전트↔에이전트) ── Phase 3, ACP-A2A-CORE.md ──
// ACP = 나↔에이전트, A2A = 에이전트↔에이전트. 이 뷰는 후자.
// a2a_agents 로 A2A 도달 가능 에이전트를 나열하되, 현재 OpenXgram 에이전트는
// AgentCard 호스팅 전이라 reachable:false + note(후속) 를 정직하게 렌더한다.
// 그래서 위임 target 은 수동 외부 A2A 에이전트 base URL 입력으로 받는다.
function A2AOverlay(props: { onClose: () => void }) {
  const [resp] = createResource<A2AAgentsResp>(() => invoke<A2AAgentsResp>("a2a_agents"));
  const agents = createMemo<A2AAgent[]>(() => {
    const r = resp();
    // a2a_agents emptyAs:[] → 배열로 올 수도, {agents,note} 객체로 올 수도 있으니 둘 다 수용.
    if (Array.isArray(r)) return r as A2AAgent[];
    return r?.agents ?? [];
  });
  const note = createMemo<string | null>(() => {
    const r = resp();
    return Array.isArray(r) ? null : (r?.note ?? null);
  });
  const reachableCount = createMemo(() => agents().filter((a) => a.reachable).length);

  // 위임 폼 상태.
  const [target, setTarget] = createSignal("");
  const [skill, setSkill] = createSignal("");
  const [task, setTask] = createSignal("");
  const [fromAgent, setFromAgent] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  const [sent, setSent] = createSignal<A2ASendResp | null>(null);

  // 폴링 상태.
  const [polling, setPolling] = createSignal(false);
  const [pollErr, setPollErr] = createSignal<string | null>(null);
  const [taskState, setTaskState] = createSignal<A2ATaskResp | null>(null);

  // target 선택 헬퍼 — reachable 에이전트의 agentCardUrl 을 폼에 채운다.
  function pick(a: A2AAgent) {
    if (a.agentCardUrl) setTarget(a.agentCardUrl);
  }

  async function send() {
    const t = target().trim();
    if (!t) {
      setErr("target — 외부 A2A 에이전트 URL 을 입력하세요.");
      return;
    }
    setBusy(true);
    setErr(null);
    setSent(null);
    setTaskState(null);
    setPollErr(null);
    try {
      const args: Record<string, unknown> = { target: t };
      if (skill().trim()) args.skill = skill().trim();
      if (task().trim()) args.task = task().trim();
      if (fromAgent().trim()) args.from_agent = fromAgent().trim();
      const r = await invoke<A2ASendResp>("a2a_send", args);
      setSent(r);
    } catch (e) {
      // discovery 실패 / 광고된 skill 없음 등 — 메시지 그대로 노출.
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function poll() {
    const s = sent();
    if (!s) return;
    setPolling(true);
    setPollErr(null);
    try {
      const r = await invoke<A2ATaskResp>("a2a_task_get", { id: s.taskId, target: s.target });
      setTaskState(r);
    } catch (e) {
      setPollErr((e as Error).message);
    } finally {
      setPolling(false);
    }
  }

  return (
    <div class="ovl" onClick={props.onClose}>
      <div class="board wide" onClick={(e) => e.stopPropagation()}>
        <div class="bh">
          <h2>🔗 A2A 위임</h2>
          <span class="sub">에이전트↔에이전트 (A2A) — ACP(나↔에이전트)와 다른 레이어</span>
          <span class="bx" onClick={props.onClose}>
            ✕
          </span>
        </div>
        <div class="bb">
          <div class="a2a-explain">
            이 화면은 <b>에이전트↔에이전트(A2A)</b> 위임 레이어입니다. 내가 직접 에이전트와 대화하는
            ACP(나↔에이전트)와 달리, 한 에이전트가 다른 에이전트의 AgentCard 를 통해 작업을 위임합니다.
          </div>

          {/* A2A 도달 가능 에이전트 목록 */}
          <div class="a2a-sec-tit">A2A 도달 가능 에이전트</div>
          <Show when={!resp.loading} fallback={<div class="org-empty">불러오는 중…</div>}>
            <Show
              when={!resp.error}
              fallback={
                <div class="org-empty">⚠ A2A 에이전트를 불러오지 못했습니다. 데몬 연결을 확인하세요.</div>
              }
            >
              <Show when={note()}>
                <div class="a2a-note">ℹ {note()}</div>
              </Show>
              <Show
                when={agents().length > 0}
                fallback={
                  <div class="org-empty">
                    현재 A2A 로 도달 가능한 OpenXgram 에이전트가 없습니다.
                    <br />
                    <span class="a2a-mut">
                      OpenXgram 에이전트의 AgentCard 호스팅은 후속 작업입니다. 그 전까지는 아래에서
                      외부 A2A 에이전트 URL 을 직접 입력해 위임하세요.
                    </span>
                  </div>
                }
              >
                <Show when={reachableCount() === 0}>
                  <div class="a2a-mut" style={{ "margin-bottom": "8px" }}>
                    아래 에이전트는 아직 AgentCard 를 호스팅하지 않아 모두 <b>도달 불가</b> 상태입니다
                    (후속 작업). 위임은 외부 A2A 에이전트 URL 입력으로 진행하세요.
                  </div>
                </Show>
                <For each={agents()}>
                  {(a) => (
                    <div class="a2a-agent">
                      <span class="a2a-ag-name">{a.alias}</span>
                      <Show when={a.agentCardUrl}>
                        <span class="a2a-ag-url">{a.agentCardUrl}</span>
                      </Show>
                      <span class={`a2a-rb${a.reachable ? " ok" : " no"}`}>
                        {a.reachable ? "도달 가능" : "도달 불가"}
                      </span>
                      <Show when={a.reachable && a.agentCardUrl}>
                        <button class="a2a-pick" onClick={() => pick(a)}>
                          target 채우기
                        </button>
                      </Show>
                    </div>
                  )}
                </For>
              </Show>
            </Show>
          </Show>

          {/* 위임 폼 */}
          <div class="a2a-sec-tit" style={{ "margin-top": "18px" }}>
            작업 위임 (A2A task 전송)
          </div>
          <div class="bl2">외부 A2A 에이전트 URL (target)</div>
          <input
            class="ctl2"
            placeholder="예: https://agent.example.com (AgentCard base URL)"
            value={target()}
            onInput={(e) => setTarget(e.currentTarget.value)}
          />
          <div class="bl2">skill (대상이 광고하는 스킬 이름 — 선택)</div>
          <input
            class="ctl2"
            placeholder="예: summarize"
            value={skill()}
            onInput={(e) => setSkill(e.currentTarget.value)}
          />
          <div class="bl2">task (위임할 작업 내용)</div>
          <input
            class="ctl2"
            placeholder="예: 이 문서를 3줄로 요약해줘"
            value={task()}
            onInput={(e) => setTask(e.currentTarget.value)}
          />
          <div class="bl2">from_agent (보내는 에이전트 alias — 선택)</div>
          <input
            class="ctl2"
            placeholder="예: Starian"
            value={fromAgent()}
            onInput={(e) => setFromAgent(e.currentTarget.value)}
          />

          <Show when={err()}>
            <div class="a2a-err">⚠ 전송 실패: {err()}</div>
          </Show>

          <div style={{ "margin-top": "12px" }}>
            <button class="savewf" disabled={busy()} onClick={send}>
              {busy() ? "전송 중…" : "🔗 작업 위임"}
            </button>
          </div>

          {/* 전송 결과 + 폴링 */}
          <Show when={sent()}>
            <div class="a2a-result">
              <div class="a2a-res-row">
                <b>taskId</b> <span class="a2a-mono">{sent()!.taskId}</span>
              </div>
              <div class="a2a-res-row">
                <b>target</b> <span class="a2a-mono">{sent()!.target}</span>
              </div>
              <Show when={sent()!.skill}>
                <div class="a2a-res-row">
                  <b>skill</b> {sent()!.skill}
                </div>
              </Show>
              <Show when={sent()!.fromAgent}>
                <div class="a2a-res-row">
                  <b>from</b> {sent()!.fromAgent}
                </div>
              </Show>
              <div style={{ "margin-top": "10px" }}>
                <button class="kk-wfbtn run" disabled={polling()} onClick={poll}>
                  {polling() ? "조회 중…" : "↻ 상태 조회"}
                </button>
              </div>
              <Show when={pollErr()}>
                <div class="a2a-err" style={{ "margin-top": "8px" }}>
                  ⚠ 상태 조회 실패: {pollErr()}
                </div>
              </Show>
              <Show when={taskState()}>
                <div class="a2a-tasklog">{JSON.stringify(taskState()!.task, null, 2)}</div>
              </Show>
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
}

// ── 실행 이력 상세 ── 목업 #runOvl — workflow_runs(실데이터) ──
function RunOverlay(props: { workflow: Workflow; onClose: () => void }) {
  const [runs] = createResource<WfRun[]>(() =>
    invoke<WfRun[]>("workflow_runs", { id: props.workflow.id }),
  );
  const [openId, setOpenId] = createSignal<string | null>(null);

  function statCls(s: string): string {
    const v = s.toLowerCase();
    if (v.includes("ok") || v.includes("success") || v.includes("complete") || v.includes("done"))
      return "ok";
    if (v.includes("fail") || v.includes("error")) return "fail";
    return "run";
  }
  function dur(r: WfRun): string {
    if (!r.finished_at) return "진행 중";
    const a = new Date(r.started_at.replace(" ", "T") + "Z").getTime();
    const b = new Date(r.finished_at.replace(" ", "T") + "Z").getTime();
    if (Number.isNaN(a) || Number.isNaN(b)) return "";
    return `${((b - a) / 1000).toFixed(1)}s`;
  }

  return (
    <div class="ovl" onClick={props.onClose}>
      <div class="board wide" onClick={(e) => e.stopPropagation()}>
        <div class="bh">
          <h2>{props.workflow.name || props.workflow.id}</h2>
          <span class="sub">실행 이력 (run history)</span>
          <span class="bx" onClick={props.onClose}>
            ✕
          </span>
        </div>
        <div class="bb">
          <Show when={!runs.loading} fallback={<div class="run-empty">불러오는 중…</div>}>
            <Show
              when={!runs.error}
              fallback={<div class="run-empty">⚠ 실행 이력을 불러오지 못했습니다. 데몬 연결을 확인하세요.</div>}
            >
              <Show
                when={(runs() ?? []).length > 0}
                fallback={<div class="run-empty">실행 이력이 없습니다. ▶ 실행을 누르면 기록이 생깁니다.</div>}
              >
                <For each={runs()}>
                  {(r) => {
                    const open = () => openId() === r.id;
                    return (
                      <div class="runrow">
                        <div class="rh" onClick={() => setOpenId(open() ? null : r.id)}>
                          <span class="rt">{fmtRunTs(r.started_at)}</span>
                          <span class="rg">{r.current_step ? `· ${r.current_step}` : ""}</span>
                          <span class={`rstat ${statCls(r.status)}`}>
                            {r.status} · {dur(r)}
                          </span>
                        </div>
                        <Show when={open()}>
                          <div class="runsteps show">
                            <div class="rstep">
                              <span class={`si ${statCls(r.status)}`}>
                                {statCls(r.status) === "ok" ? "✓" : statCls(r.status) === "fail" ? "✗" : "•"}
                              </span>
                              상태: {r.status}
                              <span class="sd">{dur(r)}</span>
                            </div>
                            <Show when={r.current_step}>
                              <div class="rstep">
                                <span class="si run">•</span>현재 단계: {r.current_step}
                                <span class="sd" />
                              </div>
                            </Show>
                            <Show when={r.total_cost != null}>
                              <div class="rstep">
                                <span class="si ok">👛</span>비용
                                <span class="sd">{r.total_cost} USDC</span>
                              </div>
                            </Show>
                            <div class="runlog">
                              run {r.id}
                              {"\n"}started: {r.started_at}
                              {r.finished_at ? `\nfinished: ${r.finished_at}` : "\n(진행 중)"}
                            </div>
                          </div>
                        </Show>
                      </div>
                    );
                  }}
                </For>
              </Show>
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
}

function SchedulesView(props: {
  schedules: Schedule[] | undefined;
  loading: boolean;
  error: unknown;
  stats: Stats | undefined;
  onCancel: (id: string) => void;
}) {
  const list = createMemo(() => props.schedules ?? []);
  return (
    <div>
      <div class="kk-flow-stats">
        <div class="kk-statcard">
          <span class="v">{props.stats?.pending ?? 0}</span>
          <span class="l">대기</span>
        </div>
        <div class="kk-statcard">
          <span class="v">{props.stats?.sent ?? 0}</span>
          <span class="l">발송</span>
        </div>
        <div class="kk-statcard">
          <span class="v">{props.stats?.failed ?? 0}</span>
          <span class="l">실패</span>
        </div>
        <div class="kk-statcard">
          <span class="v">{props.stats?.cancelled ?? 0}</span>
          <span class="l">취소</span>
        </div>
      </div>

      <div class="wfproj">⏰ 스케줄 · cron <span class="cnt">· {list().length}</span></div>
      <Show when={!props.loading} fallback={<div class="kk-flow-empty">불러오는 중…</div>}>
        <Show
          when={!props.error}
          fallback={
            <div class="kk-flow-empty err">⚠ 스케줄을 불러오지 못했습니다. 데몬 연결을 확인하세요.</div>
          }
        >
          <Show
            when={list().length > 0}
            fallback={<div class="kk-flow-empty">예약된 스케줄이 없습니다.</div>}
          >
            <For each={list()}>
              {(s) => {
                const cron = s.schedule_kind === "cron";
                return (
                  <div class="wfcard flat">
                    <div class="wftop">
                      <b>
                        {s.target_kind}: {s.target}
                      </b>
                      <span class={`trig${cron ? " cron" : ""}`}>
                        {cron ? "⏰" : "📅"} {s.schedule_kind} · {s.schedule_value}
                      </span>
                      <span class={`onoff status-${s.status}`}>{s.status}</span>
                    </div>
                    <div class="goal">{s.payload}</div>
                    <Show when={s.last_error}>
                      <div class="kk-wfnote err">에러: {s.last_error}</div>
                    </Show>
                    <div class="runline">
                      <span class="rdot" classList={{ off: s.status !== "pending" }} />
                      <span class="kk-wftime">
                        <Show when={s.next_due_at_kst} fallback={<>등록 {fmtTs(s.created_at_kst)}</>}>
                          다음 {fmtTs(s.next_due_at_kst)}
                        </Show>
                      </span>
                      <button
                        class="kk-wfbtn"
                        disabled={s.status !== "pending"}
                        onClick={() => props.onCancel(s.id)}
                      >
                        ✕ 취소
                      </button>
                    </div>
                  </div>
                );
              }}
            </For>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
