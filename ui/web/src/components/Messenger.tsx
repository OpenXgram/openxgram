import { createMemo, createResource, createSignal, createEffect, For, Show, onCleanup, onMount} from "solid-js";
import { invoke} from "@/api/client";
import { useI18n} from "../i18n";
import { AgentSidePanel} from "./AgentSidePanel";
import { SessionScreen} from "./SessionScreen";
import { RoutingRulesModal} from "./RoutingRulesModal";
import { WhitelistModal} from "./WhitelistModal";
import { WorkflowPanel} from "./WorkflowPanel";

// v1.3 Tier 1 — 좌측 머신×세션 트리 (UI-MESSENGER-SPEC §3.2, S4).
// - peer 목록 = 본인의 다른 머신/세션 — machine 별 그룹화
// - ▼/▶ collapse (S4) — 30+ 세션 한 화면 관리
// - 정렬 (이름·활동) + 필터 (전체·연결만·미연결만)
// - 4-tuple 부분표시: alias · machine · fingerprint (ULID 도입은 Tier 2 별 단계)
// - 채널(Discord/Telegram) 친구는 별 "채널" 그룹
// 중앙: L0 messages — 친구 sender 필터, 3초 폴링, peer 송신 활성 (Step 0 완료)

interface MessageDto {
 id: string;
 session_id: string;
 sender: string;
 body: string;
 timestamp: string;
 conversation_id: string;
 // rc.154 — ack tracking
 ack_status?: string;
 acked_at?: string;
 ack_via?: string;
 // rc.227 — application-level ACK (conversation_id 매칭 답신).
 app_ack_status?: string; // 'processed' | 'blocked' | undefined (대기 중)
 app_ack_at?: string;
}

// rc.154 — ack badge helper. rc.219 — outbound_queue.ack_status 도 함께 매핑.
// rc.227 — application-level ACK badge 도 transport badge 옆에 함께 렌더.
function ackBadge(m: MessageDto) {
 const s = m.ack_status;
 // transport ACK badge
 let transportEl: any = null;
 if (s && s !== "sent") {
  const map: Record<string, {bg: string; label: string; title: string}> = {
   // rc.153 message-level
   delivered: {bg: "#3a4a6a", label: "✓ delivered", title: "전달됨"},
   read: {bg: "#2a5b8a", label: "✓✓ read", title: "읽음"},
   processing: {bg: "#7a5a00", label: "⏳ processing", title: "처리 중"},
   done: {bg: "#238636", label: "✓ done", title: "처리 완료"},
   failed: {bg: "#a02828", label: "✗ failed", title: "실패"},
   // rc.219 outbound_queue ACK envelope status (transport-level)
   pending: {bg: "#5a5a5a", label: "⏳ pending", title: "미전송"},
   inbox_stored: {bg: "#3a8a3a", label: "✓ stored", title: "receiver inbox 저장 완료 (tmux 매칭 X)"},
   tmux_injected: {bg: "#238636", label: "✓✓ delivered", title: "receiver tmux 화면 inject 완료"},
   both: {bg: "#238636", label: "✓✓ delivered", title: "receiver tmux 화면 inject 완료"},
   fail: {bg: "#a02828", label: "✗ failed", title: "ACK 실패"},
   ack_timeout_max: {bg: "#a02828", label: "✗ ack-timeout", title: "ACK 30분 + 3회 재발송 후 실패"},
  };
  const v = map[s];
  if (v) {
   const via = m.ack_via ? ` · ${m.ack_via}` : "";
   transportEl = (
    <span title={`${v.title}${via}${m.acked_at ? " · " + m.acked_at.slice(0, 19) : ""}`}
     style={`margin-left:6px; padding:1px 5px; background:${v.bg}; color:#fff; border-radius:3px; font-size:9px; font-weight:bold;`}>
     {v.label}
    </span>
   );
  }
 }

 // rc.227 — application-level ACK badge.
 // app_ack_status: 'processed' = LLM 답신 도착, 'blocked' = 5분 timeout (응답 없음).
 // undefined + ack_status set = 대기 중 (transport OK, 답신 대기).
 let appAckEl: any = null;
 const isOutboundLike = (m.ack_status || "") !== "" && m.ack_status !== "delivered_inbound";
 if (isOutboundLike) {
  if (m.app_ack_status === "processed") {
   appAckEl = (
    <span title={`receiver 답신 도착 (app_ack_at=${(m.app_ack_at || "").slice(0, 19)})`}
     style="margin-left:4px; padding:1px 5px; background:#1f6f3a; color:#fff; border-radius:3px; font-size:9px; font-weight:bold;">
     ✓✓ processed
    </span>
   );
  } else if (m.app_ack_status === "blocked") {
   appAckEl = (
    <span title="5분 안에 receiver 답신 없음 — LLM 처리 안 됨 (survey prompt / context full / 비활성 등)"
     style="margin-left:4px; padding:1px 5px; background:#a02828; color:#fff; border-radius:3px; font-size:9px; font-weight:bold;">
     ⚠ no-reply
    </span>
   );
  } else if (transportEl) {
   // transport OK + app_ack 아직 미수신 → 대기 중 표시.
   appAckEl = (
    <span title="receiver 답신 대기 중 (5분 timeout)"
     style="margin-left:4px; padding:1px 5px; background:#7a5a00; color:#fff; border-radius:3px; font-size:9px; font-weight:bold;">
     ⏱ awaiting-reply
    </span>
   );
  }
 }

 if (!transportEl && !appAckEl) return null;
 return (
  <>
   {transportEl}
   {appAckEl}
  </>
 );
}

async function fetchMessages(): Promise<MessageDto[]> {
 try {
 return await invoke<MessageDto[]>("messages_recent", { limit: 100});
} catch {
 return [];
}
}

// rc.212 — peer 와의 통합 conversation. backend /v1/gui/peer_conversation/{alias} 가
// outbox-to-/inbox-from-/Peer ·/Claude Code · {alias|variant} 모든 session 합쳐서 timestamp ASC 반환.
async function fetchPeerConversation(alias: string): Promise<MessageDto[]> {
 if (!alias) return [];
 try {
 return await invoke<MessageDto[]>("peer_conversation", { alias, limit: 500});
} catch {
 return [];
}
}

// rc.212 — sender label → "내(마스터/sender)" 측 인지 분류.
// self:* / me / me:* / user → 오른쪽 (마스터 발신)
// peer:* / assistant / unverified:* / 그 외 → 왼쪽 (상대/LLM)
function isSelfSender(sender: string): boolean {
 const s = (sender || "").toLowerCase();
 if (s === "me" || s === "user") return true;
 if (s.startsWith("self:") || s.startsWith("me:")) return true;
 return false;
}

function fmtTime(iso: string): string {
 // ISO 8601 → 'MM-dd HH:mm' (KST). 실패 시 원문.
 try {
 const d = new Date(iso);
 return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
} catch {
 return iso;
}
}

interface PeerDto {
 alias: string;
 address: string;
 public_key_hex: string;
 machine?: string;
 last_seen?: string;
 // rc.92 D2 — agent_capabilities JOIN
 description?: string | null;
 capabilities?: string[];
 // rc.214 — agent list 한눈 view: role 필드 (peers.role enum: primary/worker/...)
 role?: string | null;
 // rc.226 — peer entity = 1 project folder = 1 tmux session = 1 LLM 의 본질 inline
 project_folder?: string | null;
 llm_type?: string | null;
 llm_version?: string | null;
 // rc.228 — peer sub-resources (3-level tree).
 worktrees?: { path: string; branch?: string | null}[];
 subagents?: { name: string; path: string; kind: string}[];
 ex_peers?: { alias: string; msg_count: number; last_msg_at?: string | null}[];
}

interface NotifyStatusDto {
 telegram_configured: boolean;
 discord_configured: boolean;
 discord_webhook_configured: boolean;
}

type FriendKind = "peer" | "discord" | "telegram" | "tmux" | "claude_project" | "xgram_session";

interface Friend {
 kind: FriendKind;
 id: string; // peer.alias 또는 "discord" / "telegram" / "tmux:<name>" 등
 display: string; // 화면에 보일 이름
 subtitle: string; // 화면 보조 (주소·last_seen·"connected" 등)
 meta?: PeerDto; // peer일 경우 원본 데이터
 sessionMeta?: DetectedSession; // session 일 경우
 machineTag?: string; // rc.230 — flat list 의 inline 머신 태그 (group header 아님, 그냥 tag)
}

// rc.230 — 머신 group header 폐기. 머신은 카테고리가 아니라 row 의 inline 태그.
// address IP → 머신명 매핑. peer.address(또는 last_seen endpoint) 의 IP 로 판별.
// session(로컬) 은 server-seoul. 매핑 안 되면 undefined → 태그 생략.
function machineFromAddress(addr?: string | null): string | undefined {
 if (!addr) return undefined;
 if (addr.includes("100.101.237.9")) return "server-seoul";
 if (addr.includes("100.87.11.8")) return "zalman";
 if (addr.includes("100.80.35.17")) return "zalman";
 return undefined;
}

// v1.3 §3.2 — /v1/gui/sessions 응답 (M-1 통합 detector).
interface MachineInfoDto {
 hostname: string;
 alias: string;
 tailscale_ip: string | null;
}
interface DetectedSession {
 kind: "tmux" | "claude_project" | "xgram_session";
 identifier: string;
 display: string;
 status: "active" | "attached" | "detached" | "stale";
 windows: number | null;
 attached: boolean | null;
 created_at: string | null;
 last_active_at: string | null;
 agent_id: string | null;
}
interface SessionsDto {
 machine: MachineInfoDto;
 sessions: DetectedSession[];
}
async function fetchSessions(): Promise<SessionsDto | null> {
 try {
 return await invoke<SessionsDto>("sessions");
} catch {
 return null;
}
}

async function fetchPeers(): Promise<PeerDto[]> {
 try {
 return await invoke<PeerDto[]>("peers_list");
} catch {
 return [];
}
}

async function fetchNotifyStatus(): Promise<NotifyStatusDto> {
 try {
 return await invoke<NotifyStatusDto>("notify_status");
} catch {
 return {
 telegram_configured: false,
 discord_configured: false,
 discord_webhook_configured: false,
};
}
}

function fingerprint(pubkeyHex: string): string {
 const trimmed = pubkeyHex.replace(/^0x/, "");
 if (trimmed.length < 16) return trimmed;
 return `${trimmed.slice(0, 8)}…${trimmed.slice(-8)}`;
}

