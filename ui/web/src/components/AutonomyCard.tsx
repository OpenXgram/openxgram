import { createResource, createSignal, For, Show } from "solid-js";
import { ScheduleView } from "./ScheduleView";
import { ChainView } from "./ChainView";
import { Breadcrumb } from "./Breadcrumb";
import { invoke } from "@/api/client";

// UI-AUTONOMY-SPEC v1.0 §3 — ⏰ 자율 행동 카드 (PRD §0 #6).
// 4 섹션: Cron / SelfTrigger / Role 정책 (auto_respond 마스터) / 이력.
// + 자율 한도·휴가 모드 별도.

type Tab = "cron" | "trigger" | "role" | "history" | "limit";

export function AutonomyCard(props: { onBack: () => void }) {
  const [tab, setTab] = createSignal<Tab>("cron");

  return (
    <div class="card-page">
      <Breadcrumb cardName="⏰ 자율 행동" onReturn={props.onBack} />
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">⏰</span>
        <h1>자율 행동</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #6 — 자율 행동 ("에이전트"의 본질)</div>
      <div class="card-page-oneline">
        Cron · SelfTrigger · Role 정책 (auto_respond 마스터) · nightly reflection · 자율 한도 · 휴가 모드. "잠자는 동안 수익" Cognac 모델.
      </div>

      <nav style="display:flex; gap:4px; margin-bottom:14px;">
        <button class={"link-btn " + (tab() === "cron" ? "active" : "")} onClick={() => setTab("cron")}>⏰ Cron</button>
        <button class={"link-btn " + (tab() === "trigger" ? "active" : "")} onClick={() => setTab("trigger")}>⚡ SelfTrigger</button>
        <button class={"link-btn " + (tab() === "role" ? "active" : "")} onClick={() => setTab("role")}>🎭 Role 정책</button>
        <button class={"link-btn " + (tab() === "history" ? "active" : "")} onClick={() => setTab("history")}>📜 이력</button>
        <button class={"link-btn " + (tab() === "limit" ? "active" : "")} onClick={() => setTab("limit")}>🚦 한도·휴가</button>
      </nav>

      <Show when={tab() === "cron"}>
        <section class="card-section">
          <h3>⏰ Cron — 사양 §3.1 (M-1·M-2)</h3>
          <p class="placeholder-note">
            전체 cron 통합 (모든 세션 · 모든 작업). 자연어 cron 입력 ("매주 평일 오전 9시 → 0 9 * * 1-5"). 시스템 cron (heartbeat 등 — 사용자 비활성화 불가).
            기존 ScheduleView 통합. 작업 의존성 DAG (M-8) Phase 2.
          </p>
          <ScheduleView />
        </section>
        <section class="card-section">
          <h3>🔗 Chain — 메시지 체인 (cron 의 일종)</h3>
          <ChainView />
        </section>
      </Show>

      <Show when={tab() === "trigger"}>
        <SelfTriggerSection />
        <ReflectionSection />
      </Show>

      <Show when={tab() === "role"}>
        <section class="card-section">
          <h3>🎭 Role 정책 (auto_respond 마스터) — 사양 §3.3 (M-6 V-1)</h3>
          <p class="placeholder-note">
            역할별 auto_respond 기본값 (researcher / reviewer / coder / orchestrator / scribe / ...).
            메신저 탭 2는 뷰만 (이 카드가 마스터).
            예: researcher = true, reviewer = false, orchestrator = true.
            백엔드 RolePolicy 테이블 + `GET·PUT /v1/gui/autonomy/role` 신설 필요.
          </p>
        </section>
      </Show>

      <Show when={tab() === "history"}>
        <HistorySection />
      </Show>

      <Show when={tab() === "limit"}>
        <LimitsSection />
        <VacationSection />
      </Show>
    </div>
  );
}

function SelfTriggerSection() {
  const [list, { refetch }] = createResource<any[]>(async () => { try { return await invoke<any[]>("self_triggers_list"); } catch { return []; } });
  const [event, setEvent] = createSignal("");
  const [target, setTarget] = createSignal("");
  const [action, setAction] = createSignal("");
  async function add() {
    if (!event() || !target() || !action()) return;
    try { await invoke("self_trigger_add", { event_pattern: event(), target_agent: target(), action: action() }); setEvent(""); setTarget(""); setAction(""); await refetch(); } catch (e) { alert(String(e)); }
  }
  return (
    <section class="card-section">
      <h3>⚡ SelfTrigger — 사양 §3.2 (M-5 V-7)</h3>
      <div style="display:flex; flex-direction:column; gap:4px; margin-bottom:6px;">
        <input value={event()} onInput={(e) => setEvent(e.currentTarget.value)} placeholder="event_pattern (discord:new_message)" style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
        <input value={target()} onInput={(e) => setTarget(e.currentTarget.value)} placeholder="target_agent (ZAL-001)" style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
        <input value={action()} onInput={(e) => setAction(e.currentTarget.value)} placeholder="action (wake_and_recv_messages)" style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
        <button class="link-btn" onClick={add}>+ 규칙 추가</button>
      </div>
      <For each={list() ?? []}>{(r) => (
        <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);">
          <strong>{r.event_pattern}</strong> → <code>{r.target_agent}</code> · {r.action}
          <span style="color:var(--text-3); margin-left:6px;">fired {r.fire_count}회 · {r.active ? "active" : "off"}</span>
        </div>
      )}</For>
    </section>
  );
}

