import { createResource, createSignal, createMemo, createEffect, onCleanup, For, Show } from "solid-js";
import { invoke, a2aActivityStream } from "../api/client";
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

// ── AUTO-POP (rc.334) — A2A 대화를 새 브라우저 창으로 ──
// 창 레지스트리(모듈 전역): conv 키(alias) → WindowProxy. 같은 대화는 하나의 창만(중복 방지).
// 모듈 전역이라 패널이 리렌더돼도 열린 창 핸들이 보존된다.
const a2aWindows = new Map<string, Window>();

// 팝업 URL = 기존 메커니즘 재사용(AcpConversation.openPopout 과 동일한 `?chat=<alias>`).
function popoutUrl(alias: string): string {
  return `${location.origin}${location.pathname}?chat=${encodeURIComponent(alias)}`;
}

// 클릭(사용자 제스처) 또는 활동 자동(차단될 수 있음)으로 alias 대화 창을 연다.
// 이미 열려 있으면 그 창을 재사용해 URL 만 보장(중복 창 X). 포커스는 '열리는 순간'만 OS 가 가져가며,
// 직후 opener(메인 창)로 포커스를 되돌린다. 메시지 도착 시에는 절대 .focus() 안 함(팝업이 title 깜빡임으로 처리).
// 반환: 열기/재사용 성공 시 true, 팝업 차단(window.open === null) 시 false.
function openA2AWindow(alias: string): boolean {
  const existing = a2aWindows.get(alias);
  if (existing && !existing.closed) {
    // 이미 열림 — 중복 생성 금지. URL 만 보장(흰화면 방지). focus 는 사용자 제스처 클릭일 때만 best-effort.
    try { if (!existing.location.href.includes(`chat=${encodeURIComponent(alias)}`)) existing.location.href = popoutUrl(alias); } catch { /* cross-origin 아님(같은 오리진) */ }
    return true;
  }
  // 팝업 차단기 친화: 사용자 제스처 중엔 window.open(URL, ...) 형태(첫 인자에 실제 URL)가
  //   window.open("") + 이후 .href 할당보다 훨씬 안정적으로 허용된다. URL 을 바로 넘긴다.
  const w = window.open(popoutUrl(alias), `oxgchat_${alias}`, "width=480,height=820");
  if (!w) {
    // 팝업 차단 — 자동 열기(비-제스처)에서 흔함. 호출자가 in-app 깜빡임으로 graceful fallback.
    a2aWindows.delete(alias);
    return false;
  }
  // 일부 브라우저는 window.open(URL) 만으로 로드한다. 안전망으로 about:blank 이면 한 번 더 지정.
  try { if (!w.location || w.location.href === "about:blank") w.location.href = popoutUrl(alias); } catch { /* same-origin */ }
  a2aWindows.set(alias, w);
  // 🔒 포커스 탈취 금지: 새 창이 막 열릴 때 OS 가 포커스를 가져가는 것이 '유일하게 허용된' 포커스 순간.
  //    직후 opener(메인 창)로 포커스를 되돌린다(best-effort). 이후 메시지 도착 시엔 절대 .focus() 안 함.
  try { window.focus(); } catch { /* best-effort */ }
  return true;
}