// rc.231 — entity dedup key. session display "akashic" 와 peer alias
// "aoe_akashic_5054a80a" 가 같은 entity (포털 작동중 터미널 = 하나의 peer) 이므로
// 둘을 같은 정규화 key 로 묶어 flat list 중복 0 으로 만든다.
// 매칭 규칙: [machine] prefix + aoe_/term_ prefix 제거 + _<hexid> suffix 제거 → 핵심 이름만.
//   "aoe_akashic_5054a80a"    → "akashic"
//   "akashic"                 → "akashic"
//   "term_teeup"              → "teeup"
//   "aoe_zalman-wsl_7f27e90b" → "zalman-wsl"
// 과매칭 방지:
//   - prefix/suffix 만 떼고 가운데 이름은 그대로 유지 (다른 프로젝트 병합 X).
//   - hexid suffix 는 8자 이상 16진수만 (짧은 숫자·일반 단어 보호).
//   - sv_aoe_<숫자>_<13자리 timestamp> 류 service peer 는 핵심 이름이 숫자라
//     어떤 터미널과도 안 묶임 → orphan row 로 유지 (의도된 동작).
function normalizeAlias(raw: string): string {
 let s = (raw || "").trim().toLowerCase();
 s = s.replace(/^\[[^\]]+\]\s*/, "");      // [zalman] prefix
 s = s.replace(/^(?:aoe|term)[_-]/, "");    // aoe_ / term_ prefix
 s = s.replace(/[_-][0-9a-f]{8,}$/i, "");   // _<hexid> suffix (8+ hex digits)
 return s.trim();
}

type SortMode = "name" | "activity";
type ConnFilter = "all" | "connected" | "offline";
type LeftMode = "agent" | "thread" | "workflow";

interface MachineGroup {
 machine: string;
 friends: Friend[];
 connected: number;
}

// 스레드 = 같은 conversation_id 의 메시지 묶음 (Tier 2 — client-side grouping).
// daemon side ThreadStore 는 별 단계 — 지금은 messages_recent 의 conversation_id 활용.
interface ThreadSummary {
 conversation_id: string;
 participants: string[]; // unique senders
 message_count: number;
 last_at: string; // ISO
 last_body: string; // 미리보기
}

const UNKNOWN_MACHINE = "(unknown)";
const UNKNOWN_CONV = "_no_conversation_";
// rc.230 — flat list sentinel: 이 machine 키의 group 은 머신 헤더 없이 렌더.
const FLAT_GROUP = "__flat__";