function ReflectionSection() {
  const [list, { refetch }] = createResource<any[]>(async () => { try { return await invoke<any[]>("reflection_runs_list"); } catch { return []; } });
  async function runNow() {
    try { await invoke("reflection_now", {}); await refetch(); } catch (e) { alert(String(e)); }
  }
  return (
    <section class="card-section">
      <h3>🪞 Reflection (nightly) — 사양 §3.3</h3>
      <button class="link-btn" onClick={runNow}>⏯ 지금 reflection 실행</button>
      <For each={list() ?? []}>{(r) => (
        <div style="font-size:11px; padding:4px 0; border-bottom:1px solid var(--border);">
          <span style="color:var(--text-3);">{r.started_at}</span> · {r.success ? "✓" : "—"} · 페이지 {r.new_pages} · 패턴 {r.patterns_found}
        </div>
      )}</For>
    </section>
  );
}

function HistorySection() {
  const [items] = createResource<any[]>(async () => { try { return await invoke<any[]>("autonomy_history"); } catch { return []; } });
  return (
    <section class="card-section">
      <h3>📜 실행 이력 — 사양 §3.4 (M-10 agent_lifecycle_log)</h3>
      <Show when={(items() ?? []).length === 0}>
        <div style="font-size:12px; color:var(--text-3);">이력 없음.</div>
      </Show>
      <For each={(items() ?? []).slice(0, 30)}>{(e) => (
        <div style="font-size:11px; padding:4px 0; border-bottom:1px solid var(--border);">
          <span style="color:var(--text-3);">{e.at}</span> · <strong>{e.action}</strong> · <code>{e.agent_id}</code>
          {e.reason && <span style="color:var(--text-3); margin-left:6px;">({e.reason})</span>}
        </div>
      )}</For>
    </section>
  );
}

function LimitsSection() {
  const [l] = createResource(async () => { try { return await invoke<any>("autonomy_limits"); } catch { return null; } });
  return (
    <section class="card-section">
      <h3>🚦 자율 한도 — 사양 §3.5 (M-7 V-9)</h3>
      <Show when={l()}>
        <div class="card-section-row"><span class="label">일 한도</span><span class="value">{l()?.today_used} / {l()?.daily_trigger_limit} trigger</span></div>
        <div class="card-section-row"><span class="label">월 한도</span><span class="value">{l()?.month_used} / {l()?.monthly_trigger_limit} trigger</span></div>
        <p style="font-size:11px; color:var(--text-3);">{l()?.note}</p>
      </Show>
    </section>
  );
}

function VacationSection() {
  const [v, { refetch }] = createResource(async () => { try { return await invoke<any>("autonomy_vacation"); } catch { return null; } });
  const [from, setFrom] = createSignal("");
  const [to, setTo] = createSignal("");
  async function setV() {
    if (!from() || !to()) return;
    try { await invoke("autonomy_vacation_set", { starts_at: from(), ends_at: to() }); await refetch(); } catch {}
  }
  return (
    <section class="card-section">
      <h3>🏖️ 휴가 모드 — 사양 §4.4 (M-12 V-10)</h3>
      <Show when={v()}>
        <div class="card-section-row"><span class="label">활성</span><span class="value">{v()?.active ? "✓" : "—"}</span></div>
        <div class="card-section-row"><span class="label">시작</span><span class="value">{v()?.starts_at || "미설정"}</span></div>
        <div class="card-section-row"><span class="label">종료</span><span class="value">{v()?.ends_at || "미설정"}</span></div>
      </Show>
      <div style="display:flex; gap:6px; margin-top:8px;">
        <input type="datetime-local" value={from()} onInput={(e) => setFrom(e.currentTarget.value)}
          style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
        <input type="datetime-local" value={to()} onInput={(e) => setTo(e.currentTarget.value)}
          style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
        <button class="link-btn" onClick={setV}>설정</button>
      </div>
    </section>
  );
}
