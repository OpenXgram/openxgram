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

  // P6 — 보안 공유방(방 단위 vault). 멤버만 항목 목록/추가/열람. 값은 항상 마스킹, reveal 만 평문.
  // 비멤버는 403 (백엔드 gate) — UI 도 곁뷰 자체가 그 방 멤버에게만 의미가 있다.
  interface VaultItem { item_key: string; kind: string; sensitive: boolean; value_masked: string; file_hash?: string | null; created_by: string; created_at: string; }
  interface VaultResp { room_key: string; items: VaultItem[]; rotation_needed: boolean; }
  const [showVault, setShowVault] = createSignal(false);
  const [vaultItems, setVaultItems] = createSignal<VaultItem[]>([]);
  const [vaultRotation, setVaultRotation] = createSignal(false);
  const [vaultMsg, setVaultMsg] = createSignal<string>("");
  const [revealed, setRevealed] = createSignal<Record<string, string>>({});
  const [newKey, setNewKey] = createSignal("");
  const [newVal, setNewVal] = createSignal("");
  const [newSensitive, setNewSensitive] = createSignal(false);
  async function refreshVault() {
    const room = props.selfAlias;
    if (!room) return;
    try {
      const r = await invoke<VaultResp>("room_vault_list", { key: room });
      setVaultItems(r.items ?? []);
      setVaultRotation(!!r.rotation_needed);
    } catch (e) {
      // 비멤버(403)면 접근 불가 메시지.
      setVaultMsg(`보안 스코프 접근 불가: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  async function addVaultItem() {
    const room = props.selfAlias;
    if (!room) { setVaultMsg("방(현재 대화)을 먼저 선택하세요."); return; }
    const ik = newKey().trim();
    if (!ik) { setVaultMsg("항목 이름을 입력하세요."); return; }
    if (!newVal().trim()) { setVaultMsg("값을 입력하세요."); return; }
    setVaultMsg(`🔒 ${ik} 저장 중… (vault 암호화)`);
    try {
      await invoke("room_vault_put", { key: room, item_key: ik, value: newVal(), kind: "secret", sensitive: newSensitive() });
      setVaultMsg(`✓ ${ik} 저장됨 — 멤버만 열람할 수 있습니다.`);
      setNewKey(""); setNewVal(""); setNewSensitive(false);
      void refreshVault();
    } catch (e) {
      setVaultMsg(`저장 실패/대기: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  async function revealItem(item: string) {
    const room = props.selfAlias;
    if (!room) return;
    try {
      const r = await invoke<{ value: string }>("room_vault_reveal", { key: room, item });
      setRevealed((m) => ({ ...m, [item]: r.value }));
      setVaultMsg(`🔓 ${item} 열람 (감사 로그 기록됨)`);
    } catch (e) {
      // 민감 항목은 마스터 승인/MFA 대기 메시지가 그대로 노출된다.
      setVaultMsg(`열람 거부/승인 대기: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  return (
    <>
      {/* ── A2A 실시간 미니패널 (정본 .a2a-mini) — 대화 헤더 바로 아래 한 줄 요약.
          클래스는 flow-extra.css 의 .a2a-mini* 패밀리와 정확히 매칭(스코프 충돌 없음).
          이전엔 bare .mini/.st 를 썼는데 그 CSS 는 mockup.css 의 .oxg-app 스코프 전용이라
          라이브 경로(.oxg-app 래퍼 없음)에서 무스타일 → strip 깨짐 버그였다(fix). ── */}
      <div class="a2a-mini" onClick={() => props.onOpen()} title="협업(A2A) 곁뷰 열기">
        <b>🔗 협업 현황</b>
        <Show
          when={agents().length > 0}
          fallback={<span class="a2a-mini-st"><span class="a2a-mini-dot idle" /> 활성 협업 없음</span>}
        >
          <For each={live().slice(0, 2)}>
            {(a) => (<span class="a2a-mini-st"><span class="a2a-mini-dot live" /> ↔{a.alias} 진행 가능</span>)}
          </For>
          <For each={idle().slice(0, 1)}>
            {(a) => (<span class="a2a-mini-st"><span class="a2a-mini-dot idle" /> ↔{a.alias} 대기</span>)}
          </For>
          <Show when={agents().length > 0}>
            <span class="a2a-mini-badge">{agents().length}</span>
          </Show>
        </Show>
        <span class="a2a-mini-sp" />
        <span class="a2a-mini-more">자세히 ›</span>
      </div>

      {/* ── 협업(A2A) 곁뷰 — 우측 슬라이드 패널 (정본 .side#sideA2A) ── */}
      <div class="side" classList={{ show: props.open() }}>
        <h3>
          🔗 {props.selfAlias ?? "에이전트"}의 협업 (에이전트간 ACP)
          <span class="x" onClick={() => props.onClose()}>✕</span>
        </h3>
        <div class="convs">
          <Show
            when={agents().length > 0}
            fallback={
              <div style="font-size:12px;color:var(--muted);line-height:1.5">
                도달 가능한 A2A 에이전트가 없습니다.<br />
                에이전트가 AgentCard 를 호스팅하면 여기에 협업 상대로 나타납니다.
              </div>
            }
          >
            <For each={agents()}>
              {(a) => (
                <div class="conv">
                  <div class="av" style={`width:36px;height:36px;background:${a.reachable ? "#5aa469" : "#7c8ba1"}`}>{a.alias.slice(0, 1).toUpperCase()}</div>
                  <div>
                    <div class="nm">↔ {a.alias}</div>
                    <div class="lt">{a.reachable ? "도달 가능 — A2A 위임 가능" : "미도달 — 대기"}</div>
                  </div>
                  <div class="stt">
                    <Show when={a.reachable || granting() === a.alias} fallback={<>⚪ 대기</>}>
                      <span class="live" /> {granting() === a.alias ? "턴 진행중" : "진행 가능"}
                    </Show>
                  </div>
                  <button class="give" style="margin-left:6px;border:1px solid #cdd9e4;background:#eef4fa;border-radius:7px;padding:4px 9px;font-size:11px;font-weight:700;color:#5a7fb0;cursor:pointer"
                    disabled={granting() !== null}
                    title="이 에이전트에게 발언권 부여 (누적 맥락 + 방/역할 지침)"
                    onClick={() => void grantTurn(a.alias)}>🎙</button>
                </div>
              )}
            </For>
          </Show>
          <Show when={grantMsg()}><div class="grantmsg">{grantMsg()}</div></Show>
        </div>

        {/* ── P4c — 오케스트레이션 실행 러너 ── */}
        <div class="secblk">
          <div class="sech">🔢 오케스트레이션
            <Show when={orch() && (orch()!.total_steps ?? 0) > 0}>
              <span class="n">{orch()!.status === "running" ? "🟢 실행중"
                : orch()!.status === "paused_for_approval" ? "⏸ 승인대기"
                : orch()!.status === "done" ? "✓ 완료"
                : orch()!.status === "failed" ? "✗ 실패"
                : orch()!.status === "cancelled" ? "취소됨" : orch()!.status}</span>
            </Show>
          </div>
          <Show
            when={orch() && (orch()!.total_steps ?? 0) > 0}
            fallback={<div style="font-size:11.5px;color:var(--muted);line-height:1.5">이 방에 실행할 단계가 없습니다. <b>방 설정</b>에서 순서(작업→검증→승인)를 먼저 구성하세요.</div>}
          >
            <For each={orch()!.steps}>
              {(s, i) => (
                <div class="orchstep2" classList={{ cur: i() === orch()!.current_step && orchRunning() }}>
                  <span class="ic">{stepIcon(s.state)}</span>
                  <span class="lbl">{s.label || `단계 ${i() + 1}`}</span>
                  <span class="ag">↔ {s.agent || "—"}</span>
                </div>
              )}
            </For>
          </Show>
          <Show when={orchMsg()}><div class="grantmsg">{orchMsg()}</div></Show>
        </div>

        {/* ── P5 — 방 멤버 목록 ── */}
        <div class="secblk">
          <div class="sech">👥 참가자 <span class="n">{members().length}</span></div>
          <Show when={members().length > 0} fallback={<div style="font-size:11.5px;color:var(--muted);line-height:1.5">{membersNote() ?? "1:1 — 사람 + 이 에이전트. 초대 시 그룹으로 승격."}</div>}>
            <For each={members()}>
              {(m) => (
                <div class="memrow">
                  <span class="av" style={`background:${m.is_human ? "linear-gradient(135deg,#ffd84d,#ff9e2c)" : "#7c8ba1"}`}>{m.is_human ? "👑" : m.alias.slice(0, 1).toUpperCase()}</span>
                  <span>{m.is_human ? "나 (고권한)" : m.alias}</span>
                  <span class="rl">{m.role ?? "참가자"}</span>
                  <Show when={!m.is_human}><button class="ej" title="내보내기 (수신 중단)" onClick={() => void ejectMember(m.alias)}>🚪</button></Show>
                </div>
              )}
            </For>
          </Show>
          <Show when={showInvite()}>
            <div style="margin-top:4px">
              <div class="sech">＋ 누구를 초대할까요?</div>
              <Show when={inviteCandidates().length > 0} fallback={<div style="font-size:11.5px;color:var(--muted)">초대 가능한 에이전트가 없습니다.</div>}>
                <For each={inviteCandidates()}>
                  {(a) => (
                    <div class="invrow">
                      <span class="av" style="width:24px;height:24px;border-radius:8px;background:#7c8ba1;color:#fff;display:grid;place-items:center;font-size:10px">{a.alias.slice(0, 1).toUpperCase()}</span>
                      {a.alias}
                      <button class="go" onClick={() => void inviteMember(a.alias)}>초대</button>
                    </div>
                  )}
                </For>
              </Show>
            </div>
          </Show>
          <Show when={memberMsg()}><div class="grantmsg">{memberMsg()}</div></Show>
        </div>

        {/* ── P6 보안 공유방 ── */}
        <Show when={showVault()}>
          <div class="secblk">
            <div class="sech">🔒 보안 공유방 <span class="n">{vaultItems().length}</span> · 멤버 전용</div>
            <Show when={vaultRotation()}><div class="grantmsg" style="color:#f0a020">⚠️ 멤버 퇴장 발생 — 키 회전이 필요할 수 있습니다 (마스터 결정 대기).</div></Show>
            <Show when={vaultItems().length > 0} fallback={<div style="font-size:11.5px;color:var(--muted)">공유된 항목이 없습니다. 아래에서 키/값을 추가하세요.</div>}>
              <For each={vaultItems()}>
                {(it) => (
                  <div class="vrow">
                    {it.kind === "file" ? "📎" : "🔑"} {it.item_key}
                    <Show when={it.sensitive}><span style="font-size:10px;color:#a07800;background:#fff3cf;border-radius:6px;padding:1px 6px;font-weight:700">🛡 민감</span></Show>
                    <span class="val">{revealed()[it.item_key] ?? it.value_masked}</span>
                    <Show when={it.kind !== "file"}><button class="rv" title="평문 열람 (감사 기록)" onClick={() => void revealItem(it.item_key)}>👁</button></Show>
                  </div>
                )}
              </For>
            </Show>
            <div class="vadd">
              <input placeholder="항목 이름 (예: api_key)" value={newKey()} onInput={(e) => setNewKey(e.currentTarget.value)} />
              <input type="password" placeholder="값 (암호화)" value={newVal()} onInput={(e) => setNewVal(e.currentTarget.value)} />
              <label style="font-size:11px;color:#4a555f;display:flex;align-items:center;gap:4px"><input type="checkbox" checked={newSensitive()} onChange={(e) => setNewSensitive(e.currentTarget.checked)} /> 민감</label>
              <button class="go" style="border:1px solid #cdd9e4;background:#eef4fa;border-radius:7px;padding:5px 10px;font-size:11px;font-weight:700;color:#5a7fb0;cursor:pointer" onClick={() => void addVaultItem()}>＋ 추가</button>
            </div>
            <Show when={vaultMsg()}><div class="grantmsg">{vaultMsg()}</div></Show>
          </div>
        </Show>

        <div style="padding:0 14px 4px;font-size:11px;color:var(--muted)">↑ 대화방 클릭 → 발언권/오케스트레이션/멤버/보안방 제어</div>

        {/* ── 제어 버튼 (정본 .ctrl) — 초대/내보내기/발언권/보안방 ── */}
        <div class="ctrl">
          <button disabled={!props.selfAlias} title="이 방에 에이전트 추가 (맥락 인계)" onClick={() => { setShowInvite((v) => !v); void refreshMembers(); }}>＋ 에이전트 초대</button>
          <button disabled={!props.selfAlias || members().filter((m) => !m.is_human).length === 0} title="멤버 목록의 🚪 버튼으로 내보내기" onClick={() => void refreshMembers()}>🚪 내보내기</button>
          <button disabled={granting() !== null || !props.selfAlias} title="현재 대화 에이전트에게 발언권 부여" onClick={() => props.selfAlias && void grantTurn(props.selfAlias)}>🎙 발언권 주기</button>
          <button disabled={orchBusy() || orchRunning() || !props.selfAlias} title="방 단계를 순서대로 실행" onClick={() => void startOrch()}>▶ 오케스트레이션</button>
          <Show when={orch()?.status === "paused_for_approval"}><button disabled={orchBusy()} onClick={() => void approveOrch()}>승인</button></Show>
          <Show when={orchRunning()}><button onClick={() => void cancelOrch()}>취소</button></Show>
          <button disabled={!props.selfAlias} title="이 방의 공유 보안 스코프 (멤버만 열람)" onClick={() => { setShowVault((v) => !v); void refreshVault(); }}>🔒 보안방 {showVault() ? "닫기" : "만들기"}</button>
        </div>
      </div>
    </>
  );
}
