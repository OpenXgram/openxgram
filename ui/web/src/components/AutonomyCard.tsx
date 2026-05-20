import { createSignal, Show } from "solid-js";
import { ScheduleView } from "./ScheduleView";
import { ChainView } from "./ChainView";
import { Breadcrumb } from "./Breadcrumb";

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
        <section class="card-section">
          <h3>⚡ SelfTrigger — 사양 §3.2 (M-5 V-7)</h3>
          <p class="placeholder-note">
            이벤트 → 작업 규칙. 예: "Discord 새 메시지 도착 → ZAL-001 깨움 + recv_messages 호출".
            백엔드 SelfTriggerRule 테이블 + 이벤트 버스 신설 필요.
          </p>
        </section>
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
        <section class="card-section">
          <h3>📜 실행 이력 — 사양 §3.4 (M-10)</h3>
          <p class="placeholder-note">
            90일 보존. cron · SelfTrigger · reflection · 통합 timeline.
            성공률 · 평균 토큰 · 평균 비용 · 실패 사유.
          </p>
        </section>
      </Show>

      <Show when={tab() === "limit"}>
        <section class="card-section">
          <h3>🚦 자율 한도 — 사양 §3.5 (M-7 V-9)</h3>
          <p class="placeholder-note">
            일·월·세션별 자율 행동 한도. 도달 시 휴면 또는 사용자 승인 요청.
            메신저 탭 7 의 일·월 한도와 별도 (이쪽은 자율 trigger 횟수, 메신저는 결제 비용).
          </p>
        </section>
        <section class="card-section">
          <h3>🏖️ 휴가 모드 — 사양 §4.4 (M-12 V-10)</h3>
          <p class="placeholder-note">
            기간 지정 → 모든 자율 행동 일시정지. 채널 인박스만 받기 (사람 메시지). 종료 시 자동 재개.
          </p>
        </section>
      </Show>
    </div>
  );
}