// 토글 영속(localStorage): per-agent `oxg.autopop.<alias>`, global `oxg.autopop.all`.
function readAll(): boolean {
  try { return localStorage.getItem("oxg.autopop.all") === "1"; } catch { return false; }
}
function readPerAgent(alias: string): boolean {
  try { return localStorage.getItem(`oxg.autopop.${alias}`) === "1"; } catch { return false; }
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

  // ── AUTO-POP 상태 ──
  // 토글(전역 + per-agent) — localStorage 와 동기화하는 반응형 신호.
  const [allOn, setAllOn] = createSignal<boolean>(readAll());
  const [perAgent, setPerAgent] = createSignal<Record<string, boolean>>({});
  const isAutoOn = (alias: string) => allOn() || (perAgent()[alias] ?? readPerAgent(alias));
  function toggleAll() {
    const v = !allOn();
    setAllOn(v);
    try { localStorage.setItem("oxg.autopop.all", v ? "1" : "0"); } catch { /* ignore */ }
  }
  function togglePerAgent(alias: string) {
    const cur = perAgent()[alias] ?? readPerAgent(alias);
    const v = !cur;
    setPerAgent((p) => ({ ...p, [alias]: v }));
    try { localStorage.setItem(`oxg.autopop.${alias}`, v ? "1" : "0"); } catch { /* ignore */ }
  }
  // per-agent 토글 초기 로드(localStorage → 신호).
  createEffect(() => {
    const map: Record<string, boolean> = {};
    for (const a of agents()) map[a.alias] = readPerAgent(a.alias);
    setPerAgent(map);
  });

  // 클릭(사용자 제스처)으로 대화 창 열기 + 차단 시 안내.
  const [popBlocked, setPopBlocked] = createSignal<string | null>(null);
  function clickOpen(alias: string) {
    const ok = openA2AWindow(alias);
    if (!ok) {
      // 팝업 차단(브라우저 팝업 차단기) → 사용자가 떠 있는 strip 의 칩을 눌렀는데 창이 안 뜨면
      // "아무것도 안 보임" 으로 느낀다(곁뷰가 닫혀 있어 안내도 안 보임). 가시적 fallback 보장:
      //   1) 곁뷰(side panel)를 즉시 연다 → 안내 배너 + 클릭 가능한 에이전트 행이 보인다.
      //   2) 떠 있는 strip 위에도 floating 안내 배너를 띄운다(곁뷰가 미처 안 열려도 보이게).
      // 이렇게 하면 절대 "무반응" 이 아니다.
      setPopBlocked(alias);
      if (!props.open()) props.onOpen();
    } else if (popBlocked() === alias) {
      setPopBlocked(null);
    }
  }

  // ── 자동 열기 ──
  const [autoBlink, setAutoBlink] = createSignal<Set<string>>(new Set());

  // 토글이 켜진 에이전트의 대화 창을 (재사용·중복방지) 열고, 팝업 차단 시 in-app 깜빡임으로 fallback.
  // 포커스 탈취 없음(openA2AWindow 가 열리는 순간 외엔 .focus() 안 함). 활동 신호의 단일 진입점.
  function autoPop(alias: string) {
    if (!alias || !isAutoOn(alias)) return;
    const ok = openA2AWindow(alias);
    if (!ok) {
      // 팝업 차단(자동·비제스처) → in-app strip 깜빡임 + 안내로 graceful fallback(에러 X).
      setPopBlocked(alias);
      setAutoBlink((s) => new Set(s).add(alias));
      setTimeout(() => setAutoBlink((s) => { const n = new Set(s); n.delete(alias); return n; }), 6000);
    }
  }

  // ── 자동 열기 트리거 #1 (PRIMARY) — 실제 A2A 메시지 SSE ──
  // 백엔드 `/v1/gui/a2a/stream` 구독: 어느 대화든 새 A2A 메시지/턴이 acp_messages 에 영속되면
  // `a2a_message` 이벤트가 온다. 그 즉시(reachability 전이를 기다리지 않고) 해당 대화 창을 auto-pop.
  // = 실제 메시지 단위 트리거(종전 10초 reachability poll 근사 대체). 토글로 게이트, 포커스 탈취 없음.
  const stopStream = a2aActivityStream(
    (ev) => {
      if (ev?.type !== "a2a_message") return;
      const alias = ev.alias || ev.conv_key;
      if (alias) autoPop(alias);
    },
    () => { /* 스트림 끊김 = 조용한 status fallback(아래 reachability poll 이 계속 커버) */ },
  );
  onCleanup(() => stopStream());

  // ── 자동 열기 트리거 #2 (FALLBACK) — reachability 전이 ──
  // status 폴링(reachable 플래그)의 미도달→도달 전이를 보조 활동 신호로 유지한다. SSE 가 끊겼거나
  // (장애/네트워크) 에이전트가 막 온라인된 경우의 안전망. 메시지 단위 정밀도는 #1(SSE)이 담당.
  let prevReachable = new Map<string, boolean>();
  createEffect(() => {
    const cur = agents();
    const next = new Map<string, boolean>();
    for (const a of cur) {
      next.set(a.alias, a.reachable);
      const was = prevReachable.get(a.alias);
      // idle→live 전이 = 활동 근사 신호(보조).
      if (a.reachable && was === false) autoPop(a.alias);
    }
    prevReachable = next;
  });

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

  // #2b — 스크롤 본문 어포던스: 내용이 넘치고 아직 바닥이 아니면 하단 페이드(.scrollable),
  // 다 보이거나 바닥까지 스크롤하면 페이드 제거 → '더 있음' vs '여기가 끝' 을 명확히 구분.
  function syncScrollAffordance(el: HTMLElement | null) {
    if (!el) return;
    const overflowing = el.scrollHeight - el.clientHeight > 4;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 4;
    el.classList.toggle("scrollable", overflowing && !atBottom);
  }

  return (
    <>
      {/* ── A2A 실시간 미니패널 (정본 .a2a-mini) — 대화 헤더 바로 아래 한 줄 요약.
          클래스는 flow-extra.css 의 .a2a-mini* 패밀리와 정확히 매칭(스코프 충돌 없음).
          이전엔 bare .mini/.st 를 썼는데 그 CSS 는 mockup.css 의 .oxg-app 스코프 전용이라
          라이브 경로(.oxg-app 래퍼 없음)에서 무스타일 → strip 깨짐 버그였다(fix). ── */}
      {/* 떠 있는 협업현황 strip(.a2a-mini) — 대화 위 한 줄 요약. 곁뷰가 열리면 패널이 같은 정보를
          자체 strip(.a2a-side-strip)으로 보여주므로 떠 있는 strip 은 숨겨 패널 헤더와의 겹침을 막는다(#1). */}
      <Show when={!props.open()}>
        <div class="a2a-mini" onClick={() => props.onOpen()} title="협업(A2A) 곁뷰 열기">
          <b>🔗 협업 현황</b>
          <Show
            when={agents().length > 0}
            fallback={<span class="a2a-mini-st"><span class="a2a-mini-dot idle" /> 활성 협업 없음</span>}
          >
            <For each={live().slice(0, 2)}>
              {(a) => (<span class="a2a-mini-st a2a-mini-chip" classList={{ "a2a-blink": autoBlink().has(a.alias) }} title={`↔${a.alias} 대화를 새 창으로 열기`}
                onClick={(e) => { e.stopPropagation(); clickOpen(a.alias); }}><span class="a2a-mini-dot live" /> ↔{a.alias} 진행 가능 🗗</span>)}
            </For>
            <For each={idle().slice(0, 1)}>
              {(a) => (<span class="a2a-mini-st a2a-mini-chip" classList={{ "a2a-blink": autoBlink().has(a.alias) }} title={`↔${a.alias} 대화를 새 창으로 열기`}
                onClick={(e) => { e.stopPropagation(); clickOpen(a.alias); }}><span class="a2a-mini-dot idle" /> ↔{a.alias} 대기 🗗</span>)}
            </For>
            <Show when={agents().length > 0}>
              <span class="a2a-mini-badge">{agents().length}</span>
            </Show>
          </Show>
          <span class="a2a-mini-sp" />
          <span class="a2a-mini-more">자세히 ›</span>
        </div>
        {/* 팝업 차단 시 떠 있는 strip 위 floating 안내(곁뷰가 닫혀 있어도 보이게) — "무반응" 방지.
            클릭하면 곁뷰를 열어 클릭 가능한 에이전트 행 + 상세 안내를 보여준다. */}
        <Show when={popBlocked()}>
          <div class="a2a-mini-popblock" title="협업 곁뷰 열기 (대화 시작)"
            onClick={(e) => { e.stopPropagation(); props.onOpen(); }}>
            ⚠ 팝업이 차단되어 ‹{popBlocked()}› 새 창을 열지 못했습니다 — 여기를 눌러 곁뷰에서 대화하세요 ›
          </div>
        </Show>
      </Show>

      {/* ── 협업(A2A) 곁뷰 — 우측 슬라이드 패널 (정본 .side#sideA2A).
          깔끔한 세로 스택: 헤더(곁뷰 전용 닫기) → 협업현황 strip → 스크롤 에이전트 목록 →
          오케스트레이션/참가자 → 하단 제어 버튼. (이전엔 떠 있는 strip 이 헤더를 덮었다 — #1) ── */}
      <div class="side a2a-side-panel" classList={{ show: props.open() }}>
        <h3>
          {/* 곁뷰 전용 닫기 — 카톡 패널처럼 좌측 back 버튼으로 닫는다(대화 헤더 우상단 아이콘 클러스터와 hit area 가 겹치지 않게).
              대화 ✕ 와 구별되는 명확한 라벨(‹ 닫기). z-index 를 대화 헤더(.chat-top z:30) 위로 올려 클릭이 가로채이지 않게 한다(#3). */}
          <button class="a2a-side-close" title="협업 곁뷰 닫기 (대화로 돌아가기)" aria-label="협업 곁뷰 닫기" onClick={() => props.onClose()}>‹ 닫기</button>
          <span class="a2a-side-title">🔗 {props.selfAlias ?? "에이전트"}의 협업 (에이전트간 ACP)</span>
          {/* 전역 AUTO-POP 토글 — 모든 에이전트의 A2A 활동 시 자동으로 새 창을 연다. localStorage 영속. */}
          <button class="a2a-autopop-all" classList={{ on: allOn() }}
            title={allOn() ? "자동 새 창: 모든 에이전트에 적용 중 — 끄기" : "자동 새 창: 모든 에이전트에 적용 — 켜기"}
            onClick={() => toggleAll()}>{allOn() ? "🔔 모든 에이전트 자동열기 ON" : "🔕 모든 에이전트 자동열기 OFF"}</button>
        </h3>

        {/* 협업현황 strip — 패널 안 자체 한 줄(in-flow). 떠 있는 strip 과 달리 헤더와 겹치지 않는다(#1).
            가로 스크롤 가능 시 우측 페이드 chevron 으로 '더 있음' 단서(#2a). */}
        <div class="a2a-side-strip-wrap">
          <div class="a2a-mini a2a-side-strip">
            <b>🔗 협업 현황</b>
            <Show
              when={agents().length > 0}
              fallback={<span class="a2a-mini-st"><span class="a2a-mini-dot idle" /> 활성 협업 없음</span>}
            >
              <For each={live()}>
                {(a) => (<span class="a2a-mini-st a2a-mini-chip" classList={{ "a2a-blink": autoBlink().has(a.alias) }} title={`↔${a.alias} 대화를 새 창으로 열기`}
                  onClick={(e) => { e.stopPropagation(); clickOpen(a.alias); }}><span class="a2a-mini-dot live" /> ↔{a.alias} 진행 가능 🗗</span>)}
              </For>
              <For each={idle()}>
                {(a) => (<span class="a2a-mini-st a2a-mini-chip" classList={{ "a2a-blink": autoBlink().has(a.alias) }} title={`↔${a.alias} 대화를 새 창으로 열기`}
                  onClick={(e) => { e.stopPropagation(); clickOpen(a.alias); }}><span class="a2a-mini-dot idle" /> ↔{a.alias} 대기 🗗</span>)}
              </For>
              <span class="a2a-mini-badge">{agents().length}</span>
            </Show>
            <span class="a2a-mini-sp" />
          </div>
        </div>

        {/* 스크롤 본문 — 에이전트 목록 + 오케스트레이션 + 참가자(+보안방). flex:1 + min-height:0 으로
            실제로 스크롤. 내용이 넘치면 하단 페이드(.a2a-side-body2.scrollable)로 '더 있음' 단서,
            다 보이면 페이드 없이 깔끔히 끝남(#2b). */}
        <div class="a2a-side-body2" ref={(el) => queueMicrotask(() => syncScrollAffordance(el))} onScroll={(e) => syncScrollAffordance(e.currentTarget)}>
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
                <div class="conv" classList={{ "a2a-blink": autoBlink().has(a.alias) }}>
                  {/* 행 클릭(아바타·이름 영역) → 이 에이전트 A2A 대화를 새 창으로 (사용자 제스처). */}
                  <div class="av" style={`width:36px;height:36px;background:${a.reachable ? "#5aa469" : "#7c8ba1"};cursor:pointer`}
                    title={`↔${a.alias} 대화를 새 창으로 열기`}
                    onClick={() => clickOpen(a.alias)}>{a.alias.slice(0, 1).toUpperCase()}</div>
                  <div style="cursor:pointer" title={`↔${a.alias} 대화를 새 창으로 열기`} onClick={() => clickOpen(a.alias)}>
                    <div class="nm">↔ {a.alias} 🗗</div>
                    <div class="lt">{a.reachable ? "도달 가능 — A2A 위임 가능" : "미도달 — 대기"}</div>
                  </div>
                  <div class="stt">
                    <Show when={a.reachable || granting() === a.alias} fallback={<>⚪ 대기</>}>
                      <span class="live" /> {granting() === a.alias ? "턴 진행중" : "진행 가능"}
                    </Show>
                  </div>
                  {/* per-agent AUTO-POP 토글 — 켜면 이 에이전트 활동 시 자동으로 새 창. (전역 ON 이면 항상 적용) */}
                  <button class="a2a-autopop-toggle" classList={{ on: isAutoOn(a.alias) }}
                    title={allOn() ? "자동 열기: 전역 ON (모든 에이전트 적용 중)" : (isAutoOn(a.alias) ? "자동 열기 ON — 끄기" : "자동 열기 OFF — 켜기")}
                    disabled={allOn()}
                    onClick={() => togglePerAgent(a.alias)}>{isAutoOn(a.alias) ? "🔔 자동" : "🔕 자동"}</button>
                  <button class="give" style="margin-left:6px;border:1px solid #cdd9e4;background:#eef4fa;border-radius:7px;padding:4px 9px;font-size:11px;font-weight:700;color:#5a7fb0;cursor:pointer"
                    disabled={granting() !== null}
                    title="이 에이전트에게 발언권 부여 (누적 맥락 + 방/역할 지침)"
                    onClick={() => void grantTurn(a.alias)}>🎙</button>
                </div>
              )}
            </For>
          </Show>
          <Show when={popBlocked()}>
            <div class="grantmsg" style="background:#fff3cf;color:#8a6d1a">
              ⚠ 팝업이 차단되어 ‹{popBlocked()}› 창을 자동으로 열지 못했습니다. 위 칩/행을 직접 클릭하면 열립니다(차단 해제 권장).
            </div>
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
        </div>{/* /a2a-side-body2 (스크롤 본문 끝) */}

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