export function Messenger(props: { onJumpToSettings?: () => void} = {}) {
 const { t} = useI18n();
 // rc.228 — refetchPeers 노출 (ex_peer 삭제 후 peer 목록 즉시 갱신).
 const [peers, { refetch: refetchPeers}] = createResource(fetchPeers);
 const [notifyStatus] = createResource(fetchNotifyStatus);
 // v1.3 §3.2 — 이 머신의 tmux + Claude Code projects + xgram sessions 통합.
 const [sessions, { refetch: refetchSessions}] = createResource(fetchSessions);
 // rc.142 — 메신저 등록된 에이전트 set (agent_capabilities.messenger_enabled=1).
 // 친구 옆 ✓ 배지로 사용자에게 등록 여부 즉시 표시.
 const [registeredAgents] = createResource<Set<string>>(async () => {
 try {
 const list = await invoke<Array<{alias: string; messenger_enabled: boolean}>>("agents_list");
 return new Set(list.filter((a) => a.messenger_enabled).map((a) => a.alias));
 } catch { return new Set(); }
 });

 // rc.170 — auto-echo enforcer visual: 각 binding 의 매칭 상태 (no_match/first_setup/pending_echo/up_to_date).
 // friend row 옆 chip 으로 표시 → 마스터가 GUI 에서 매칭 정상 여부 직접 확인.
 interface BindingStatus {
   agent_id: string;
   platform: string;
   channel_ref: string;
   bot_alias?: string | null;
   bot_label?: string | null;
   matched_session_count: number;
   latest_preview?: string | null;
   last_echoed_ulid?: string | null;
   would_echo: boolean;
   match_status: "no_match" | "no_assistant_messages" | "first_setup" | "pending_echo" | "up_to_date";
 }
 const [bindingsByAgent] = createResource<Map<string, BindingStatus[]>>(async () => {
   try {
     const resp = await invoke<{bindings: BindingStatus[]}>("bindings_status");
     const map = new Map<string, BindingStatus[]>();
     for (const b of resp.bindings || []) {
       const arr = map.get(b.agent_id) || [];
       arr.push(b);
       map.set(b.agent_id, arr);
     }
     return map;
   } catch { return new Map(); }
 });
 const [selected, setSelected] = createSignal<string | null>(null); // friend id (에이전트 모드)
 const [selectedThread, setSelectedThread] = createSignal<string | null>(null); // conversation_id
 const [leftMode, setLeftMode] = createSignal<LeftMode>("agent"); // L1
 // 컬럼 너비 — drag 로 조절, localStorage 영구
 const initialSidebar = (() => { const v = parseInt(localStorage.getItem("messenger.sidebar_w") || "240"); return isNaN(v) ? 240 : v; })();
 const initialSidepanel = (() => { const v = parseInt(localStorage.getItem("messenger.sidepanel_w") || "320"); return isNaN(v) ? 320 : v; })();
 const [sidebarW, setSidebarW] = createSignal(initialSidebar);
 const [sidepanelW, setSidepanelW] = createSignal(initialSidepanel);
 // rc.233 — 설정 패널(우측 sidepanel) 토글. 기본 숨김(2단) → 설정 버튼 클릭 시 3단.
 const [showSettings, setShowSettings] = createSignal(localStorage.getItem("messenger.show_settings") === "1");
 function toggleSettings() {
   const v = !showSettings();
   setShowSettings(v);
   localStorage.setItem("messenger.show_settings", v ? "1" : "0");
 }
 function startResize(which: "left" | "right", e: MouseEvent) {
 e.preventDefault();
 const startX = e.clientX;
 const startSidebar = sidebarW();
 const startSidepanel = sidepanelW();
 const onMove = (ev: MouseEvent) => {
 const dx = ev.clientX - startX;
 if (which === "left") {
 const w = Math.min(500, Math.max(160, startSidebar + dx));
 setSidebarW(w);
 } else {
 const w = Math.min(800, Math.max(160, startSidepanel - dx));
 setSidepanelW(w);
 }
 };
 const onUp = () => {
 window.removeEventListener("mousemove", onMove);
 window.removeEventListener("mouseup", onUp);
 localStorage.setItem("messenger.sidebar_w", String(sidebarW()));
 localStorage.setItem("messenger.sidepanel_w", String(sidepanelW()));
 };
 window.addEventListener("mousemove", onMove);
 window.addEventListener("mouseup", onUp);
 }
 // L5 — Hand-off 모달
 const [handoffSource, setHandoffSource] = createSignal<MessageDto | null>(null);
 const [showRouting, setShowRouting] = createSignal(false); // V11
 const [showWhitelist, setShowWhitelist] = createSignal(false); // M-5
 const [messages, { refetch: refetchMessages}] = createResource(fetchMessages);

 // 좌측 컨트롤
 const [sortMode, setSortMode] = createSignal<SortMode>("activity");
 const [connFilter, setConnFilter] = createSignal<ConnFilter>("all");
 const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({});
 // rc.228 — peer card 의 3 sub-section (worktree / subagents / ex_peers) expand 상태.
 //   key = `${peer.alias}::${section}` (section: "card"|"worktrees"|"subagents"|"ex_peers").
 //   default: card collapsed (▶), 각 section 도 collapsed.
 const [peerExpand, setPeerExpand] = createSignal<Record<string, boolean>>({});
 function togglePeerExpand(key: string) {
   setPeerExpand((prev) => ({ ...prev, [key]: !prev[key]}));
 }
 // rc.229 fix#3 — session row (터미널 group) on-demand detail.
 //   화면이 보여주는 건 sessions() 인데 rc.226/228 enrich 는 peers() 에 붙어 안 보였음.
 //   session row expand 시 그 alias 로 agent_detail 호출 → 4-metadata + worktree/subagent/ex_peer tree.
 //   key = session alias. 값: undefined(미요청) | "loading" | AgentDetail | "error".
 const [agentDetails, setAgentDetails] = createSignal<Record<string, unknown>>({});
 async function loadAgentDetail(alias: string) {
   if (!alias) return;
   const cur = agentDetails()[alias];
   if (cur === "loading" || (cur && typeof cur === "object")) return; // 이미 로딩/완료
   setAgentDetails((prev) => ({ ...prev, [alias]: "loading"}));
   try {
     const d = await invoke<Record<string, unknown>>("agent_detail", { alias});
     setAgentDetails((prev) => ({ ...prev, [alias]: d}));
   } catch {
     setAgentDetails((prev) => ({ ...prev, [alias]: "error"}));
   }
 }
 // rc.228 — ex Peer thread 삭제 핸들러. confirm dialog 후 DELETE 호출 + refetch.
 async function deleteExPeer(selfAlias: string, otherAlias: string) {
   const ok = window.confirm(
     `정말 '${otherAlias}' 와의 대화 thread (outbox/inbox + messages + outbound_queue) 를 삭제할까요?\n되돌릴 수 없습니다.`
   );
   if (!ok) return;
   try {
     await invoke("ex_peer_delete", { self_alias: selfAlias, other_alias: otherAlias});
     // peer 목록 새로고침 (ex_peers 갱신).
     void refetchPeers();
   } catch (e) {
     window.alert(`ex Peer 삭제 실패: ${e}`);
   }
 }

 // 3초 간격 메시지 폴링 — 활동 흐름 모니터링.
 const pollTimer = setInterval(() => {
 void refetchMessages();
}, 3000);
 // 10초 간격 세션 폴링 — 새 tmux/Claude 세션 자동 감지
 const sessionsPollTimer = setInterval(() => {
 void refetchSessions();
}, 10000);
 onCleanup(() => { clearInterval(pollTimer); clearInterval(sessionsPollTimer);});

 function toggleCollapse(machine: string) {
 setCollapsed((prev) => ({ ...prev, [machine]: !prev[machine]}));
}

 function isConnected(p: PeerDto): boolean {
 // last_seen 이 1시간 이내면 연결, 아니면 offline 으로 간주.
 if (!p.last_seen) return false;
 const ts = Date.parse(p.last_seen);
 if (Number.isNaN(ts)) return false;
 return Date.now() - ts < 60 * 60 * 1000;
}

 // rc.230 — 머신 group header 폐기. peer + session 을 단일 flat list 로 병합.
 //   머신은 카테고리가 아니라 row 의 inline 태그 (machineTag). 채널만 별 group 유지.
 //   FLAT_GROUP = 머신 헤더 없이 렌더되는 sentinel machine 키.
 const groups = createMemo<MachineGroup[]>(() => {
 // alias → Friend 로 dedup (session 우선, peer 가 같은 alias 면 session 의 meta 보강).
 const byAlias = new Map<string, Friend>();

 // (1) 로컬 sessions (포털 작동중 터미널) — flat list 의 핵심.
 const sess = sessions();
 if (sess) {
 const localIp = sess.machine.tailscale_ip || "";
 const localMachine = machineFromAddress(localIp) || sess.machine.alias || sess.machine.hostname || "server-seoul";
 // rc.141 — portal:/aoe: 가 같은 tmux session 가리키면 portal: 만 유지 (중복 제거).
 const tmuxNameRe = /aoe_[a-z0-9_-]+/i;
 const portalTmux = new Set<string>();
 for (const s of sess.sessions) {
 const m = s.identifier.match(tmuxNameRe);
 if (m && /(?:^|:)portal:/.test(s.identifier)) portalTmux.add(m[0]);
 }
 const dedup = sess.sessions.filter((s) => {
 const m = s.identifier.match(tmuxNameRe);
 if (m && /(?:^|:)aoe:/.test(s.identifier) && portalTmux.has(m[0])) return false;
 return true;
 });
 for (const s of dedup) {
 if (s.kind === "claude_project") continue; // rc.139 — claude_project 숨김 유지
 const conn = s.status === "attached" || s.status === "active";
 if (connFilter() === "connected" && !conn) continue;
 if (connFilter() === "offline" && conn) continue;
 // identifier "peer:<alias>:..." 면 원격 머신, 아니면 local.
 let machineTag = localMachine;
 if (s.identifier.startsWith("peer:")) {
 const parts = s.identifier.split(":");
 if (parts.length >= 2) machineTag = machineFromAddress(parts[1]) || parts[1];
 }
 // rc.231 — dedup key = 정규화된 entity 이름. peer alias 와 collide 시키려고
 // aoe_/portal/tmux name 또는 display 를 normalizeAlias 로 통일.
 const aoeM = s.identifier.match(/aoe_[a-zA-Z0-9_-]+/);
 const portalM = s.identifier.match(/(?:^|:)portal:([^:]+)/);
 const tmuxM = s.identifier.match(/(?:^|:)tmux:([^:]+)/);
 const dispClean = s.display.replace(/^\[[^\]]+\]\s*/, "");
 const rawKey = aoeM ? aoeM[0] : (portalM ? portalM[1] : (tmuxM ? tmuxM[1] : dispClean));
 const key = normalizeAlias(rawKey) || rawKey;
 byAlias.set(key, {
 kind: s.kind as FriendKind,
 id: `session:${s.identifier}`,
 display: dispClean, // [zalman] prefix 제거
 subtitle: (() => {
   const ts = s.last_active_at;
   if (!ts) return "";
   const d = new Date(ts);
   if (isNaN(d.getTime())) return "";
   return `최근 활동: ${d.toLocaleString()}`;
 })(),
 sessionMeta: s,
 machineTag,
 });
 }
}

 // (2) peers (원격 포함) — 같은 alias session 이 없으면 flat list 에 합류.
 //   현재 머신 group 으로 흩어져 화면에서 사라진 zalman peer 들을 머신 태그로 표시.
 for (const p of peers() ?? []) {
 if (connFilter() === "connected" && !isConnected(p)) continue;
 if (connFilter() === "offline" && isConnected(p)) continue;
 const machineTag = machineFromAddress(p.address) || p.machine?.trim() || undefined;
 const friend: Friend = {
 kind: "peer",
 id: `peer:${p.alias}`,
 display: p.alias,
 subtitle:
 `${(p.address || "").slice(0, 10)} · ${fingerprint(p.public_key_hex)}` +
 (p.last_seen ? ` · ${p.last_seen}` : ""),
 meta: p,
 machineTag,
};
 // rc.231 — 정규화 key 로 session row 와 같은 entity 인지 판정.
 //   같은 entity → 로컬 session 이 진짜 entity 이므로 session row 로 흡수
 //   (peer 의 address/machine 태그만 병합, peer row 따로 안 보임).
 //   매칭 session 없으면 (원격 zalman peer 등) peer row 로 추가.
 const nkey = normalizeAlias(p.alias) || p.alias;
 const existing = byAlias.get(nkey);
 if (existing) {
 // session row 에 peer 메타 병합 (address/machine 태그 + on-demand detail 용 meta).
 if (!existing.meta) existing.meta = p;
 if (!existing.machineTag && machineTag) existing.machineTag = machineTag;
 } else {
 byAlias.set(nkey, friend);
 }
}

 // 정렬
 const sorter = (a: Friend, b: Friend) => {
 if (sortMode() === "name") return a.display.localeCompare(b.display);
 // activity: meta.last_seen / sessionMeta.last_active_at DESC.
 const aT = a.meta?.last_seen ? Date.parse(a.meta.last_seen)
   : (a.sessionMeta?.last_active_at ? Date.parse(a.sessionMeta.last_active_at) : 0);
 const bT = b.meta?.last_seen ? Date.parse(b.meta.last_seen)
   : (b.sessionMeta?.last_active_at ? Date.parse(b.sessionMeta.last_active_at) : 0);
 return (isNaN(bT) ? 0 : bT) - (isNaN(aT) ? 0 : aT);
};
 const flat = Array.from(byAlias.values()).sort(sorter);

 const out: MachineGroup[] = [];
 // 단일 flat group — 머신 헤더 없이 렌더 (machine === FLAT_GROUP).
 out.push({
 machine: FLAT_GROUP,
 friends: flat,
 connected: flat.filter((f) => f.meta && isConnected(f.meta)).length,
 });

 // 채널 그룹
 const ns = notifyStatus();
 if (ns) {
 const channels: Friend[] = [];
 channels.push({
 kind: "discord",
 id: "channel:discord",
 display: "Discord",
 subtitle: ns.discord_configured
 ? t("messenger.connected") || "connected"
 : t("messenger.add-bot") || "add bot →",
});
 channels.push({
 kind: "telegram",
 id: "channel:telegram",
 display: "Telegram",
 subtitle: ns.telegram_configured
 ? t("messenger.connected") || "connected"
 : t("messenger.add-bot") || "add bot →",
});
 if (channels.length > 0) {
 out.push({
 machine: " 채널",
 friends: channels,
 connected:
 (ns.discord_configured ? 1 : 0) + (ns.telegram_configured ? 1 : 0),
});
}
}

 return out;
});

 const friends = createMemo<Friend[]>(() =>
 groups().flatMap((g) => g.friends),
);

 // 직전 selectedFriend 캐시 — sessions/peers 폴링 시 일시적으로 friends 가 비었을 때
 // sidepanel 이 사라지는 깜빡임을 막기 위함.
 let lastSelectedFriend: Friend | null = null;
 const selectedFriend = createMemo(() => {
 const id = selected();
 if (!id) { lastSelectedFriend = null; return null;}
 const found = friends().find((f) => f.id === id);
 if (found) { lastSelectedFriend = found; return found;}
 // 일시적으로 못 찾으면 캐시 사용 (sessions polling 중)
 if (lastSelectedFriend && lastSelectedFriend.id === id) return lastSelectedFriend;
 return null;
});

 // 스레드 모드 (L1) — messages 를 conversation_id 별 그룹화
 const threads = createMemo<ThreadSummary[]>(() => {
 const all = messages() ?? [];
 const map = new Map<string, MessageDto[]>();
 for (const m of all) {
 const cid = m.conversation_id || UNKNOWN_CONV;
 if (!map.has(cid)) map.set(cid, []);
 map.get(cid)!.push(m);
}
 const list: ThreadSummary[] = [];
 for (const [cid, msgs] of map.entries()) {
 msgs.sort((a, b) => Date.parse(b.timestamp) - Date.parse(a.timestamp));
 const participants = Array.from(new Set(msgs.map((m) => m.sender)));
 list.push({
 conversation_id: cid,
 participants,
 message_count: msgs.length,
 last_at: msgs[0].timestamp,
 last_body: msgs[0].body.slice(0, 60),
});
}
 // 최근 활동순
 list.sort((a, b) => Date.parse(b.last_at) - Date.parse(a.last_at));
 return list;
});

 // rc.233 — sidepanel(상세/설정) 은 설정 버튼 토글(showSettings) 시만. 기본 2단.
 const hasSidepanel = () =>
 showSettings() &&
 leftMode() === "agent" &&
 !!selectedFriend() &&
 (selectedFriend()!.kind === "peer" ||
 selectedFriend()!.kind === "tmux" ||
 selectedFriend()!.kind === "claude_project" ||
 selectedFriend()!.kind === "xgram_session");
 const gridCols = () => hasSidepanel()
 ? `${sidebarW()}px 5px minmax(0, 1fr) 5px ${sidepanelW()}px`
 : `${sidebarW()}px 5px minmax(0, 1fr)`;
 return (
 <div
 class="messenger-shell"
 style={{ "grid-template-columns": gridCols()}}
 >
 {/* 좌: 머신×세션 트리 + 스레드 모드 (Tier 1 + L1) */}
 <aside class="messenger-sidebar">
 {/* L1 — 좌측 상단 3 모드 탭 */}
 <div class="messenger-sidebar-mode" style="display:flex; gap:4px; padding:6px 8px; border-bottom:1px solid var(--border);">
 <button
 type="button"
 class={leftMode() === "agent" ? "active" : ""}
 onClick={() => setLeftMode("agent")}
 style="flex:1;"
 >
 에이전트
 </button>
 <button
 type="button"
 class={leftMode() === "thread" ? "active" : ""}
 onClick={() => setLeftMode("thread")}
 style="flex:1;"
 title={`스레드 — 같은 conversation_id 의 메시지 묶음 (대화 단위). 에이전트 모드(누가)와 다름. 메시지 송수신 시작하면 자동 생성. 현재 ${threads().length}건`}
 >
 스레드·{threads().length}
 </button>
 <button
 type="button"
 class={leftMode() === "workflow" ? "active" : ""}
 onClick={() => setLeftMode("workflow")}
 style="flex:1;"
 title="오케스트레이션 — W-1~W-10 워크플로 정의·실행·조율 (web_search·llm_call·email step). 사양 UI-MESSENGER-SPEC v1.4 §20"
 >
 오케스트레이션
 </button>
 </div>
 {/* L1b — 액션 (RoutingRule + Whitelist) 2 버튼 */}
 <div class="messenger-sidebar-actions" style="display:flex; gap:4px; padding:0 8px 6px; border-bottom:1px solid var(--border);">
 <button
 type="button"
 onClick={() => setShowRouting(true)}
 title="RoutingRule (V11) — 에이전트↔에이전트 라우팅"
 style="flex:1; padding:6px 8px; white-space:nowrap; font-size:12px;"
 >
 라우팅
 </button>
 <button
 type="button"
 onClick={() => setShowWhitelist(true)}
 title="화이트리스트 (M-5) — 자동 등록 패턴"
 style="flex:1; padding:6px 8px; white-space:nowrap; font-size:12px;"
 >
 허용
 </button>
 </div>
 <Show when={showRouting()}>
 <RoutingRulesModal onClose={() => setShowRouting(false)} />
 </Show>
 <Show when={showWhitelist()}>
 <WhitelistModal onClose={() => setShowWhitelist(false)} />
 </Show>

 <header class="messenger-sidebar-head" style="padding:10px 14px;">
 <strong style="font-size:13px;">
 {leftMode() === "agent" ? " 에이전트·봇" : " 스레드"}
 </strong>
 <Show when={leftMode() === "agent"}>
 <span style="display:flex; gap:6px; align-items:center;">
 {/* rc.233 — 설정(⚙) 토글: 우측 상세/설정 패널(3단) on/off. 모든 친구 종류에서 작동. */}
 <button
 type="button"
 onClick={toggleSettings}
 title={showSettings() ? "설정 패널 닫기 (2단)" : "설정 패널 열기 (3단)"}
 style={`padding:4px 9px; border:1px solid var(--border); border-radius:4px; cursor:pointer; font-size:13px; background:${showSettings() ? "rgba(58, 130, 246, 0.25)" : "var(--surface-2)"}; color:var(--text-1);`}
 >
 ⚙ 설정
 </button>
 <button
 type="button"
 class="messenger-add-btn"
 title="peer 등록 / 봇 연결"
 style="padding:4px 10px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; cursor:pointer; font-size:12px;"
 onClick={() => alert("peer 등록: peer add CLI 또는 채널 카드 → 봇 등록")}
 >
 + 추가
 </button>
 </span>
 </Show>
 </header>

 {/* 정렬·필터 컨트롤 (S4) — 에이전트 모드만 */}
 <Show when={leftMode() === "agent"}>
 <div class="messenger-sidebar-ctrl" style="display:flex; gap:6px; padding:6px 8px; font-size:0.85em;">
 <select
 value={sortMode()}
 onChange={(e) => setSortMode(e.currentTarget.value as SortMode)}
 title="정렬"
 >
 <option value="activity">활동순</option>
 <option value="name">이름순</option>
 </select>
 <select
 value={connFilter()}
 onChange={(e) => setConnFilter(e.currentTarget.value as ConnFilter)}
 title="연결 필터"
 >
 <option value="all">전체</option>
 <option value="connected">연결만</option>
 <option value="offline">offline만</option>
 </select>
 </div>

 <div class="messenger-friend-list">
 <For each={groups()}>
 {(g) => {
 const isFlat = g.machine === FLAT_GROUP;
 const isCollapsed = () => !isFlat && collapsed()[g.machine] === true;
 return (
 <div class="messenger-machine-group">
 {/* rc.230 — 채널 group 만 헤더. flat list(peer/터미널) 는 머신 헤더 없음. */}
 <Show when={!isFlat}>
 <div
 class="messenger-machine-header"
 style="cursor:pointer; display:flex; align-items:center; gap:6px;"
 >
 <span onClick={() => toggleCollapse(g.machine)} style="flex:1;">
 <span style="margin-right:4px;">
 {isCollapsed() ? "▶" : "▼"}
 </span>
 {(g.machine || "(unknown)").replace(/\.c\.[a-z0-9-]+\.internal$/, " (GCP)").replace(/\.tail[a-z0-9]+\.ts\.net$/, " (Tailscale)")}{" "}
 <span style="font-weight:400; color:var(--text-3); font-size:12px;">
 ({g.friends.length})
 </span>
 </span>
 <button class="link-btn" title="sessions 즉시 새로고침 (자동 10초)"
 style="font-size:11px; padding:2px 6px;"
 onClick={(e) => { e.stopPropagation(); refetchSessions();}}>↻</button>
 </div>
 </Show>
 <Show when={!isCollapsed()}>
 {(() => {
 // rc.230 — sub-section 폐기. flat group 은 머신 헤더·sub-header 없이 단일 ul.
 //   claude_project 는 backend 단계에서 이미 제외됨 (groups() 에서 skip).
 const items = g.friends;
 return (
 <For each={[{ items}]}>
 {(sub) => {
 return (
 <div>
 <ul style="margin:0; padding:0;">
 <For each={sub.items}>
 {(f) => (
 <li
 class={selected() === f.id ? "messenger-friend selected" : "messenger-friend"}
 onClick={() => setSelected(f.id)}
 >
 {(() => {
 // 종류별 아이콘 + 색
 const kindInfo: Record<string, { icon: string; color: string; bg: string; label: string}> = {
 peer: { icon: "P", color: "#fff", bg: "#7b61ff", label: "Peer"},
 tmux: { icon: "$", color: "#fff", bg: "#d4a017", label: "tmux"},
 claude_project: { icon: "C", color: "#fff", bg: "#06c", label: "Claude Code"},
 xgram_session: { icon: "X", color: "#fff", bg: "#5a9", label: "xgram"},
 discord: { icon: "D", color: "#fff", bg: "#5865F2", label: "Discord"},
 telegram: { icon: "t", color: "#fff", bg: "#26A5E4", label: "Telegram"},
 };
 const info = kindInfo[f.kind] || { icon: "?", color: "#fff", bg: "#555", label: f.kind};
 // 상태별 dot 색
 let dotColor = "#666"; let dotTitle = "비활성";
 if (f.kind === "tmux") {
 // rc.148 — portal AoE activity_state 기반. active = LLM 작업 중, waiting = 입력 대기
 if (f.sessionMeta?.status === "active") { dotColor = "#4caf50"; dotTitle = "active (에이전트 작업 중)";}
 else { dotColor = "#d4a017"; dotTitle = "waiting (대기 — 사용자 입력 기다림)";}
 } else if (f.kind === "claude_project") {
 const la = f.sessionMeta?.last_active_at;
 if (la && (Date.now() - new Date(la).getTime()) < 86400_000) {
 dotColor = "#06c"; dotTitle = "최근 24h 활동";
 } else { dotColor = "#666"; dotTitle = "오래된 활동";}
 } else if (f.kind === "peer") {
 const ls = f.meta?.last_seen;
 if (ls && (Date.now() - new Date(ls).getTime()) < 3600_000) {
 dotColor = "#4caf50"; dotTitle = "최근 1h 연결";
 }
 } else if (f.kind === "discord" || f.kind === "telegram") {
 dotColor = "#06c"; dotTitle = "채널 (항상 enable)";
 }
 // rc.233 — P/$ 색배지 아이콘 제거. status dot 1개 + 작은 kind 글리프(muted)로 정리.
 const isConn = dotColor !== "#666";
 return (
 <>
 <span
 title={`${dotTitle}${isConn ? "" : " · ○ 미연결"}`}
 style={`display:inline-block; width:9px; height:9px; border-radius:50%; margin-right:7px; flex:none; background:${dotColor};`}
 />
 <span class="messenger-friend-text">
 <span class="messenger-friend-name" style="font-size:14px;" title={info.label}>
 {f.display}
 {(() => {
 // rc.162 — peer 친구도 display name 으로 명시 등록 시 ✓ MSG 표시.
 // 사용자가 우측 패널에서 peer 친구 등록하면 agent_capabilities.alias=display
 // (예: zalman-wsl) row 생김 → 사이드바 매칭 가능.
 const set = registeredAgents();
 const id = f.id || "";
 const isPeer = /(?:^|:)peer:/.test(id);
 if (isPeer) {
 // peer 친구는 display name 매칭만 (false positive 줄임)
 if (set && f.display && set.has(f.display)) {
 return (
 <span title="메신저 등록됨 (peer) — 다른 peer 의 list_peers 에 노출"
 style="display:inline-block; width:6px; height:6px; border-radius:50%; margin-left:6px; background:#238636;" />
 );
 }
 return null;
 }
 let name: string | null = null;
 const aoeM = id.match(/aoe_[a-zA-Z0-9_-]+/);
 if (aoeM) name = aoeM[0];
 else {
 const portalM = id.match(/(?:^|:)portal:([^:]+)/);
 if (portalM) name = portalM[1];
 else {
 const tmuxM = id.match(/(?:^|:)tmux:([^:]+)/);
 if (tmuxM) name = tmuxM[1];
 }
 }
 const reg = set && name && set.has(name);
 return reg ? (
 <span title="메신저 등록됨 — 다른 peer 의 list_peers 에 노출"
 style="display:inline-block; width:6px; height:6px; border-radius:50%; margin-left:6px; background:#238636;" />
 ) : null;
 })()}
 {(() => {
  // rc.170 — auto-echo enforcer visual chip. friend.display 로 binding 매칭.
  const map = bindingsByAgent();
  if (!map || !f.display) return null;
  const matches = map.get(f.display);
  if (!matches || matches.length === 0) return null;
  return matches.map((b) => {
    // rc.170+: transient(first_setup) / edge(no_assistant_messages) 는 chip 숨김 — 마스터 무관심.
    if (b.match_status === "first_setup" || b.match_status === "no_assistant_messages") return null;
    // rc.233 polish — 정상(up_to_date) 도 chip 숨김. fix 필요(no_match)·발송예정(pending) 만 노출.
    if (b.match_status === "up_to_date") return null;
    const colorByStatus: Record<string, string> = {
      "no_match": "#b00020",        // ✗ 빨강 — fix 필요
      "pending_echo": "#3a82f6",     // → 파랑 — 60초 안 Discord 발송 예정
      "up_to_date": "#238636",       // ✓ 초록 — 정상 (모두 echo 완료)
    };
    const platformIcon = b.platform === "discord" ? "D" : b.platform === "telegram" ? "T" : "X";
    const statusLabel: Record<string, string> = {
      "no_match": "매칭 X — fix 필요",
      "pending_echo": "60s 안 Discord 발송 예정",
      "up_to_date": "정상 (모두 echo 완료)",
    };
    const title = `${b.platform} -> bot=${b.bot_alias || b.bot_label || "?"} ch=${b.channel_ref.slice(0, 10)}\n매칭 세션: ${b.matched_session_count}\n${statusLabel[b.match_status] || b.match_status}` + (b.latest_preview ? `\n최근: ${b.latest_preview.slice(0, 80)}` : "");
    const bg = colorByStatus[b.match_status] || "#6a737d";
    const txt = b.match_status === "no_match" ? "✗" : b.match_status === "pending_echo" ? "→" : "✓";
    return (
      <span title={title} style={`margin-left:4px; padding:0 5px; border-radius:3px; font-size:9px; font-weight:bold; background:${bg}; color:white;`}>
        {platformIcon}{txt}
      </span>
    );
  });
 })()}
 {/* rc.230 — 머신 inline 태그 (group header 아님, 그냥 태그 수준). */}
 {f.machineTag && f.kind !== "discord" && f.kind !== "telegram" ? (
   <span title={`머신: ${f.machineTag}`}
     style="margin-left:auto; padding:0 6px; color:var(--text-3); font-size:10px; opacity:0.7; white-space:nowrap;">
     {f.machineTag}
   </span>
 ) : null}
 </span>
 <span class="messenger-friend-sub">{f.subtitle}</span>
 {(() => {
   // rc.214 — agent list 한눈 view: peer 행에 role + capabilities inline 표시.
   // rc.226 — 추가로 4-metadata (project_folder · tmux session · LLM · machine) 표시.
   // peer entity = 1 project folder = 1 tmux session = 1 LLM 의 본질 inline.
   if (f.kind !== "peer" || !f.meta) return null;
   const role = (f.meta.role || "").trim();
   const caps = Array.isArray(f.meta.capabilities) ? f.meta.capabilities : [];
   const desc = (f.meta.description || "").trim();
   const projectFolder = (f.meta.project_folder || "").trim();
   const llmType = (f.meta.llm_type || "").trim();
   const llmVersion = (f.meta.llm_version || "").trim();
   const machine = (f.meta.machine || "").trim();
   const tmuxSession = (f.meta.alias || "").trim();
   const hasMeta226 = !!(projectFolder || llmType || machine);
   if (!role && caps.length === 0 && !desc && !hasMeta226) return null;
   const capsShown = caps.slice(0, 4);
   const capsRest = caps.length > 4 ? ` +${caps.length - 4}` : "";
   // project_folder 의 home prefix 단축 (~/)
   const shortFolder = projectFolder
     ? projectFolder.replace(/^\/home\/[^/]+/, "~").replace(/^\/Users\/[^/]+/, "~")
     : "";
   const llmDisplay = llmType && llmVersion
     ? `${llmType} ${llmVersion}`
     : (llmType || "");
   const tooltip =
     (role ? `role: ${role}\n` : "") +
     (desc ? `description: ${desc}\n` : "") +
     (projectFolder ? `project: ${projectFolder}\n` : "") +
     (tmuxSession ? `tmux: ${tmuxSession}\n` : "") +
     (llmDisplay ? `llm: ${llmDisplay}\n` : "") +
     (machine ? `machine: ${machine}\n` : "") +
     (caps.length > 0 ? `capabilities: ${caps.join(", ")}` : "");
   const roleBg: Record<string, string> = {
     "primary": "#7b61ff",
     "worker": "#06c",
     "channel": "#26A5E4",
     "service": "#5a9",
   };
   return (
     <span
       class="messenger-friend-caps"
       title={tooltip}
       style="display:block; font-size:10px; opacity:0.85; margin-top:2px; line-height:1.3; max-width:100%;"
     >
       {/* line 1: role badge + description + capabilities */}
       {(role || desc || capsShown.length > 0) ? (
         <span style="display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">
           {role ? (
             <span
               style={`display:inline-block; padding:0 5px; margin-right:5px; background:${roleBg[role.toLowerCase()] || "#555"}; color:#fff; border-radius:3px; font-size:9px; font-weight:bold; text-transform:uppercase;`}
             >
               {role}
             </span>
           ) : null}
           {desc ? (
             <span style="opacity:0.9; margin-right:6px;">{desc.length > 22 ? desc.slice(0, 22) + "…" : desc}</span>
           ) : null}
           {capsShown.length > 0 ? (
             <span style="opacity:0.75;">
               {capsShown.join(" · ")}{capsRest}
             </span>
           ) : null}
         </span>
       ) : null}
       {/* rc.226 line 2: 4-metadata inline — 📁 project · 📟 tmux · 🤖 LLM · 🏠 machine */}
       {hasMeta226 ? (
         <span style="display:block; font-family:ui-monospace, SFMono-Regular, Menlo, monospace; font-size:9.5px; color:#9ca3af; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; margin-top:1px;">
           <span
             style="cursor:pointer; margin-right:4px; color:#7b61ff; font-weight:bold;"
             onClick={(e) => { e.stopPropagation(); togglePeerExpand(`${f.meta!.alias}::card`);}}
             title="3 sub-resource (worktree · subagents · ex Peer) 펼치기/접기"
           >
             {peerExpand()[`${f.meta!.alias}::card`] ? "▼" : "▶"}
           </span>
           {shortFolder ? (
             <span style="margin-right:8px;" title={`project_folder: ${projectFolder}`}>📁 {shortFolder}</span>
           ) : null}
           {tmuxSession ? (
             <span style="margin-right:8px;" title={`tmux session: ${tmuxSession}`}>📟 {tmuxSession}</span>
           ) : null}
           {llmDisplay ? (
             <span
               style={`margin-right:8px; color:${llmType === "unknown" ? "#6b7280" : "#a78bfa"};`}
               title={`llm: ${llmDisplay}`}
             >🤖 {llmDisplay}</span>
           ) : null}
           {machine ? (
             <span style="color:#60a5fa;" title={`machine: ${machine}`}>🏠 {machine}</span>
           ) : null}
         </span>
       ) : null}
       {/* rc.228 — 3-level tree: worktree / Subagents / ex Peer. card expand 시만 보임. */}
       {hasMeta226 && peerExpand()[`${f.meta!.alias}::card`] ? (() => {
         const wts = Array.isArray(f.meta!.worktrees) ? f.meta!.worktrees : [];
         const sas = Array.isArray(f.meta!.subagents) ? f.meta!.subagents : [];
         const exs = Array.isArray(f.meta!.ex_peers) ? f.meta!.ex_peers : [];
         const subHeaderStyle = "display:block; cursor:pointer; padding:1px 0 1px 16px; font-style:italic; font-size:10px; color:#cbd5e1;";
         const entryStyle = "display:block; padding:0 0 0 28px; font-size:9.5px; color:#9ca3af; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;";
         const wtKey = `${f.meta!.alias}::worktrees`;
         const saKey = `${f.meta!.alias}::subagents`;
         const exKey = `${f.meta!.alias}::ex_peers`;
         return (
           <span style="display:block; margin-top:2px;">
             {/* 🌿 worktree */}
             <span
               style={subHeaderStyle}
               onClick={(e) => { e.stopPropagation(); togglePeerExpand(wtKey);}}
               title="git worktree list"
             >
               {peerExpand()[wtKey] ? "▼" : "▶"} 🌿 worktree ({wts.length})
             </span>
             {peerExpand()[wtKey] && wts.length === 0 ? (
               <span style={entryStyle}>(없음)</span>
             ) : null}
             {peerExpand()[wtKey] ? (
               <For each={wts}>
                 {(w) => (
                   <span style={entryStyle} title={w.path}>
                     {w.branch ? `[${w.branch}] ` : ""}{w.path.replace(/^\/home\/[^/]+/, "~")}
                   </span>
                 )}
               </For>
             ) : null}
             {/* 🤖 Subagents */}
             <span
               style={subHeaderStyle}
               onClick={(e) => { e.stopPropagation(); togglePeerExpand(saKey);}}
               title=".claude/agents/ + agents/ scan"
             >
               {peerExpand()[saKey] ? "▼" : "▶"} 🤖 Subagents ({sas.length})
             </span>
             {peerExpand()[saKey] && sas.length === 0 ? (
               <span style={entryStyle}>(없음)</span>
             ) : null}
             {peerExpand()[saKey] ? (
               <For each={sas}>
                 {(s) => (
                   <span style={entryStyle} title={`${s.kind}: ${s.path}`}>
                     {s.kind === "claude_agents" ? "🅒 " : "🅟 "}{s.name}
                   </span>
                 )}
               </For>
             ) : null}
             {/* 💬 ex Peer (per-peer thread, 삭제 가능) */}
             <span
               style={subHeaderStyle}
               onClick={(e) => { e.stopPropagation(); togglePeerExpand(exKey);}}
               title="이 peer 가 대화한 다른 peer thread"
             >
               {peerExpand()[exKey] ? "▼" : "▶"} 💬 ex Peer ({exs.length})
             </span>
             {peerExpand()[exKey] && exs.length === 0 ? (
               <span style={entryStyle}>(없음)</span>
             ) : null}
             {peerExpand()[exKey] ? (
               <For each={exs}>
                 {(x) => (
                   <span style="display:flex; padding:0 0 0 28px; font-size:9.5px; color:#9ca3af; align-items:center; gap:4px;">
                     <span style="flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">
                       {x.alias} · {x.msg_count}msg
                       {x.last_msg_at ? ` · ${x.last_msg_at.slice(0, 16)}` : ""}
                     </span>
                     <span
                       style="cursor:pointer; padding:0 4px; color:#f87171; font-weight:bold;"
                       onClick={(e) => { e.stopPropagation(); void deleteExPeer(f.meta!.alias, x.alias);}}
                       title={`'${x.alias}' 와의 thread 삭제`}
                     >×</span>
                   </span>
                 )}
               </For>
             ) : null}
           </span>
         );
       })() : null}
     </span>
   );
 })()}
 {(() => {
   // rc.229 fix#3 — session row (터미널 group) 의 on-demand 4-metadata + tree.
   //   화면이 실제 보여주는 건 sessions() row. 클릭/expand 시 agent_detail 로 enrich.
   if (f.kind === "peer" || !f.sessionMeta) return null;
   // alias 추출: identifier 의 aoe_<...> tmux session name, 없으면 display.
   const ident = f.sessionMeta.identifier || "";
   const aoeM = ident.match(/aoe_[a-zA-Z0-9_-]+/);
   const portalM = ident.match(/(?:^|:)portal:([^:]+)/);
   const tmuxM = ident.match(/(?:^|:)tmux:([^:]+)/);
   const alias = aoeM ? aoeM[0]
     : (portalM ? portalM[1]
     : (tmuxM ? tmuxM[1] : (f.display || "").trim()));
   if (!alias) return null;
   const cardKey = `sess::${alias}::card`;
   const open = peerExpand()[cardKey];
   const detail = agentDetails()[alias];
   const loading = detail === "loading";
   const err = detail === "error";
   const d = (detail && typeof detail === "object") ? (detail as Record<string, unknown>) : null;
   const projectFolder = (d?.["project_folder"] as string | null) || "";
   const llmType = (d?.["llm_type"] as string | null) || "";
   const llmVersion = (d?.["llm_version"] as string | null) || "";
   const machine = (d?.["machine"] as string | null) || "";
   const shortFolder = projectFolder
     ? projectFolder.replace(/^\/home\/[^/]+/, "~").replace(/^\/Users\/[^/]+/, "~")
     : "";
   const llmDisplay = llmType && llmVersion ? `${llmType} ${llmVersion}` : (llmType || "");
   const wts = Array.isArray(d?.["worktrees"]) ? (d!["worktrees"] as Array<Record<string, string>>) : [];
   const sas = Array.isArray(d?.["subagents"]) ? (d!["subagents"] as Array<Record<string, string>>) : [];
   const exs = Array.isArray(d?.["ex_peers"]) ? (d!["ex_peers"] as Array<Record<string, unknown>>) : [];
   const subHeaderStyle = "display:block; cursor:pointer; padding:1px 0 1px 16px; font-style:italic; font-size:10px; color:#cbd5e1;";
   const entryStyle = "display:block; padding:0 0 0 28px; font-size:9.5px; color:#9ca3af; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;";
   const wtKey = `sess::${alias}::worktrees`;
   const saKey = `sess::${alias}::subagents`;
   const exKey = `sess::${alias}::ex_peers`;
   return (
     <span class="messenger-friend-caps" style="display:block; font-size:10px; margin-top:2px; max-width:100%;">
       <span style="display:block; font-family:ui-monospace, SFMono-Regular, Menlo, monospace; font-size:9.5px; color:#9ca3af; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">
         <span
           style="cursor:pointer; margin-right:4px; color:#7b61ff; font-weight:bold;"
           onClick={(e) => { e.stopPropagation(); togglePeerExpand(cardKey); if (!open) void loadAgentDetail(alias);}}
           title="에이전트 상세 (project · LLM · machine · worktree · Subagents · ex Peer) 펼치기/접기"
         >
           {open ? "▼" : "▶"}
         </span>
         {open && loading ? <span style="color:#a78bfa;">⏳ 로딩…</span> : null}
         {open && err ? <span style="color:#f87171;">상세 로드 실패</span> : null}
         {open && d ? (
           <>
             {shortFolder ? <span style="margin-right:8px;" title={`project_folder: ${projectFolder}`}>📁 {shortFolder}</span> : null}
             {alias ? <span style="margin-right:8px;" title={`tmux session: ${alias}`}>📟 {alias}</span> : null}
             {llmDisplay ? <span style={`margin-right:8px; color:${llmType === "unknown" ? "#6b7280" : "#a78bfa"};`} title={`llm: ${llmDisplay}`}>🤖 {llmDisplay}</span> : null}
             {machine ? <span style="color:#60a5fa;" title={`machine: ${machine}`}>🏠 {machine}</span> : null}
           </>
         ) : null}
       </span>
       {open && d ? (
         <span style="display:block; margin-top:2px;">
           <span style={subHeaderStyle} onClick={(e) => { e.stopPropagation(); togglePeerExpand(wtKey);}} title="git worktree list">
             {peerExpand()[wtKey] ? "▼" : "▶"} 🌿 worktree ({wts.length})
           </span>
           {peerExpand()[wtKey] && wts.length === 0 ? <span style={entryStyle}>(없음)</span> : null}
           {peerExpand()[wtKey] ? (
             <For each={wts}>{(w) => (
               <span style={entryStyle} title={w.path}>{w.branch ? `[${w.branch}] ` : ""}{(w.path || "").replace(/^\/home\/[^/]+/, "~")}</span>
             )}</For>
           ) : null}
           <span style={subHeaderStyle} onClick={(e) => { e.stopPropagation(); togglePeerExpand(saKey);}} title=".claude/agents/ + agents/ scan">
             {peerExpand()[saKey] ? "▼" : "▶"} 🤖 Subagents ({sas.length})
           </span>
           {peerExpand()[saKey] && sas.length === 0 ? <span style={entryStyle}>(없음)</span> : null}
           {peerExpand()[saKey] ? (
             <For each={sas}>{(s) => (
               <span style={entryStyle} title={`${s.kind}: ${s.path}`}>{s.kind === "claude_agents" ? "🅒 " : "🅟 "}{s.name}</span>
             )}</For>
           ) : null}
           <span style={subHeaderStyle} onClick={(e) => { e.stopPropagation(); togglePeerExpand(exKey);}} title="이 에이전트가 대화한 다른 peer thread">
             {peerExpand()[exKey] ? "▼" : "▶"} 💬 ex Peer ({exs.length})
           </span>
           {peerExpand()[exKey] && exs.length === 0 ? <span style={entryStyle}>(없음)</span> : null}
           {peerExpand()[exKey] ? (
             <For each={exs}>{(x) => (
               <span style={entryStyle}>{String(x["alias"])} · {String(x["msg_count"])}msg{x["last_msg_at"] ? ` · ${String(x["last_msg_at"]).slice(0, 16)}` : ""}</span>
             )}</For>
           ) : null}
         </span>
       ) : null}
     </span>
   );
 })()}
 </span>
 </>
 );
 })()}
 </li>
 )}
 </For>
 </ul>
 </div>
 );
 }}
 </For>
 );
 })()}
 </Show>
 </div>
);
}}
 </For>
 <Show when={(friends() ?? []).length === 0}>
 <div class="messenger-empty" style="padding:12px;">
 {t("messenger.no-friends") || "친구 없음 — + 버튼으로 추가"}
 </div>
 </Show>
 </div>
 </Show>

 {/* L1 — 스레드 모드 콘텐츠 */}
 <Show when={leftMode() === "workflow"}>
 <WorkflowPanel />
 </Show>

 <Show when={leftMode() === "thread"}>
 <div class="messenger-friend-list">
 <Show
 when={threads().length > 0}
 fallback={
 <div class="messenger-empty" style="padding:12px;">
 스레드 없음 — 메시지 송수신 시 conversation_id 별 자동 생성
 </div>
}
 >
 <For each={threads()}>
 {(th) => (
 <div
 class={
 selectedThread() === th.conversation_id
 ? "messenger-friend selected"
 : "messenger-friend"
}
 onClick={() => setSelectedThread(th.conversation_id)}
 style="cursor:pointer; padding:8px;"
 >
 <div style="font-weight:600; font-size:0.9em;">
 {th.conversation_id === UNKNOWN_CONV
 ? "(no conv_id)"
 : `#${th.conversation_id.slice(0, 8)}`}
 </div>
 <div style="font-size:0.8em; opacity:0.7;">
 {th.participants.length} agents · {th.message_count} msg ·{" "}
 {fmtTime(th.last_at)}
 </div>
 <div style="font-size:0.85em; opacity:0.85; margin-top:2px;">
 {th.last_body}
 </div>
 </div>
)}
 </For>
 </Show>
 </div>
 </Show>
 </aside>

 {/* Resizer 1: sidebar ↔ thread */}
 <div class="messenger-resizer" title="드래그: 좌측 너비 조절" onMouseDown={(e) => startResize("left", e)} />

 {/* 중: 대화 — 에이전트 모드 (friend 선택) or 스레드 모드 (conv_id 선택) */}
 <main class="messenger-thread">
 {/* 스레드 모드: 선택된 conversation_id 의 메시지 시간순 */}
 <Show when={leftMode() === "thread" && selectedThread()}>
 {() => {
 const cid = selectedThread()!;
 const th = createMemo(() => threads().find((x) => x.conversation_id === cid));
 const threadMsgs = createMemo(() =>
 (messages() ?? [])
 .filter((m) => (m.conversation_id || UNKNOWN_CONV) === cid)
 .sort((a, b) => Date.parse(a.timestamp) - Date.parse(b.timestamp)),
);
 return (
 <>
 <header class="messenger-thread-head">
 <h2>
 {" "}
 {cid === UNKNOWN_CONV ? "(no conv_id)" : `#${cid.slice(0, 12)}`}
 </h2>
 <small>
 참여 {th()?.participants.length ?? 0} · 메시지{" "}
 {th()?.message_count ?? 0}
 </small>
 </header>
 <section class="messenger-thread-body">
 <Show
 when={threadMsgs().length > 0}
 fallback={
 <div class="messenger-placeholder">메시지 없음</div>
}
 >
 <ul class="messenger-thread-list">
 <For each={threadMsgs()}>
 {(m) => (
 <li class="messenger-thread-item">
 <div class="messenger-thread-meta">
 <span class="messenger-thread-sender"> {m.sender}</span>
 <span class="messenger-thread-time">{fmtTime(m.timestamp)}</span>
 <button
 type="button"
 title="Hand-off — 다른 에이전트에 인계"
 onClick={() => setHandoffSource(m)}
 style="background:transparent; border:none; cursor:pointer; opacity:0.6; padding:0 4px;"
 >
 ↗
 </button>
 </div>
 <div class="messenger-thread-body-text">{m.body}{ackBadge(m)}</div>
 </li>
)}
 </For>
 </ul>
 </Show>
 </section>
 </>
);
}}
 </Show>

 {/* 에이전트 모드 (기존) */}
 <Show
 when={leftMode() === "agent" && selectedFriend()}
 fallback={
 <Show when={!(leftMode() === "thread" && selectedThread())}>
 <div class="messenger-thread-empty">
 <p>
 {leftMode() === "agent"
 ? t("messenger.select-friend") || "왼쪽에서 친구를 선택하세요."
 : "왼쪽에서 스레드를 선택하세요."}
 </p>
 <p class="messenger-thread-hint">
 Tier 2 — 에이전트/스레드 2-모드. 4-tuple 표시 (alias·machine·address).
 </p>
 </div>
 </Show>
}
 >
 {(f) => {
 // 세션 친구 (tmux/claude_project/xgram_session) → SessionScreen (xterm.js, S5).
 const sessionKinds = new Set(["tmux", "claude_project", "xgram_session"]);
 if (sessionKinds.has(f().kind) && f().sessionMeta) {
 return (
 <SessionScreen
 identifier={f().sessionMeta!.identifier}
 display={f().display}
 />
);
}
 // rc.212 — peer 친구: peer_conversation endpoint 로 전체 양방향 session 통합 fetch.
 // 마스터 발 메시지 (sender=self:*) + peer reply (sender=peer:*) + LLM session (user/assistant) 모두.
 // 3초 폴링.
 const [peerConv, { refetch: refetchPeerConv}] = createResource(
 () => f().kind === "peer" ? f().display : null,
 (alias) => fetchPeerConversation(alias as string),
 );
 const convPollTimer = setInterval(() => {
 if (f().kind === "peer") void refetchPeerConv();
 }, 3000);
 onCleanup(() => clearInterval(convPollTimer));

 // filtered 계산: peer 면 peerConv (timestamp ASC 정렬됨), 아니면 messages 전체.
 const filtered = createMemo<MessageDto[]>(() => {
 if (f().kind === "peer") {
 return peerConv() ?? [];
 }
 return messages() ?? [];
});

 // rc.224 — peer view 안 탭 (대화 / 화면). peer alias = tmux session name 1:1 매핑.
 // rc.233 — 기본 = "screen" (터미널 화면). 클릭 즉시 그 세션 tmux capture 표시.
 const [peerTab, setPeerTab] = createSignal<"conv" | "screen">("screen");

 return (
 <>
 <header class="messenger-thread-head">
 <div style="display:flex; align-items:center; gap:10px;">
 <h2 style="margin:0;"> {f().display}</h2>
 {/* rc.233 — 설정(⚙) 토글: 우측 상세/설정 패널(3단) on/off. */}
 <button
 type="button"
 onClick={toggleSettings}
 title={showSettings() ? "설정 패널 닫기 (2단)" : "설정 패널 열기 (3단)"}
 style={`margin-left:auto; padding:4px 10px; font-size:13px; cursor:pointer; border:1px solid var(--border); border-radius:4px; background:${showSettings() ? "rgba(58, 130, 246, 0.25)" : "transparent"}; color:inherit;`}
 >
 ⚙ 설정
 </button>
 </div>
 {/* L2 4-tuple — alias · machine · address · fingerprint */}
 <small>
 {f().meta?.machine ? `${f().meta?.machine} · ` : ""}
 {f().meta?.address?.slice(0, 18) || ""}
 {f().meta?.public_key_hex
 ? ` · ${fingerprint(f().meta!.public_key_hex)}`
 : ""}
 </small>
 {/* rc.224 — peer 친구만 탭 노출. rc.233 — 화면(tmux) 기본, 대화 보조. */}
 <Show when={f().kind === "peer"}>
 <div class="messenger-peer-tabs" style="display:flex; gap:4px; margin-top:8px;">
 <button
 type="button"
 onClick={() => setPeerTab("screen")}
 title="이 피어의 tmux 세션 화면 (5초 polling)"
 style={`padding:4px 12px; font-size:12px; cursor:pointer; border:1px solid var(--border); border-radius:4px; background:${peerTab() === "screen" ? "rgba(58, 130, 246, 0.25)" : "transparent"}; color:inherit;`}
 >
 화면
 </button>
 <button
 type="button"
 onClick={() => setPeerTab("conv")}
 style={`padding:4px 12px; font-size:12px; cursor:pointer; border:1px solid var(--border); border-radius:4px; background:${peerTab() === "conv" ? "rgba(58, 130, 246, 0.25)" : "transparent"}; color:inherit;`}
 >
 대화
 </button>
 </div>
 </Show>
 </header>
 <section class="messenger-thread-body">
 <Show
 when={f().kind === "peer" && peerTab() === "screen"}
 fallback={
 <Show
 when={(filtered() ?? []).length > 0}
 fallback={
 <div class="messenger-placeholder">
 {t("messenger.thread-empty") ||
 `${f().display} 의 메시지 없음 — daemon 가동 + 메시지 도착 시 3초 내 표시됩니다.`}
 </div>
}
 >
 {/* rc.212 — peer 면 timestamp ASC (오래된 → 최신 아래). 아니면 기존 reverse. */}
 <ul class="messenger-thread-list">
 <For each={f().kind === "peer" ? filtered() : filtered().slice().reverse()}>
 {(m) => {
 const self = isSelfSender(m.sender);
 // rc.212 — sender 측 (마스터) = 오른쪽 정렬 + 파란 톤. peer/assistant = 왼쪽 + 회색 톤.
 const align = self ? "flex-end" : "flex-start";
 const bg = self ? "rgba(58, 130, 246, 0.15)" : "rgba(255, 255, 255, 0.04)";
 const border = self ? "rgba(58, 130, 246, 0.4)" : "var(--border)";
 return (
 <li class="messenger-thread-item" style={`display:flex; flex-direction:column; align-items:${align}; padding:6px 8px;`}>
 <div class="messenger-thread-meta" style={`display:flex; gap:8px; font-size:11px; opacity:0.75; align-items:center; ${self ? "flex-direction:row-reverse;" : ""}`}>
 <span class="messenger-thread-sender" style="font-weight:600;">{m.sender}</span>
 <span class="messenger-thread-time">{fmtTime(m.timestamp)}</span>
 <button
 type="button"
 title="Hand-off — 다른 에이전트에 인계"
 onClick={() => setHandoffSource(m)}
 style="background:transparent; border:none; cursor:pointer; opacity:0.6; padding:0 4px;"
 >
 ↗
 </button>
 </div>
 <div
 class="messenger-thread-body-text"
 style={`max-width:75%; margin-top:3px; padding:6px 10px; border-radius:8px; background:${bg}; border:1px solid ${border}; white-space:pre-wrap; word-break:break-word;`}
 >
 {m.body}{ackBadge(m)}
 </div>
 </li>
 );
 }}
 </For>
 </ul>
 </Show>
}
 >
 {/* rc.224 — tmux 화면 preview. alias = tmux session name. */}
 <TmuxPreview alias={f().display} />
 </Show>
 </section>
 <PeerInput
 friend={f()}
 onSent={() => {
 void refetchMessages();
 if (f().kind === "peer") void refetchPeerConv();
}}
 />
 </>
);
}}
 </Show>
 </main>

 {/* Resizer 2 + 우측 사이드 패널 — sidepanel 있을 때만 */}
 <Show when={hasSidepanel()}>
 <div class="messenger-resizer" title="드래그: 우측 너비 조절" onMouseDown={(e) => startResize("right", e)} />
 </Show>
 {/* 우: 12 탭 사이드 패널 (사양 §5 — peer 또는 session friend 선택 시) */}
 <Show
 when={
 showSettings() &&
 leftMode() === "agent" &&
 selectedFriend() &&
 (selectedFriend()!.kind === "peer" ||
 selectedFriend()!.kind === "tmux" ||
 selectedFriend()!.kind === "claude_project" ||
 selectedFriend()!.kind === "xgram_session")
}
 >
 <AgentSidePanel
 peer={(() => {
 const f = selectedFriend();
 if (!f) return { alias: "", address: "", public_key_hex: ""} as any;
 const machineAlias = sessions()?.machine?.alias?.replace(/\.c\.[a-z0-9-]+\.internal$/, " (GCP)").replace(/\.tail[a-z0-9]+\.ts\.net$/, " (Tailscale)") || sessions()?.machine?.hostname;
 return f.meta ?? {
 alias: f.display,
 address: f.sessionMeta?.identifier ?? "",
 public_key_hex: "",
 machine: machineAlias,
 last_seen: f.sessionMeta?.last_active_at ?? undefined,
 };
 })()}
 onJumpToSettings={() => props.onJumpToSettings?.()}
 />
 </Show>

 {/* L5 — Hand-off 모달 (메시지 옆 ↗ 버튼 클릭 시) */}
 <Show when={handoffSource()}>
 {() => (
 <HandoffModal
 source={handoffSource()!}
 peers={(peers() ?? []).filter((p) => p.alias !== handoffSource()!.sender)}
 onClose={() => setHandoffSource(null)}
 onSent={() => {
 setHandoffSource(null);
 void refetchMessages();
}}
 />
)}
 </Show>
 </div>
);
}

