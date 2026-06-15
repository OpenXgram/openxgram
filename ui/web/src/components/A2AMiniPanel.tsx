import { createResource, createSignal, createMemo, createEffect, onCleanup, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./flow-extra.css";

// P2 — 사람↔ACP 대화 안의 A2A(에이전트↔에이전트) 실시간 미니패널 + 협업 곁뷰.
// 정본 디자인: _mockups/openxgram-conversation-model-mockup.html (.mini / #sideA2A).
//
// 데이터 출처: 기존 `a2a_agents` 라우트(GET /v1/gui/a2a/agents) — 새 엔드포인트를 만들지 않음.
//   응답: 배열 [{alias, reachable, agentCardUrl}] 또는 { agents, note }.
//   reachable=true → 🟢(도달 가능/진행 가능), false → ⚪(대기/미도달). 가짜 "턴 상태" 만들지 않음.
//
// 주의(P3+ 미구현 셸): 방별 발언권(턴 제어)·초대/내보내기·보안방은 백엔드 미구축이라
//   곁뷰의 제어 버튼은 비활성(non-enforced) 셸로 노출하고 데이터를 위조하지 않는다.

interface A2AAgent {
  alias: string;
  reachable: boolean;
  agentCardUrl?: string | null;
}
interface A2AAgentsResp {
  agents: A2AAgent[];
  note?: string | null;
}

function normalizeAgents(r: A2AAgent[] | A2AAgentsResp | null | undefined): A2AAgent[] {
  if (!r) return [];
  if (Array.isArray(r)) return r as A2AAgent[];
  return (r as A2AAgentsResp).agents ?? [];
}

/**
 * 미니패널 + 협업 곁뷰. selfAlias 는 현재 대화 중인 사람↔ACP 에이전트 alias(요약 라벨에 사용).
 * open/onToggle 로 곁뷰(side panel) 열림을 부모(TalkTab)가 제어 — 작업환경(tmux) 곁뷰와 상호배타.
 */
export function A2AMiniPanel(props: {
  selfAlias?: string | null;
  open: () => boolean;
  onOpen: () => void;
  onClose: () => void;
}) {
  // 기존 a2a_agents 엔드포인트만 사용. SSE /stream 은 방별 A2A 상태 이벤트를 싣지 않으므로
  // (그건 ACP 세션 스트림임) 폴링으로 실시간성을 낸다 — 10초 간격.
  const [resp, { refetch }] = createResource<A2AAgent[] | A2AAgentsResp>(() =>
    invoke<A2AAgent[] | A2AAgentsResp>("a2a_agents"),
  );
  const timer = setInterval(() => void refetch(), 10000);
  onCleanup(() => clearInterval(timer));

  const agents = createMemo<A2AAgent[]>(() => normalizeAgents(resp()));
  const live = createMemo(() => agents().filter((a) => a.reachable));
  const idle = createMemo(() => agents().filter((a) => !a.reachable));

  // P4a — 발언권 주기(턴 부여). 진행자(사람)가 특정 에이전트에게 "지금 발언하라"를 누른다.
  // 방 키 = 현재 대화 중인 사람↔ACP 에이전트 alias(selfAlias). 누적 맥락 + 방/역할 지침으로 한 번 턴 발화.
  const [granting, setGranting] = createSignal<string | null>(null);
  const [grantMsg, setGrantMsg] = createSignal<string>("");

  // P4c — 오케스트레이션 실행. 방의 단계(orchestration_json)를 데몬이 순서대로 실제 실행.
  // status 폴링(3초)으로 현재 단계/상태를 실시간 표시. 곁뷰가 열려 있을 때만 폴링.
  interface OrchStep { label: string; agent: string; role: string; action?: string | null; state: string; result?: string | null; error?: string | null; }
  interface OrchStatus { run_id: string | null; current_step: number; total_steps: number; status: string; error?: string | null; steps: OrchStep[]; }
  const [orch, setOrch] = createSignal<OrchStatus | null>(null);
  const [orchBusy, setOrchBusy] = createSignal(false);
  const [orchMsg, setOrchMsg] = createSignal<string>("");
  async function refreshOrch() {
    const room = props.selfAlias;
    if (!room) return;
    try {
      const s = await invoke<OrchStatus>("room_orchestrate_status", { key: room });
      setOrch(s);
    } catch {
      /* 상태 없음 — 조용히 무시(시작 전). */
    }
  }
  const orchTimer = setInterval(() => { if (props.open()) void refreshOrch(); }, 3000);
  onCleanup(() => clearInterval(orchTimer));
  // 곁뷰가 열리는 순간 즉시 1회 상태 조회(3초 틱 대기 없이). 닫히면 폴링은 위 틱에서 자동 정지.
  createEffect(() => { if (props.open() && props.selfAlias) void refreshOrch(); });
  async function startOrch() {
    const room = props.selfAlias;
    if (!room) { setOrchMsg("방(현재 대화)을 먼저 선택하세요."); return; }
    setOrchBusy(true);
    setOrchMsg("▶ 오케스트레이션 시작…");
    try {
      await invoke("room_orchestrate_start", { key: room });
      setOrchMsg("실행 중 — 단계별 진행을 표시합니다.");
      void refreshOrch();
    } catch (e) {
      setOrchMsg(`시작 실패: ${e instanceof Error ? e.message : String(e)}`);
    } finally { setOrchBusy(false); }
  }
  async function approveOrch() {
    const room = props.selfAlias;
    if (!room) return;
    setOrchBusy(true);
    try {
      await invoke("room_orchestrate_approve", { key: room });
      setOrchMsg("✓ 승인 — 다음 단계로 진행합니다.");
      void refreshOrch();
    } catch (e) {
      setOrchMsg(`승인 실패: ${e instanceof Error ? e.message : String(e)}`);
    } finally { setOrchBusy(false); }
  }
  async function cancelOrch() {
    const room = props.selfAlias;
    if (!room) return;
    try { await invoke("room_orchestrate_cancel", { key: room }); setOrchMsg("실행을 취소했습니다."); void refreshOrch(); }
    catch (e) { setOrchMsg(`취소 실패: ${e instanceof Error ? e.message : String(e)}`); }
  }
  const stepIcon = (st: string) =>
    st === "done" ? "✓" : st === "running" ? "🟢" : st === "paused_for_approval" ? "⏸" : st === "failed" ? "✗" : "○";
  const orchRunning = createMemo(() => { const s = orch()?.status; return s === "running" || s === "paused_for_approval"; });
  async function grantTurn(agent: string) {
    const room = props.selfAlias;
    if (!room) {
      setGrantMsg("방(현재 대화)을 먼저 선택하세요.");
      return;
    }
    setGranting(agent);
    setGrantMsg(`🎙 ${agent} 에게 발언권 부여 중… (턴 진행)`);
    try {
      await invoke("room_grant_turn", { key: room, agent });
      setGrantMsg(`🟢 ${agent} 발언 완료 — 대화 스레드에 영속되었습니다.`);
      void refetch();
    } catch (e) {
      setGrantMsg(`발언권 부여 실패: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setGranting(null);
    }
  }

  // P5 — 방 동적 멤버십(초대/내보내기/멤버 목록). 카톡 단톡방 멤버 리스트처럼 단순하게.
  // 방 키 = selfAlias(현재 대화 에이전트). 멤버 목록은 곁뷰 열림 시 + 변경 후 폴링.
  interface RoomMember { alias: string; role?: string | null; joined_at: string; is_human: boolean; }
  interface MembersResp { room_key: string; members: RoomMember[]; note?: string | null; }
  const [members, setMembers] = createSignal<RoomMember[]>([]);
  const [membersNote, setMembersNote] = createSignal<string | null>(null);
  const [memberMsg, setMemberMsg] = createSignal<string>("");
  const [showInvite, setShowInvite] = createSignal(false);
  async function refreshMembers() {
    const room = props.selfAlias;
    if (!room) return;
    try {
      const r = await invoke<MembersResp>("room_members", { key: room });
      setMembers(r.members ?? []);
      setMembersNote(r.note ?? null);
    } catch { /* 없으면 빈 목록 */ }
  }
  createEffect(() => { if (props.open() && props.selfAlias) void refreshMembers(); });
  // 초대 후보 = a2a_agents 중 아직 활성 멤버가 아닌 에이전트(자기 자신 제외).
  const memberAliases = createMemo(() => new Set(members().map((m) => m.alias)));
  const inviteCandidates = createMemo(() =>
    agents().filter((a) => a.alias !== props.selfAlias && !memberAliases().has(a.alias)),
  );
  async function inviteMember(member: string) {
    const room = props.selfAlias;
    if (!room) { setMemberMsg("방(현재 대화)을 먼저 선택하세요."); return; }
    setMemberMsg(`＋ ${member} 초대 중… (맥락 인계)`);
    try {
      await invoke("room_invite", { key: room, member });
      setMemberMsg(`✓ ${member} 초대됨 — 방 맥락이 인계되고 전달이 시작됩니다.`);
      setShowInvite(false);
      void refreshMembers();
    } catch (e) {
      setMemberMsg(`초대 실패: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  async function ejectMember(member: string) {
    const room = props.selfAlias;
    if (!room) return;
    if (!confirm(`'${member}' 를 이 방에서 내보낼까요? (수신 중단 · ACP 분리)`)) return;
    setMemberMsg(`🚪 ${member} 내보내는 중…`);
    try {
      await invoke("room_eject", { key: room, member });
      setMemberMsg(`✓ ${member} 내보냄 — 더 이상 발언권/턴 대상이 아닙니다.`);
      void refreshMembers();
    } catch (e) {
      setMemberMsg(`내보내기 실패: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  return (
    <>
      {/* ── A2A 실시간 미니패널 (정본 .mini) — 대화 헤더 바로 아래 한 줄 요약 ── */}
      <div class="a2a-mini" onClick={() => props.onOpen()} title="협업(A2A) 곁뷰 열기">
        <b>🔗 협업 현황</b>
        <Show
          when={agents().length > 0}
          fallback={<span class="a2a-mini-st idle"><span class="a2a-mini-dot idle" /> 활성 협업 없음</span>}
        >
          <For each={live().slice(0, 2)}>
            {(a) => (
              <span class="a2a-mini-st live">
                <span class="a2a-mini-dot live" /> ↔{a.alias} 진행 가능
              </span>
            )}
          </For>
          <For each={idle().slice(0, 1)}>
            {(a) => (
              <span class="a2a-mini-st idle">
                <span class="a2a-mini-dot idle" /> ↔{a.alias} 대기
              </span>
            )}
          </For>
        </Show>
        <span class="a2a-mini-sp" />
        <Show when={live().length > 0}>
          <span class="a2a-mini-badge">{live().length}</span>
        </Show>
        <span class="a2a-mini-more">자세히 ›</span>
      </div>

      {/* ── 협업(A2A) 곁뷰 — 우측 슬라이드 패널 (정본 #sideA2A). FlowTab A2A 뷰 스타일 재사용. ── */}
      <div class={`a2a-side${props.open() ? " show" : ""}`}>
        <h3>
          🔗 {props.selfAlias ?? "에이전트"}의 협업 (에이전트간 ACP)
          <span class="a2a-side-x" onClick={() => props.onClose()}>✕</span>
        </h3>
        <div class="a2a-side-body">
          <Show
            when={agents().length > 0}
            fallback={
              <div class="a2a-side-empty">
                도달 가능한 A2A 에이전트가 없습니다.<br />
                <span class="sub">에이전트가 AgentCard 를 호스팅하면 여기에 협업 상대로 나타납니다.</span>
              </div>
            }
          >
            <For each={agents()}>
              {(a) => (
                <div class="a2a-conv">
                  <div class={`a2a-conv-av${a.reachable ? "" : " off"}`}>{a.alias.slice(0, 1).toUpperCase()}</div>
                  <div class="a2a-conv-meta">
                    <div class="a2a-conv-nm">↔ {a.alias}</div>
                    <div class="a2a-conv-lt">{a.reachable ? "도달 가능 — A2A 위임 가능" : "미도달 — 대기"}</div>
                  </div>
                  <div class={`a2a-conv-stt${a.reachable ? " live" : ""}`}>
                    {granting() === a.alias ? "🟢 턴 진행중" : a.reachable ? "🟢 진행 가능" : "⚪ 대기"}
                  </div>
                  {/* P4a — 발언권 주기: 이 에이전트에게 지금 턴 부여. */}
                  <button
                    class="a2a-grant-btn"
                    disabled={granting() !== null}
                    title="이 에이전트에게 발언권을 줘 한 번 발언시킵니다 (누적 맥락 + 방/역할 지침)"
                    onClick={() => void grantTurn(a.alias)}
                  >🎙</button>
                </div>
              )}
            </For>
          </Show>
          <Show when={grantMsg()}>
            <div class="a2a-grant-msg">{grantMsg()}</div>
          </Show>

          {/* ── P4c — 오케스트레이션 실행 러너 (방 단계를 순서대로 실제 실행). ── */}
          <div class="a2a-orch">
            <div class="a2a-orch-hd">
              <b>🔢 오케스트레이션</b>
              <Show when={orch() && (orch()!.total_steps ?? 0) > 0}>
                <span class={`a2a-orch-st ${orch()!.status}`}>
                  {orch()!.status === "running" ? "🟢 실행중"
                    : orch()!.status === "paused_for_approval" ? "⏸ 승인대기"
                    : orch()!.status === "done" ? "✓ 완료"
                    : orch()!.status === "failed" ? "✗ 실패"
                    : orch()!.status === "cancelled" ? "취소됨"
                    : orch()!.status}
                </span>
              </Show>
            </div>
            <Show
              when={orch() && (orch()!.total_steps ?? 0) > 0}
              fallback={
                <div class="a2a-orch-empty">
                  이 방에 실행할 단계가 없습니다. <b>방 설정</b>에서 순서(작업→검증→승인)를 먼저 구성하세요.
                </div>
              }
            >
              <div class="a2a-orch-steps">
                <For each={orch()!.steps}>
                  {(s, i) => (
                    <div class={`a2a-orch-step${i() === orch()!.current_step && orchRunning() ? " cur" : ""}`}>
                      <span class="a2a-orch-icon">{stepIcon(s.state)}</span>
                      <span class="a2a-orch-lbl">{s.label || `단계 ${i() + 1}`}</span>
                      <span class="a2a-orch-ag">↔ {s.agent || "—"}</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
            <div class="a2a-orch-btns">
              <button
                class="a2a-orch-run"
                disabled={orchBusy() || orchRunning() || !props.selfAlias}
                title="방의 단계를 순서대로 실제 실행합니다"
                onClick={() => void startOrch()}
              >▶ 오케스트레이션 실행</button>
              <Show when={orch()?.status === "paused_for_approval"}>
                <button class="a2a-orch-approve" disabled={orchBusy()} onClick={() => void approveOrch()}>승인</button>
              </Show>
              <Show when={orchRunning()}>
                <button class="a2a-orch-cancel" onClick={() => void cancelOrch()}>취소</button>
              </Show>
            </div>
            <Show when={orchMsg()}>
              <div class="a2a-orch-msg">{orchMsg()}</div>
            </Show>
          </div>

          {/* ── P5 — 방 멤버 목록 (카톡 단톡방 멤버 리스트식). 사람=👑 고권한. ── */}
          <div class="a2a-members">
            <div class="a2a-members-h">👥 참가자 <span class="a2a-members-n">{members().length}</span></div>
            <Show
              when={members().length > 0}
              fallback={<div class="a2a-members-empty">{membersNote() ?? "1:1 — 사람 + 이 에이전트. 초대 시 그룹으로 승격."}</div>}
            >
              <For each={members()}>
                {(m) => (
                  <div class="a2a-member">
                    <span class="a2a-member-av">{m.is_human ? "👑" : m.alias.slice(0, 1).toUpperCase()}</span>
                    <span class="a2a-member-nm">{m.is_human ? "나 (고권한)" : m.alias}</span>
                    <span class="a2a-member-role">{m.role ?? "참가자"}</span>
                    <Show when={!m.is_human}>
                      <button class="a2a-member-eject" title="이 멤버 내보내기 (수신 중단)" onClick={() => void ejectMember(m.alias)}>🚪</button>
                    </Show>
                  </div>
                )}
              </For>
            </Show>
            {/* 초대 피커 — a2a_agents 후보에서 선택 */}
            <Show when={showInvite()}>
              <div class="a2a-invite-pick">
                <div class="a2a-invite-h">＋ 누구를 초대할까요?</div>
                <Show when={inviteCandidates().length > 0} fallback={<div class="a2a-members-empty">초대 가능한 에이전트가 없습니다.</div>}>
                  <For each={inviteCandidates()}>
                    {(a) => (
                      <div class="a2a-invite-row">
                        <span class="a2a-member-av">{a.alias.slice(0, 1).toUpperCase()}</span>
                        <span class="a2a-member-nm">{a.alias}</span>
                        <button class="a2a-invite-go" onClick={() => void inviteMember(a.alias)}>초대</button>
                      </div>
                    )}
                  </For>
                </Show>
              </div>
            </Show>
            <Show when={memberMsg()}>
              <div class="a2a-grant-msg">{memberMsg()}</div>
            </Show>
          </div>

          <div class="a2a-side-hint">
            ↑ 협업 위임·실행은 <b>워크플로우</b> 탭의 A2A 위임에서도 가능 (이 곁뷰는 현황 요약 + 실행).
          </div>
        </div>

        {/* ── 제어 버튼 — 발언권(P4a)+멤버십(P5) 활성. 보안방은 P6 미구축(비활성 셸). ── */}
        <div class="a2a-ctrl">
          <button
            disabled={!props.selfAlias}
            title="이 방에 에이전트 추가 (방 맥락 인계 + 전달 시작)"
            onClick={() => { setShowInvite((v) => !v); void refreshMembers(); }}
          >＋ 에이전트 초대</button>
          <button
            disabled={!props.selfAlias || members().filter((m) => !m.is_human).length === 0}
            title="멤버 목록에서 🚪 버튼으로 내보냅니다 (수신 중단 · ACP 분리)"
            onClick={() => void refreshMembers()}
          >🚪 내보내기</button>
          <button
            disabled={granting() !== null || !props.selfAlias}
            title="현재 대화 에이전트에게 발언권을 줘 한 번 발언시킵니다 (누적 맥락 + 방/역할 지침)"
            onClick={() => props.selfAlias && void grantTurn(props.selfAlias)}
          >🎙 발언권 주기</button>
          <button disabled title="P6 — 보안 공유방(vault scope) 백엔드 미구축">🔒 보안방 만들기</button>
        </div>
        <div class="a2a-ctrl-note">
          🎙 발언권(P4a) · ＋초대/🚪내보내기(P5) 동작합니다 — 초대 시 방 맥락 인계, 내보내기 시 수신 중단. 보안방은 다음 단계(P6).
        </div>
      </div>
    </>
  );
}
