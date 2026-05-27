import { createMemo, createResource, createSignal, For, Show, onCleanup} from "solid-js";
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
}

async function fetchMessages(): Promise<MessageDto[]> {
 try {
 return await invoke<MessageDto[]>("messages_recent", { limit: 100});
} catch {
 return [];
}
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

export function Messenger(props: { onJumpToSettings?: () => void} = {}) {
 const { t} = useI18n();
 const [peers] = createResource(fetchPeers);
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
 const [selected, setSelected] = createSignal<string | null>(null); // friend id (에이전트 모드)
 const [selectedThread, setSelectedThread] = createSignal<string | null>(null); // conversation_id
 const [leftMode, setLeftMode] = createSignal<LeftMode>("agent"); // L1
 // 컬럼 너비 — drag 로 조절, localStorage 영구
 const initialSidebar = (() => { const v = parseInt(localStorage.getItem("messenger.sidebar_w") || "240"); return isNaN(v) ? 240 : v; })();
 const initialSidepanel = (() => { const v = parseInt(localStorage.getItem("messenger.sidepanel_w") || "320"); return isNaN(v) ? 320 : v; })();
 const [sidebarW, setSidebarW] = createSignal(initialSidebar);
 const [sidepanelW, setSidepanelW] = createSignal(initialSidepanel);
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

 // peer 만 머신 그룹화 + 채널은 별 "채널" 가짜 머신.
 const groups = createMemo<MachineGroup[]>(() => {
 const byMachine = new Map<string, Friend[]>();

 for (const p of peers() ?? []) {
 if (connFilter() === "connected" && !isConnected(p)) continue;
 if (connFilter() === "offline" && isConnected(p)) continue;
 // rc.138 — peers schema 에 machine 컬럼 없음 → alias 를 머신명 fallback 으로 사용.
 // zalman / 다른 peer 가 (unknown) 그룹 대신 각자 머신 그룹으로 표시.
 const m = (p.machine?.trim() || p.alias?.trim() || UNKNOWN_MACHINE);
 const friend: Friend = {
 kind: "peer",
 id: `peer:${p.alias}`,
 display: p.alias,
 // L2 4-tuple: alias · machine · address(short) · fingerprint
 subtitle:
 `${(p.address || "").slice(0, 10)} · ${fingerprint(p.public_key_hex)}` +
 (p.last_seen ? ` · ${p.last_seen}` : ""),
 meta: p,
};
 if (!byMachine.has(m)) byMachine.set(m, []);
 byMachine.get(m)!.push(friend);
}

 // 이 머신의 tmux + Claude Code projects + xgram sessions (M-1).
 // sessions.machine.alias 를 머신 그룹으로 사용 — peers 의 machine 과 중복 시 같은 그룹에 추가.
 const sess = sessions();
 if (sess) {
 const localMachine = sess.machine.alias || sess.machine.hostname || UNKNOWN_MACHINE;
 // rc.141 — portal:/aoe: 가 같은 tmux session 가리키면 portal: 만 유지.
 // zalman daemon 의 /api/terminals + /api/aoe/sessions 가 같은 aoe_xxx tmux 를 2번 detect → 중복.
 // selectedFriend 가 polling 마다 두 identifier 사이에서 깜빡거리는 문제 해결.
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
 const conn = s.status === "attached" || s.status === "active";
 if (connFilter() === "connected" && !conn) continue;
 if (connFilter() === "offline" && conn) continue;
 // identifier "peer:<alias>:..." 면 그 alias 머신으로 분리, 아니면 local 머신.
 let machine = localMachine;
 if (s.identifier.startsWith("peer:")) {
 const parts = s.identifier.split(":");
 if (parts.length >= 2) machine = parts[1]; // peer alias 자체가 머신명
 }
 const arr = byMachine.get(machine) ?? [];
 arr.push({
 kind: s.kind as FriendKind,
 id: `session:${s.identifier}`,
 display: s.display.replace(/^\[[^\]]+\]\s*/, ""), // [zalman] prefix 제거 (이미 머신 그룹으로 분리됨)
 subtitle:
 s.kind === "tmux"
 ? `tmux · ${s.windows ?? 0} win · ${s.attached ? "attached" : "detached"}`
 : s.kind === "claude_project"
 ? `Claude Code · ${s.last_active_at ? new Date(s.last_active_at).toLocaleString() : "—"}`
 : "xgram session",
 sessionMeta: s,
 });
 byMachine.set(machine, arr);
 }
}

 // 정렬
 const sorter = (a: Friend, b: Friend) => {
 if (sortMode() === "name") return a.display.localeCompare(b.display);
 // activity: meta.last_seen DESC (없으면 뒤로)
 const ta = a.meta?.last_seen ? Date.parse(a.meta.last_seen) : 0;
 const tb = b.meta?.last_seen ? Date.parse(b.meta.last_seen) : 0;
 return tb - ta;
};
 for (const arr of byMachine.values()) arr.sort(sorter);

 const out: MachineGroup[] = [];
 // 머신 정렬: 이름순. UNKNOWN_MACHINE 은 마지막.
 const machines = Array.from(byMachine.keys()).sort((a, b) => {
 if (a === UNKNOWN_MACHINE) return 1;
 if (b === UNKNOWN_MACHINE) return -1;
 return a.localeCompare(b);
});
 for (const m of machines) {
 const friends = byMachine.get(m)!;
 out.push({
 machine: m,
 friends,
 connected: friends.filter((f) => f.meta && isConnected(f.meta)).length,
});
}

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

 const hasSidepanel = () =>
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
 <button
 type="button"
 class="messenger-add-btn"
 title="peer 등록 / 봇 연결"
 style="padding:4px 10px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; cursor:pointer; font-size:12px;"
 onClick={() => alert("peer 등록: peer add CLI 또는 채널 카드 → 봇 등록")}
 >
 + 추가
 </button>
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
 const isCollapsed = () => collapsed()[g.machine] === true;
 return (
 <div class="messenger-machine-group">
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
 {(() => {
 const t = g.friends.filter(f => f.kind === "tmux").length;
 const att = g.friends.filter(f => f.kind === "tmux" && f.sessionMeta?.attached).length;
 const c = g.friends.filter(f => f.kind === "claude_project").length;
 const p = g.friends.filter(f => f.kind === "peer").length;
 const parts: string[] = [];
 if (t > 0) parts.push(`tmux ${att}/${t} 접속 (local)`);
 if (c > 0) parts.push(`Claude Code ${c} (local)`);
 if (p > 0) parts.push(`P2P peer ${g.connected}/${p} 연결`);
 return `(${parts.join(" · ") || g.friends.length})`;
 })()}
 </span>
 </span>
 <button class="link-btn" title="이 머신 sessions 즉시 새로고침 (자동 10초)"
 style="font-size:11px; padding:2px 6px;"
 onClick={(e) => { e.stopPropagation(); refetchSessions();}}>↻</button>
 </div>
 <Show when={!isCollapsed()}>
 {(() => {
 // 사용자 의도 기준 4그룹. tmux raw attached/detached 구분 폐기.
 type Sub = { key: string; label: string; items: Friend[]};
 const subs: Sub[] = [
 { key: "portal", label: "터미널 (포털 등록)", items: []},
 { key: "claude-active", label: "Claude Code · 최근 24h", items: []},
 { key: "claude-old", label: "Claude Code · 오래됨", items: []},
 { key: "peer", label: "다른 머신 (peer)", items: []},
 { key: "channel", label: "채널 (Discord/Telegram)", items: []},
 { key: "other", label: "기타", items: []},
 ];
 for (const f of g.friends) {
 const id = f.id || "";
 // rc.139 — claude_project 완전 숨김 (사용자 결정 A).
 // ~/.claude/projects 의 36+ 디렉토리 자동 detect 가 사용자 의도와 다름.
 // 메신저 = 활성 터미널만. claude 디렉토리 history 노출 X.
 if (f.kind === "claude_project") continue;
 // portal:* (옛 portal 등록 터미널) + aoe:* (AoE 세션) + tmux:* (portal 없는 머신 fallback)
 if (id.startsWith("portal:") || id.startsWith("aoe:") || f.kind === "tmux") {
 subs[0].items.push(f);
 } else if (f.kind === "peer") {
 subs[3].items.push(f);
 } else if (f.kind === "discord" || f.kind === "telegram") {
 subs[4].items.push(f);
 } else {
 subs[5].items.push(f);
 }
 }
 return (
 <For each={subs.filter(s => s.items.length > 0)}>
 {(sub) => {
 const subKey = `${g.machine}::${sub.key}`;
 // default: 터미널 (포털 등록) 만 펼침, 나머지는 접힘. 사용자 클릭하면 명시값 유지.
 const subCollapsed = () => {
 const v = collapsed()[subKey];
 if (v !== undefined) return v;
 return sub.key !== "portal";
 };
 return (
 <div>
 <div
 onClick={() => setCollapsed(p => ({...p, [subKey]: !p[subKey]}))}
 style="cursor:pointer; padding:5px 12px 5px 18px; font-size:11px; color:var(--text-3); display:flex; align-items:center; gap:4px; background:rgba(255,255,255,0.02); border-top:1px solid var(--border);"
 >
 <span>{subCollapsed() ? "▸" : "▾"}</span>
 <span style="flex:1;">{sub.label}</span>
 <span style="opacity:0.7;">{sub.items.length}</span>
 </div>
 <Show when={!subCollapsed()}>
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
 if (f.sessionMeta?.attached) { dotColor = "#4caf50"; dotTitle = "attached (사용자 접속 중)";}
 else { dotColor = "#d4a017"; dotTitle = "detached (백그라운드 실행 중)";}
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
 return (
 <>
 <span
 class="messenger-friend-icon"
 title={info.label}
 style={`background:${info.bg}; color:${info.color}; width:18px; height:18px; border-radius:3px; display:inline-flex; align-items:center; justify-content:center; font-size:11px; font-weight:bold; margin-right:6px;`}
 >
 {info.icon}
 </span>
 <span
 title={dotTitle}
 style={`display:inline-block; width:8px; height:8px; border-radius:50%; margin-right:6px; background:${dotColor};`}
 />
 <span class="messenger-friend-text">
 <span class="messenger-friend-name">
 {f.display}
 {(() => {
 const set = registeredAgents();
 const m = (f.id || "").match(/aoe_[a-z0-9_-]+/i);
 const reg = set && m && set.has(m[0]);
 return reg ? (
 <span title="메신저 등록됨 — 다른 peer 의 list_peers 에 노출"
 style="margin-left:5px; padding:0 5px; background:#238636; color:white; border-radius:3px; font-size:9px; font-weight:bold;">
 ✓ MSG
 </span>
 ) : null;
 })()}
 </span>
 <span class="messenger-friend-sub">{f.subtitle}</span>
 </span>
 </>
 );
 })()}
 </li>
 )}
 </For>
 </ul>
 </Show>
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
 <div class="messenger-thread-body-text">{m.body}</div>
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
 // peer 친구는 sender 로 필터, 채널(Discord/Telegram)은 일단 전체 보여줌.
 const filtered = createMemo<MessageDto[]>(() => {
 const all = messages() ?? [];
 if (f().kind !== "peer") return all;
 const alias = f().display;
 const addr = f().meta?.address?.toLowerCase();
 return all.filter((m) => {
 const s = m.sender.toLowerCase();
 return s === alias.toLowerCase() || (addr ? s === addr : false);
});
});

 return (
 <>
 <header class="messenger-thread-head">
 <h2> {f().display}</h2>
 {/* L2 4-tuple — alias · machine · address · fingerprint */}
 <small>
 {f().meta?.machine ? `${f().meta?.machine} · ` : ""}
 {f().meta?.address?.slice(0, 18) || ""}
 {f().meta?.public_key_hex
 ? ` · ${fingerprint(f().meta!.public_key_hex)}`
 : ""}
 </small>
 </header>
 <section class="messenger-thread-body">
 <Show
 when={(filtered() ?? []).length > 0}
 fallback={
 <div class="messenger-placeholder">
 {t("messenger.thread-empty") ||
 `${f().display} 의 메시지 없음 — daemon 가동 + 메시지 도착 시 3초 내 표시됩니다.`}
 </div>
}
 >
 <ul class="messenger-thread-list">
 <For each={filtered().slice().reverse()}>
 {(m) => (
 <li class="messenger-thread-item">
 <div class="messenger-thread-meta">
 <span class="messenger-thread-sender">{m.sender}</span>
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
 <div class="messenger-thread-body-text">{m.body}</div>
 </li>
)}
 </For>
 </ul>
 </Show>
 </section>
 <PeerInput
 friend={f()}
 onSent={() => {
 void refetchMessages();
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