// ── rc.224 TmuxPreview — peer card 안 tmux 세션 inline preview ─────────────
// peer alias = tmux session name 1:1 매핑 (auto-seed mechanism).
// backend GET /v1/gui/sessions/{alias}/screen 5초 polling.
// dead session 시 polling stop + 명시 표시.
// xterm.js 안 쓰고 simple plain text (capture-pane 출력). monospace dark.
function TmuxPreview(props: { alias: string}) {
 const [content, setContent] = createSignal<string>("");
 const [lines, setLines] = createSignal<number>(0);
 const [sourceNote, setSourceNote] = createSignal<string>("");
 const [fetchedAt, setFetchedAt] = createSignal<string>("");
 const [error, setError] = createSignal<string | null>(null);
 const [dead, setDead] = createSignal<boolean>(false);
 const [autoPoll, setAutoPoll] = createSignal<boolean>(true);
 // rc.233 — 첫 fetch 전 loading spinner. 클릭 즉시 1회 fetch → < 1s 목표.
 const [loading, setLoading] = createSignal<boolean>(true);
 let pollTimer: number | undefined;

 interface SessionScreenDto {
 identifier: string;
 kind: string;
 display: string;
 content: string;
 lines: number;
 source_note: string;
 fetched_at: string;
 }

 // ANSI escape sequence (CSI / OSC / 단독 ESC) 를 plain text 로 strip.
 // capture-pane -e 출력에는 컬러 코드가 포함 — preview 는 단순 텍스트.
 function stripAnsi(s: string): string {
 // CSI: ESC [ ... letter
 // OSC: ESC ] ... BEL or ESC \
 return s
 .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "")
 .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "")
 .replace(/\x1b[@-Z\\-_]/g, "");
 }

 async function refresh() {
 try {
 const dto = await invoke<SessionScreenDto>("session_screen", {
 identifier: props.alias,
 });
 setContent(stripAnsi(dto.content || ""));
 setLines(dto.lines || 0);
 setSourceNote(dto.source_note || "");
 setFetchedAt(dto.fetched_at || "");
 setError(null);
 setDead(false);
 setLoading(false);
 } catch (e) {
 setLoading(false);
 const msg = String(e);
 setError(msg);
 // 404 / not found / session not exist → dead. polling stop.
 if (/not.?found|404|no such|exist/i.test(msg)) {
 setDead(true);
 if (pollTimer) {
 clearInterval(pollTimer);
 pollTimer = undefined;
 }
 }
 }
 }

 function startPolling() {
 if (pollTimer) clearInterval(pollTimer);
 if (!autoPoll() || dead()) return;
 pollTimer = setInterval(() => void refresh(), 5000) as unknown as number;
 }

 onMount(() => {
 void refresh();
 startPolling();
 });
 onCleanup(() => {
 if (pollTimer) clearInterval(pollTimer);
 });

 // alias 가 변경되면 reset (다른 peer 카드로 전환 시).
 createEffect(() => {
 const a = props.alias;
 if (!a) return;
 setDead(false);
 setError(null);
 setContent("");
 setLoading(true);
 void refresh();
 startPolling();
 });

 // autoPoll toggle 시 polling 재시작.
 createEffect(() => {
 const on = autoPoll();
 if (pollTimer) {
 clearInterval(pollTimer);
 pollTimer = undefined;
 }
 if (on && !dead()) startPolling();
 });

 // rc.233 — 중앙 메인 화면 = tail 60 줄 (preview 보다 넉넉히).
 const tailContent = createMemo<string>(() => {
 const c = content();
 if (!c) return "";
 const allLines = c.split(/\r?\n/);
 const tail = allLines.slice(-60);
 return tail.join("\n");
 });

 return (
 <div class="tmux-preview-wrap" style="padding:12px; display:flex; flex-direction:column; gap:8px; height:100%; box-sizing:border-box;">
 <div class="tmux-preview-toolbar" style="display:flex; gap:8px; align-items:center; font-size:12px; opacity:0.85;">
 <strong>tmux: {props.alias}</strong>
 <span style="opacity:0.6;">{sourceNote()}</span>
 <span style="margin-left:auto; display:flex; gap:6px; align-items:center;">
 <button
 type="button"
 onClick={() => void refresh()}
 style="padding:2px 8px; font-size:11px; cursor:pointer; border:1px solid var(--border); border-radius:3px; background:transparent; color:inherit;"
 >
 Refresh
 </button>
 <label style="display:flex; gap:4px; align-items:center; cursor:pointer;">
 <input
 type="checkbox"
 checked={autoPoll()}
 onChange={(e) => setAutoPoll(e.currentTarget.checked)}
 />
 auto (5s)
 </label>
 </span>
 </div>
 <Show when={dead()}>
 <div
 style="padding:12px; background:rgba(248, 81, 73, 0.12); border:1px solid rgba(248, 81, 73, 0.4); border-radius:4px; font-size:12px;"
 >
 tmux session not found — "{props.alias}" 세션이 머신에 없거나 종료됨. polling 중지.
 </div>
 </Show>
 <Show when={!dead() && error()}>
 <div
 style="padding:8px; background:rgba(210, 153, 34, 0.12); border:1px solid rgba(210, 153, 34, 0.4); border-radius:4px; font-size:11px; opacity:0.8;"
 >
 fetch error: {error()}
 </div>
 </Show>
 <Show when={loading() && !content() && !dead()}>
 <div style="flex:1; display:flex; align-items:center; justify-content:center; color:#7b61ff; font-size:13px; gap:8px;">
 <span style="display:inline-block; width:14px; height:14px; border:2px solid #7b61ff; border-top-color:transparent; border-radius:50%; animation:tmux-spin 0.7s linear infinite;" />
 화면 불러오는 중…
 </div>
 <style>{`@keyframes tmux-spin{to{transform:rotate(360deg)}}`}</style>
 </Show>
 <pre
 class="tmux-preview"
 style="font-family: ui-monospace, Menlo, Consolas, monospace; font-size:12px; background:#0a0a0a; color:#e6edf3; padding:12px; border-radius:4px; white-space:pre; overflow:auto; flex:1; min-height:0; margin:0;"
 >{tailContent()}</pre>
 <div style="font-size:10px; opacity:0.5; text-align:right;">
 {lines() > 0 ? `${lines()} lines total · tail 30 · ` : ""}
 {fetchedAt() ? `fetched ${fmtTime(fetchedAt())}` : ""}
 </div>
 </div>
 );
}

