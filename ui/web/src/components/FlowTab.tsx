import { createResource, createSignal, createMemo, For, Show } from "solid-js";
import { invoke } from "../api/client";

// 흐름 탭 — 카카오톡 정본 목업(_mockups/kakao-mockup.html) 충실 이식.
// 정본 #wfOvl 의 .board / .bh / .bb / .wfcard / .wftop / .trig / .onoff / .goal / .runline / .rdot 마크업·CSS 를
// 그대로(verbatim) 포팅하고, 샘플 텍스트만 라이브 데이터로 치환. 오버레이 chrome(.ovl/.bx) 은 탭 본문이라 제거.
// 워크플로우 DTO 에 project 필드가 없으므로 .wfproj 그룹은 만들지 않고 flat 렌더(가짜 그룹 X). 카드 스타일은 정본 그대로.
// 백엔드 contract 재사용 (신규 명령 발명 X — WorkflowPanel/ScheduleView 의 invoke 그대로):
//   workflows_list   → Workflow[]   (워크플로우 보드)
//   workflow_run     → { run_id }   (▶ 실행)
//   workflow_delete  → ()           (삭제)
//   schedule_list    → Schedule[]   (스케줄/cron 목록)
//   schedule_stats   → Stats        (집계 칩)
//   schedule_cancel  → ()           (스케줄 취소)
// daemon no-fallback 규칙: 로딩/에러/빈 상태를 모두 명시적으로 표시.

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

  // 정본 .ovl > .board 구조를 탭 본문(.kk-flow)으로 인라인화. .board 의 .bh / .bb 그대로.
  // 헤더는 세그먼트 토글(워크플로우/스케줄) 을 .bh 안에 배치.
  return (
    <div class="kk-flow">
      <div class="board">
        <div class="bh">
          <h2>🔀 흐름</h2>
          <span class="sub">paperclip 대신 가볍게 · 목표 + 트리거(시간·webhook)</span>
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
            <WorkflowsView
              workflows={workflows()}
              loading={workflows.loading}
              error={workflows.error}
              busy={busyWf()}
              note={wfNote()}
              onRun={runWorkflow}
              onDelete={deleteWorkflow}
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
        <Show
          when={list().length > 0}
          fallback={
            <div class="kk-flow-empty">
              아직 등록된 워크플로우가 없습니다.<br />
              <span class="sub">paperclip 대신 가볍게 · 목표 + 트리거(시간·webhook). 백엔드 엔진은 준비됨.</span>
            </div>
          }
        >
          {/* 정본 .wfproj 는 project 필드가 DTO 에 없어 만들지 않음. flat 카운트 헤더만. */}
          <div class="wfproj">🔀 워크플로우 <span class="cnt">· 흐름 {list().length}</span></div>
          <For each={list()}>
            {(w) => {
              const tg = trigInfo(w);
              const on = () => w.enabled !== false;
              return (
                // 정본 .wfcard 그대로. project 그룹이 없으므로 들여쓰기(.wfcard margin-left)는 .flat 으로 0.
                <div class="wfcard flat">
                  <div class="wftop">
                    <b>{w.name || w.id}</b>
                    <span class={`trig${tg.cls ? " " + tg.cls : ""}`}>
                      {tg.icon} {tg.label}
                    </span>
                    <span class={`onoff${on() ? " on" : " off"}`}>{on() ? "ON" : "OFF"}</span>
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
                  <div class="runline">
                    <span class="rdot" classList={{ off: !on() }} />
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