// ── L5 Hand-off 모달 ──────────────────────────────────────────────
function HandoffModal(props: {
 source: MessageDto;
 peers: PeerDto[];
 onClose: () => void;
 onSent: () => void;
}) {
 const [target, setTarget] = createSignal<string>(props.peers[0]?.alias ?? "");
 const [threadMode, setThreadMode] = createSignal<"new" | "existing">("new");
 const [summary, setSummary] = createSignal(
 `[hand-off] ${props.source.sender} 가 보낸 메시지 인계 — 검토·후속 작업 부탁:\n"${props.source.body.slice(0, 200)}"`,
);
 const [sending, setSending] = createSignal(false);
 const [error, setError] = createSignal<string | null>(null);

 async function handoff() {
 if (!target()) {
 setError("대상 peer 가 없습니다 (다른 peer 1명 이상 필요)");
 return;
}
 setSending(true);
 setError(null);
 try {
 // 새 스레드 = conversation_id 생략 (daemon 자동 생성).
 // 기존 스레드 = source 의 conversation_id 그대로.
 const conv = threadMode() === "existing" ? props.source.conversation_id : undefined;
 await invoke("peer_send", {
 alias: target(),
 body: summary(),
 ...(conv ? { conversation_id: conv} : {}),
});
 props.onSent();
} catch (e: any) {
 setError(typeof e === "string" ? e : (e?.message ?? String(e)));
} finally {
 setSending(false);
}
}

 return (
 <div class="handoff-overlay" onClick={props.onClose}>
 <div class="handoff-modal" onClick={(e) => e.stopPropagation()}>
 <h3>↗ Hand-off — 메시지 인계</h3>
 <div class="source-preview">
 <div>출처: {props.source.sender}</div>
 <div style="margin-top:4px;">
 {props.source.body.slice(0, 120)}
 {props.source.body.length > 120 ? "…" : ""}
 </div>
 </div>

 <div style="margin-bottom:10px;">
 <label>대상 peer</label>
 <select value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
 <For each={props.peers}>
 {(p) => <option value={p.alias}>{p.alias} · {p.machine || "(unknown)"}</option>}
 </For>
 <Show when={props.peers.length === 0}>
 <option value="">— 다른 peer 없음 —</option>
 </Show>
 </select>
 </div>

 <div style="margin-bottom:10px;">
 <label>자동 요약 (편집 가능)</label>
 <textarea
 rows={4}
 value={summary()}
 onInput={(e) => setSummary(e.currentTarget.value)}
 />
 </div>

 <div style="margin-bottom:14px;">
 <label>스레드 처리 (radio — 둘 중 하나)</label>
 <div class="radio-row">
 <label style="cursor:pointer;">
 <input
 type="radio"
 checked={threadMode() === "new"}
 onChange={() => setThreadMode("new")}
 />{" "}
 새 스레드 생성 (parent: 이 스레드)
 </label>
 </div>
 <div class="radio-row">
 <label style="cursor:pointer;">
 <input
 type="radio"
 checked={threadMode() === "existing"}
 onChange={() => setThreadMode("existing")}
 />{" "}
 기존 스레드에 추가 (conv: {props.source.conversation_id.slice(0, 10)}…)
 </label>
 </div>
 </div>

 <Show when={error()}>
 <div style="color:#f87171; font-size:12px; margin-bottom:8px;">{error()}</div>
 </Show>

 <div class="actions">
 <button type="button" onClick={props.onClose} disabled={sending()}>
 취소
 </button>
 <button
 type="button"
 class="primary"
 onClick={() => void handoff()}
 disabled={sending() || !target() || props.peers.length === 0}
 >
 {sending() ? "인계 중…" : "인계 →"}
 </button>
 </div>
 </div>
 </div>
);
}

// 채널(Discord/Telegram) 친구는 입력 비활성, peer 만 송신 가능.
function PeerInput(props: { friend: Friend; onSent: () => void}) {
 const { t} = useI18n();
 const [text, setText] = createSignal("");
 const [sending, setSending] = createSignal(false);
 const [error, setError] = createSignal<string | null>(null);

 const isPeer = () => props.friend.kind === "peer";

 // UI-MESSENGER-SPEC v1.3 §4.5 — 사용자 개입 = system priority (기본) / 일반 참여자.
 const [intervention, setIntervention] = createSignal(true);

 async function send() {
 const raw = text().trim();
 if (!raw) return;
 if (!isPeer()) {
 setError(t("messenger.send-peer-only") || "송신은 peer 친구에게만 가능 (Discord/Telegram 채널 송신은 별도)");
 return;
}
 setSending(true);
 setError(null);
 try {
 // 개입 모드 = body 에 [개입] prefix (수신 측 LLM 이 system priority 로 인식).
 const body = intervention() ? `[개입] ${raw}` : raw;
 await invoke("peer_send", { alias: props.friend.display, body});
 setText("");
 props.onSent();
} catch (e: any) {
 setError(typeof e === "string" ? e : (e?.message ?? String(e)));
} finally {
 setSending(false);
}
}

 return (
 <footer class="messenger-thread-input">
 {/* L5/§4.5 — 입력 모드 토글 */}
 <div class="messenger-input-mode">
 <label>
 <input
 type="radio"
 checked={intervention()}
 onChange={() => setIntervention(true)}
 />{" "}
 개입 (system priority)
 </label>
 <label>
 <input
 type="radio"
 checked={!intervention()}
 onChange={() => setIntervention(false)}
 />{" "}
 일반 참여
 </label>
 </div>
 <div class="messenger-thread-input-row">
 <textarea
 rows={2}
 value={text()}
 onInput={(ev) => setText(ev.currentTarget.value)}
 placeholder={
 isPeer()
 ? intervention()
 ? "[개입] 메시지 — system priority 로 전송 (Enter, Shift+Enter 줄바꿈)"
 : (t("messenger.input-placeholder") || "메시지 입력")
 : (t("messenger.send-peer-only") || "Discord/Telegram 채널 송신은 별도")
}
 disabled={!isPeer() || sending()}
 onKeyDown={(ev) => {
 if (ev.key === "Enter" && !ev.shiftKey) {
 ev.preventDefault();
 void send();
}
}}
 />
 {/* S7 첨부 업로드 — content-addressed (V2/V3 refcount) */}
 <button
 type="button"
 title="첨부 파일 (S7 — <1MB inline / ≥1MB disk)"
 onClick={() => {
 const input = document.createElement("input");
 input.type = "file";
 input.onchange = async (ev: any) => {
 const f = ev.target?.files?.[0];
 if (!f) return;
 const reader = new FileReader();
 reader.onload = async () => {
 const b64 = (reader.result as string).split(",")[1] ?? "";
 try {
 const res: any = await invoke("attachment_upload", { content_b64: b64, mime: f.type || "application/octet-stream"});
 setText(`${text()}\n attachment://${res.content_hash} (${(res.size_bytes/1024).toFixed(1)} KB · ${res.storage})`);
} catch (e) {
 setError(`첨부 실패: ${e}`);
}
};
 reader.readAsDataURL(f);
};
 input.click();
}}
 style="background:transparent; border:1px solid var(--border); padding:6px 10px; border-radius:4px; cursor:pointer;"
 ></button>
 <button
 type="button"
 disabled={!isPeer() || sending() || !text().trim()}
 onClick={() => void send()}
 >
 {sending()
 ? (t("messenger.sending") || "보내는 중…")
 : intervention()
 ? "[개입] 전송"
 : (t("messenger.send") || "전송")}
 </button>
 </div>
 <Show when={error()}>
 <div class="messenger-thread-error" role="alert">{error()}</div>
 </Show>
 </footer>
);
}

