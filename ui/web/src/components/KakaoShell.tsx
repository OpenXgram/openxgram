import { createSignal, createResource, createMemo, createEffect, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./mockup.css";
import { AcpConversation, aiTypeToAdapter, type AcpPreset } from "./AcpConversation";
import { A2AMiniPanel } from "./A2AMiniPanel";
import { RoomModal } from "./RoomModal";
import { AddAgentModal } from "./AddAgentModal";
import { AddFriendModal } from "./AddFriendModal";
import { AgentRequestsInbox } from "./AgentRequestsInbox";
import { FlowTab } from "./FlowTab";
import { MarketTab } from "./MarketTab";
import { ConfigTab } from "./ConfigTab";
import { RuntimeTab } from "./RuntimeTab";
import { WikiTab } from "./WikiTab";
import { type DetectedSession, type SessionsDto, normPath, isTooBroadPath } from "./agentSessions";

// ──────────────────────────────────────────────────────────────────────────
// OpenXgram 대화 모델 셸 — 정본 목업 VERBATIM 이식.
//   정본: _mockups/openxgram-conversation-model-mockup.html
//   목업 markup(.app/.rail/.list/.room/.chat/.chead/.mini/.comp/.side/.dash/.me-pane)을
//   그대로 JSX 로 옮기고, 목업이 하드코딩하던 샘플 배열(DATA/ROOMS/STUB/friends)을
//   라이브 데이터·엔드포인트로 치환한다. CSS 는 mockup.css(verbatim 포팅).
//
//   라이브 데이터: agents_list(명부·분류·ai_type) · peers_list(online) ·
//     messages_recent(미리보기/시각). 대화 본문은 AcpConversation(ACP SSE 스트림) 임베드.
//   협업 곁뷰(.side)=A2AMiniPanel(P4a 발언권/P4c 오케스트레이션/P5 멤버/P6 보안방).
//   방 설정 모달=RoomModal(room_config get/set). 모두 verbatim 목업 구조 안에 배선.
// ──────────────────────────────────────────────────────────────────────────

interface AgentRow {
  alias: string;
  role?: string | null;
  description?: string | null;
  ai_type?: string | null;
  classification?: string | null;
  project_path?: string | null;
  machine?: string | null;
  display_name?: string | null;
  is_public?: boolean | null;
  perm_mode?: string | null;
  model?: string | null;
  thinking?: string | null;
  execution_mode?: string | null;
  unread?: number | null;
}
interface PeerDto { alias: string; last_seen?: string; machine?: string; address?: string }
interface MessageDto { id: string; sender: string; body: string; timestamp: string; conversation_id: string }

const agentName = (a: { display_name?: string | null; alias: string }) =>
  (a.display_name && a.display_name.trim()) || a.alias;

// ai_type → 목업 .av 배경(목업의 하드코딩 색을 라이브 ai_type 으로 매핑).
const AI_BG: Record<string, string> = {
  claude: "#d97757", codex: "#10a37f", gemini: "#4285f4", ollama: "#5b5b66", hermes: "#7c5cff",
};
function avatarBg(a: AgentRow, isPrimary: boolean): string {
  if (isPrimary) return "linear-gradient(135deg,#ffd84d,#ff9e2c)";
  return (a.ai_type && AI_BG[a.ai_type.toLowerCase()]) || "#7c8ba1";
}

function isOnline(lastSeen?: string): boolean {
  if (!lastSeen) return false;
  const t = Date.parse(lastSeen);
  return !Number.isNaN(t) && Date.now() - t < 60 * 60 * 1000;
}
function fmtClock(iso: string): string {
  try { const d = new Date(iso); if (Number.isNaN(d.getTime())) return "";
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  } catch { return ""; }
}
function fmtPreviewTime(iso: string): string {
  try {
    const d = new Date(iso); if (Number.isNaN(d.getTime())) return "";
    const now = new Date();
    if (d.toDateString() === now.toDateString()) return fmtClock(iso);
    const y = new Date(now); y.setDate(now.getDate() - 1);
    if (d.toDateString() === y.toDateString()) return "어제";
    return `${d.getMonth() + 1}/${d.getDate()}`;
  } catch { return ""; }
}

type Tab = "chat" | "dash" | "flow" | "market" | "art" | "me" | "settings";
type SettingsSub = "general" | "runtime" | "wiki";

export function KakaoShell(props: { onLogout?: () => void }) {
  const [tab, setTab] = createSignal<Tab>("chat");
  const [sub, setSub] = createSignal<SettingsSub>("general");

  const [agents, { refetch: refetchAgents, mutate: mutateAgents }] = createResource<AgentRow[]>(() => invoke("agents_list"));
  const [peers] = createResource<PeerDto[]>(() => invoke("peers_list"), { initialValue: [] });
  const [recent] = createResource<MessageDto[]>(() => invoke("messages_recent", { limit: 100 }), { initialValue: [] });
  // 작업환경(tmux) 곁뷰 + 로컬/원격 머신 판정 데이터 소스 — sessions 라우트(이 머신 tmux+워크트리·machine 정보). 동적 only.
  const [sessions] = createResource<SessionsDto | null>(() => invoke("sessions"), { initialValue: null });

  const [selected, setSelected] = createSignal<string | null>(null);
  const [search, setSearch] = createSignal("");

  // ── 로스터 그룹 접기/펴기 ──────────────────────────────────────────────────
  // 그룹 헤더(머신·분류 서브그룹) 클릭 → 그 아래 row 숨김/표시. 기본 = 펼침(false).
  //   상태는 그룹별 localStorage 키(oxg.roster.collapsed.<groupId>)로 영속 → 리로드 후 유지.
  //   카운트 배지는 접혀도 헤더에 계속 보이고, 셰브론(▸/▾)으로 상태 표시.
  const COLLAPSE_KEY = (gid: string) => `oxg.roster.collapsed.${gid}`;
  const readCollapsed = (gid: string): boolean => {
    try { return localStorage.getItem(COLLAPSE_KEY(gid)) === "1"; } catch { return false; }
  };
  // 시그널 맵으로 반응성 보장(For/Show 가 토글 시 즉시 재렌더).
  const [collapseMap, setCollapseMap] = createSignal<Record<string, boolean>>({});
  const isCollapsed = (gid: string): boolean => collapseMap()[gid] ?? readCollapsed(gid);
  const toggleCollapse = (gid: string) => {
    const next = !isCollapsed(gid);
    setCollapseMap((prev) => ({ ...prev, [gid]: next }));
    try { localStorage.setItem(COLLAPSE_KEY(gid), next ? "1" : "0"); } catch { /* localStorage 불가 환경 무시 */ }
  };
  const chevron = (gid: string) => (isCollapsed(gid) ? "▸" : "▾");
  // 곁뷰: a2a(협업) / tmux(작업환경) — 상호배타. 목업 .side 두 패널.
  const [sideA2A, setSideA2A] = createSignal(false);
  const [sideTmux, setSideTmux] = createSignal(false);
  const [roomCfgOpen, setRoomCfgOpen] = createSignal(false);
  const [addOpen, setAddOpen] = createSignal(false);
  // rc.334 Phase 4a — 친구 추가 choice 모달(🖥 머신 추가 한쪽 / 🤝 에이전트 추가 상호 / 🌐 외부 A2A).
  //   addOpen(로컬 에이전트 신규 등록 = AddAgentModal)과 별개. friendOpen 은 AddFriendModal.
  const [friendOpen, setFriendOpen] = createSignal(false);
  // 🤝 4b — 에이전트 사용 요청 inbox(소유자 받은 요청 수락·가격책정 / 내가 보낸 요청 상태).
  const [reqInboxOpen, setReqInboxOpen] = createSignal(false);
  // 받은 요청(소유자) 중 pending 개수 → 버튼 배지. 라이브 폴링 없이 가벼운 단발 조회.
  const [reqPending, { refetch: refetchReqPending }] = createResource<number>(
    async () => {
      try {
        const r = await invoke<{ requests?: { status?: string }[] }>("agent_requests_list", { role: "incoming" });
        return (r?.requests ?? []).filter((x) => x.status === "pending").length;
      } catch {
        return 0;
      }
    },
    { initialValue: 0 },
  );

  const peerMap = createMemo(() => {
    const m = new Map<string, PeerDto>();
    for (const p of peers() ?? []) m.set(p.alias.toLowerCase(), p);
    return m;
  });
  const lastMsgByAlias = createMemo(() => {
    const map = new Map<string, MessageDto>();
    const aliases = (agents() ?? []).map((a) => a.alias);
    for (const m of recent() ?? []) {
      const s = (m.sender || "").toLowerCase();
      const cid = (m.conversation_id || "").toLowerCase();
      for (const a of aliases) {
        const al = a.toLowerCase();
        if (s === al || s === `peer:${al}` || cid.includes(al)) {
          const cur = map.get(al);
          if (!cur || Date.parse(m.timestamp) > Date.parse(cur.timestamp)) map.set(al, m);
        }
      }
    }
    return map;
  });

  const isPrimary = (a: AgentRow) => (a.classification ?? "") === "primary";

  // 검색 필터(대화명·이름·역할·머신·ai_type).
  const matchSearch = (a: AgentRow): boolean => {
    const q = search().trim().toLowerCase();
    if (!q) return true;
    return [a.alias, a.display_name, a.role, a.machine, a.ai_type].some((f) => (f ?? "").toLowerCase().includes(q));
  };

  // ── 로컬/원격(머신) 판정 ────────────────────────────────────────────────
  // 이 머신(데몬이 도는 머신) = 로컬. machine 필드가 비었거나 이 머신을 가리키면 로컬.
  //   현재 머신 후보: "seoul"/"서울"/"local" + sessions().machine.alias/hostname.
  //   머신 값이 다르면 REMOTE — 그 머신명으로 그룹화(마스터 피드백: 평평한 목록 X).
  const selfMachineNames = createMemo<string[]>(() => {
    const m = sessions()?.machine;
    const names = ["seoul", "서울", "local", "server-seoul"];
    if (m?.alias) names.push(m.alias.toLowerCase());
    if (m?.hostname) names.push(m.hostname.toLowerCase());
    return [...new Set(names.filter(Boolean).map((n) => n.toLowerCase()))];
  });
  const isLocalMachine = (machine: string | null | undefined): boolean => {
    const mm = (machine ?? "").trim().toLowerCase();
    if (!mm) return true; // machine 미설정 = 로컬(기존 로컬 에이전트).
    return selfMachineNames().some((n) => n && (mm === n || mm.includes(n) || n.includes(mm)));
  };

  // ── 원격 머신 "실제 연결" 판정 (마스터 결정: 연결된 머신만 표시, 미연결은 숨김 — "미연결" 라벨 X) ──
  //   원격 머신은 그 머신을 가리키는 *실제 peer 연결*이 있을 때만 연결로 간주한다.
  //   실제 원격 peer = address 의 host 가 이 머신(로컬 tailscale_ip)·localhost·loopback 이 아닌 peer.
  //   (단지 machine=X 라벨이 붙은 에이전트만으로는 연결 아님 — seoul 에 등록된 잘만 라벨 에이전트가 그 예.)
  //   현재 모든 peer address 가 로컬 seoul IP → 연결된 원격 머신 0 → 잘만 라벨 에이전트 전부 숨김.
  //   진짜 원격 머신(다른 IP 의 peer)이 추가되면 그 머신명이 자동으로 연결 집합에 들어와 표시된다.
  const localHostTokens = createMemo<string[]>(() => {
    const out: string[] = ["127.0.0.1", "localhost", "::1", "0.0.0.0"];
    const ip = sessions()?.machine?.tailscale_ip;
    if (ip) out.push(ip.toLowerCase());
    const host = sessions()?.machine?.hostname;
    if (host) out.push(host.toLowerCase());
    return [...new Set(out.filter(Boolean))];
  });
  const peerAddrIsRemote = (address: string | null | undefined): boolean => {
    const a = (address ?? "").trim().toLowerCase();
    if (!a) return false;
    // host 추출 (scheme·port·path 제거).
    let host = a.replace(/^[a-z]+:\/\//, "").split("/")[0].split(":")[0];
    if (!host) return false;
    return !localHostTokens().some((t) => t && (host === t || host.includes(t) || t.includes(host)));
  };
  // 실제 원격 peer 가 가리키는 머신명 집합(소문자). peer.machine 라벨 우선; 없으면 attribution 불가 → 제외.
  const connectedRemoteMachines = createMemo<Set<string>>(() => {
    const s = new Set<string>();
    for (const p of peers() ?? []) {
      if (!peerAddrIsRemote(p.address)) continue; // 로컬 IP peer = 원격 연결 아님.
      const m = (p.machine ?? "").trim().toLowerCase();
      if (m && !isLocalMachine(m)) s.add(m);
    }
    return s;
  });
  const isRemoteMachineConnected = (machine: string): boolean => {
    const mm = machine.trim().toLowerCase();
    if (!mm) return false;
    const set = connectedRemoteMachines();
    return [...set].some((n) => n && (mm === n || mm.includes(n) || n.includes(mm)));
  };

  // 분류 키(👑 primary / 📁 project / ⚙️ special / 📁 분류 미지정).
  //   classification 이 명시되면 그대로. null/미상이면 role 로 약하게 추정, 그래도 모르면 "unknown"(미지정).
  //   primary/special 을 날조하지 않는다(데이터 sparse 정직 반영).
  type ClsKey = "primary" | "project" | "special" | "unknown";
  const classKey = (a: AgentRow): ClsKey => {
    const c = (a.classification ?? "").toLowerCase();
    if (c === "primary") return "primary";
    if (c === "special") return "special";
    if (c === "project") return "project";
    const role = (a.role ?? "").toLowerCase();
    if (/primary|프라이머리|통합관리|orchestrat/.test(role)) return "primary";
    if (/special|특수|시스템|system/.test(role)) return "special";
    return "unknown";
  };
  const CLS_GROUPS: { key: ClsKey; title: string }[] = [
    { key: "primary", title: "👑 프라이머리" },
    { key: "project", title: "📁 프로젝트 에이전트" },
    { key: "special", title: "⚙️ 특수 에이전트" },
    { key: "unknown", title: "📁 분류 미지정" },
  ];
  const tsOf = (a: AgentRow) => { const m = lastMsgByAlias().get(a.alias.toLowerCase()); return m ? Date.parse(m.timestamp) : 0; };
  const sortRows = (rows: AgentRow[]) => [...rows].sort((x, y) => tsOf(y) - tsOf(x));
  const bucketByClass = (rows: AgentRow[]): Record<ClsKey, AgentRow[]> => {
    const by: Record<ClsKey, AgentRow[]> = { primary: [], project: [], special: [], unknown: [] };
    for (const a of rows) by[classKey(a)].push(a);
    (Object.keys(by) as ClsKey[]).forEach((k) => { by[k] = sortRows(by[k]); });
    return by;
  };

  // 로컬 머신 그룹 — 분류별 버킷.
  const localGroups = createMemo<Record<ClsKey, AgentRow[]>>(() =>
    bucketByClass((agents() ?? []).filter((a) => matchSearch(a) && isLocalMachine(a.machine))),
  );
  // 원격 머신 그룹 — [{ machine, byClass, hasPrimary }]. 프라이머리 보유 머신 먼저, 그다음 이름순.
  const remoteGroups = createMemo(() => {
    const buckets = new Map<string, AgentRow[]>();
    for (const a of (agents() ?? []).filter((a) => matchSearch(a) && !isLocalMachine(a.machine))) {
      const mach = (a.machine ?? "").trim() || "알 수 없는 머신";
      if (!buckets.has(mach)) buckets.set(mach, []);
      buckets.get(mach)!.push(a);
    }
    const groups = [...buckets.entries()]
      // 마스터 결정: 실제 연결된 원격 머신만 표시. 미연결 머신 라벨 그룹은 통째로 숨김("미연결" 표기 X).
      .filter(([machine]) => isRemoteMachineConnected(machine))
      .map(([machine, rows]) => {
        const byClass = bucketByClass(rows);
        return { machine, byClass, hasPrimary: byClass.primary.length > 0 };
      });
    groups.sort((a, b) => {
      if (a.hasPrimary !== b.hasPrimary) return a.hasPrimary ? -1 : 1;
      return a.machine.localeCompare(b.machine);
    });
    return groups;
  });
  const localCount = createMemo(() => (Object.values(localGroups()) as AgentRow[][]).reduce((n, arr) => n + arr.length, 0));

  const selAgent = createMemo(() => {
    const sel = selected(); if (!sel) return null;
    return (agents() ?? []).find((a) => a.alias === sel) ?? null;
  });

  const acpPreset = createMemo<AcpPreset | null>(() => {
    const a = selAgent(); if (!a) return null;
    return {
      adapter: aiTypeToAdapter(a.ai_type),
      cwd: a.project_path ?? null,
      execMode: a.execution_mode ?? null,
      label: a.alias,
      displayName: agentName(a),
      classification: a.classification ?? null,
      machine: a.machine ?? null,
      permMode: a.perm_mode ?? null,
      model: a.model ?? null,
      thinking: a.thinking ?? null,
    };
  });


  // 등록 에이전트들의 project_path 집합 — descendant 세션의 longest-prefix 귀속 판정용.
  const registeredCwds = createMemo<Set<string>>(() => {
    const s = new Set<string>();
    for (const a of agents() ?? []) {
      const p = normPath((a.project_path ?? "").trim());
      if (p) s.add(p);
    }
    return s;
  });

  // 선택 에이전트의 tmux 세션 — cwd 매칭 우선 + alias 매칭 보조(TalkTab selSessions 로직 이식).
  const selSessions = createMemo<DetectedSession[]>(() => {
    const alias = (selected() ?? "").toLowerCase();
    if (!alias) return [];
    const convoCwd = normPath((selAgent()?.project_path ?? "").trim());
    const regs = registeredCwds();
    const all = sessions()?.sessions ?? [];
    return all.filter((s) => {
      if (s.kind !== "tmux") return false;
      const sCwd = s.cwd ? normPath(s.cwd.trim()) : "";
      if (convoCwd && sCwd) {
        if (sCwd === convoCwd) return true;
        if (sCwd.startsWith(convoCwd + "/") && !isTooBroadPath(convoCwd)) {
          // longest-prefix: 더 구체적인 등록 에이전트가 있으면 그쪽 것(prefix-ownership leak 방지).
          let closest = convoCwd;
          for (const r of regs) {
            if (r === convoCwd) continue;
            if ((sCwd === r || sCwd.startsWith(r + "/")) && r.length > closest.length) closest = r;
          }
          if (closest === convoCwd) return true;
        }
      }
      const aid = (s.agent_id ?? "").toLowerCase();
      const disp = (s.display ?? "").toLowerCase();
      const ident = (s.identifier ?? "").toLowerCase();
      return aid === alias || disp === alias || ident === alias || ident === `tmux:${alias}`;
    });
  });

  // 매칭 세션들의 nested worktrees 합집합(path 기준 dedup).
  const selWorktrees = createMemo<{ path: string; branch?: string | null }[]>(() => {
    const seen = new Set<string>();
    const out: { path: string; branch?: string | null }[] = [];
    for (const s of selSessions()) {
      for (const w of s.worktrees ?? []) {
        if (w.path && !seen.has(w.path)) { seen.add(w.path); out.push(w); }
      }
    }
    return out;
  });

  const baseName = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

  // 라이브 pane 캡처 — 곁뷰가 열려 있고 세션이 있으면 첫 세션의 화면을 capture-pane 으로 가져온다.
  //   GET /v1/gui/sessions/{identifier}/screen → { content(ANSI), lines, source_note }.
  const captureTarget = createMemo<DetectedSession | null>(() => (sideTmux() ? (selSessions()[0] ?? null) : null));
  const [paneScreen] = createResource(
    () => captureTarget()?.identifier ?? null,
    async (identifier) => {
      try {
        const r = await invoke("session_screen", { identifier });
        return r as { content?: string; lines?: number; source_note?: string } | null;
      } catch { return null; }
    },
  );

  // tmux 라이브 새 창 열기 (?tmux=identifier).
  function openTmuxPopout(identifier: string, display: string) {
    const url = `${location.origin}${location.pathname}?tmux=${encodeURIComponent(identifier)}&label=${encodeURIComponent(display)}`;
    const w = window.open("", `oxgtmux_${identifier}`, "width=820,height=620");
    if (!w) { location.href = url; return; }
    w.location.href = url;
    // A2A 자동 팝업과 동일한 예의 — 새 창을 열되 opener 포커스를 되돌려준다(포커스 가로채기 금지).
    try { window.focus(); } catch { /* noop */ }
  }

  // 에이전트 전환 시 곁뷰 닫기.
  createEffect(() => { selected(); setSideA2A(false); setSideTmux(false); });

  // 로드 시 프라이머리(👑/⭐) 자동 선택 — 목업처럼 대화가 바로 보이게.
  //   프라이머리가 없으면 placeholder 유지(자동 선택 안 함).
  let autoPicked = false;
  createEffect(() => {
    if (autoPicked || selected()) return;
    const list = agents();
    if (!list || list.length === 0) return;
    const prim = list.find(isPrimary);
    if (prim) { autoPicked = true; pick(prim.alias); }
  });

  function pick(alias: string) {
    setSelected(alias);
    void invoke("acp_conv_read", { key: alias }).catch(() => {});
    mutateAgents((prev) => (prev ?? []).map((a) => (a.alias === alias ? { ...a, unread: 0 } : a)));
  }

  function preview(a: AgentRow): string {
    const m = lastMsgByAlias().get(a.alias.toLowerCase());
    if (m && m.body) return m.body.replace(/\n+/g, " ").trim();
    return a.role || a.description || a.machine || "—";
  }
  function previewTime(a: AgentRow): string {
    const m = lastMsgByAlias().get(a.alias.toLowerCase());
    return m ? fmtPreviewTime(m.timestamp) : "";
  }

  const onlineFor = (a: AgentRow) => isOnline(peerMap().get(a.alias.toLowerCase())?.last_seen);
  const friends = createMemo<AgentRow[]>(() => (agents() ?? []));

  // 명부 row — 로컬/원격 양쪽 그룹에서 공유. primaryHint = primary 서브그룹에서 렌더되는지(아바타 ⭐).
  const roomRow = (a: AgentRow, primaryHint: boolean) => {
    const primary = primaryHint || isPrimary(a);
    return (
      <div class="room" classList={{ on: selected() === a.alias }} onClick={() => pick(a.alias)}>
        <div class="av" style={`background:${avatarBg(a, primary)}`}>{primary ? "⭐" : agentName(a).slice(0, 1).toUpperCase()}</div>
        <div class="meta">
          <div class="nm">
            {agentName(a)}
            <Show when={primary}><span class="tag pri">프라이머리</span></Show>
            <Show when={(a.classification ?? "") === "security"}><span class="tag lock">🔒 보안</span></Show>
            <Show when={(a.classification ?? "") === "group"}><span class="tag grp">그룹</span></Show>
          </div>
          <div class="ms">
            <Show when={onlineFor(a)}><span style="color:var(--green)">● </span></Show>
            {preview(a)}
          </div>
        </div>
        <div class="rt">
          <div class="tm">{previewTime(a)}</div>
          <Show when={(a.unread ?? 0) > 0}><div class="badge">{(a.unread ?? 0) > 99 ? "99+" : a.unread}</div></Show>
        </div>
      </div>
    );
  };

  // 현황 카드 집계 — 라이브.
  const onlineCount = createMemo(() => (agents() ?? []).filter(onlineFor).length);
  const primaryAgent = createMemo(() => (agents() ?? []).find(isPrimary) ?? null);

  function openTab(t: Tab) {
    setTab(t);
  }

  const SETTINGS_SUB: { id: SettingsSub; ic: string; label: string }[] = [
    { id: "general", ic: "⚙️", label: "일반" },
    { id: "runtime", ic: "🧠", label: "하네스" },
    { id: "wiki", ic: "📚", label: "위키" },
  ];

  return (
    <div class="oxg-app app">
      {/* ── 레일 ── (정본 .rail) */}
      <div class="rail">
        <button class="me" classList={{ on: tab() === "me" }} title="내 프로필 · 친구(에이전트) 관리" onClick={() => openTab("me")}>나</button>
        <button classList={{ on: tab() === "chat" }} title="채팅" onClick={() => openTab("chat")}>💬</button>
        <button classList={{ on: tab() === "dash" }} title="현황" onClick={() => openTab("dash")}><span class="dot" />📊</button>
        <button classList={{ on: tab() === "flow" }} title="워크플로우" onClick={() => openTab("flow")}>🔀</button>
        <button classList={{ on: tab() === "market" }} title="마켓" onClick={() => openTab("market")}>🛒</button>
        <button classList={{ on: tab() === "art" }} title="아티팩트" onClick={() => openTab("art")}>📎</button>
        <div class="sp" />
        <button classList={{ on: tab() === "settings" }} title="설정" onClick={() => openTab("settings")}>⚙️</button>
        <Show when={props.onLogout}>
          <button title="잠금" onClick={() => props.onLogout!()}>🔒</button>
        </Show>
      </div>

      {/* ── 채팅 목록 ── (정본 .list / .rooms / .room) */}
      <div class="list" classList={{ hide: tab() !== "chat" }}>
        {/* 🤝 4b — 채팅 목록 헤더에 "사용 요청" inbox 진입점. 받은 요청(소유자) pending 개수 배지. */}
        <h2 style="display:flex;align-items:center;justify-content:space-between">
          <span>채팅</span>
          <button
            class="kk-reqinbox-btn"
            title="🤝 에이전트 사용 요청 (받은 요청 수락·가격책정 / 보낸 요청 상태)"
            style="position:relative;background:none;border:1px solid var(--line,#2a3a48);border-radius:8px;padding:3px 8px;font-size:12px;color:var(--muted,#9bb0c0);cursor:pointer;display:inline-flex;align-items:center;gap:4px"
            onClick={() => setReqInboxOpen(true)}
          >
            🤝 사용 요청
            <Show when={(reqPending() ?? 0) > 0}>
              <span style="position:absolute;top:-6px;right:-6px;min-width:16px;height:16px;padding:0 4px;border-radius:8px;background:#f85149;color:#fff;font-size:10px;line-height:16px;text-align:center;font-weight:700">
                {(reqPending() ?? 0) > 99 ? "99+" : reqPending()}
              </span>
            </Show>
          </button>
        </h2>
        <input class="search" type="text" value={search()} onInput={(e) => setSearch(e.currentTarget.value)} placeholder="🔍  에이전트·대화방 검색" />
        <div class="rooms">
          <Show when={!agents.loading} fallback={<div style="padding:16px;color:var(--muted);font-size:13px">불러오는 중…</div>}>
            {/* ── 이 머신 (로컬) — 데몬이 도는 머신. 👑 프라이머리 / 📁 프로젝트 / ⚙️ 특수 / 미지정. ── */}
            <Show when={localCount() > 0}>
              <div class="group-title machine collapsible" classList={{ collapsed: isCollapsed("local:machine") }} onClick={() => toggleCollapse("local:machine")}>
                <span class="gt-chev">{chevron("local:machine")}</span>🖥 이 머신 (로컬) <span class="gt-sub">({localCount()})</span>
              </div>
            </Show>
            <Show when={!isCollapsed("local:machine")}>
              <For each={CLS_GROUPS}>
                {(g) => (
                  <Show when={(localGroups()[g.key] ?? []).length > 0}>
                    <div class="group-title sub collapsible" classList={{ collapsed: isCollapsed(`local:${g.key}`) }} onClick={() => toggleCollapse(`local:${g.key}`)}>
                      <span class="gt-chev">{chevron(`local:${g.key}`)}</span>{g.title} <span class="gt-sub">({localGroups()[g.key].length})</span>
                    </div>
                    <Show when={!isCollapsed(`local:${g.key}`)}>
                      <For each={localGroups()[g.key]}>{(a) => roomRow(a, g.key === "primary")}</For>
                    </Show>
                  </Show>
                )}
              </For>
            </Show>

            {/* ── 다른 머신 (원격) — 머신별 그룹. 각 머신 안에서 프라이머리→프로젝트→특수→미지정. ── */}
            <For each={remoteGroups()}>
              {(mg) => {
                const machGid = `remote:${mg.machine}`;
                const machCount = (Object.values(mg.byClass) as AgentRow[][]).reduce((n, arr) => n + arr.length, 0);
                return (
                  <>
                    <div class="group-title machine collapsible" classList={{ collapsed: isCollapsed(machGid) }} onClick={() => toggleCollapse(machGid)}>
                      <span class="gt-chev">{chevron(machGid)}</span>🖥 {mg.machine} <span class="gt-sub">(다른 머신 · {machCount})</span>
                    </div>
                    <Show when={!isCollapsed(machGid)}>
                      <For each={CLS_GROUPS}>
                        {(g) => {
                          const subGid = `${machGid}:${g.key}`;
                          return (
                            <Show when={(mg.byClass[g.key] ?? []).length > 0}>
                              <div class="group-title sub collapsible" classList={{ collapsed: isCollapsed(subGid) }} onClick={() => toggleCollapse(subGid)}>
                                <span class="gt-chev">{chevron(subGid)}</span>{g.title} <span class="gt-sub">({mg.byClass[g.key].length})</span>
                              </div>
                              <Show when={!isCollapsed(subGid)}>
                                <For each={mg.byClass[g.key]}>{(a) => roomRow(a, g.key === "primary")}</For>
                              </Show>
                            </Show>
                          );
                        }}
                      </For>
                    </Show>
                  </>
                );
              }}
            </For>

            {/* rc.334 Phase 4a — 추가는 두 흐름으로 분리.
                ① 새 에이전트(이 머신) = AddAgentModal(로컬 신규 등록 · 새 ACP).
                ② 머신/에이전트 추가 = AddFriendModal(🖥 내 머신 한쪽 · 🤝 상대 에이전트 상호 · 🌐 외부 A2A). */}
            <div class="room" onClick={() => setAddOpen(true)} title="이 머신에 새 에이전트 등록 (새 ACP)">
              <div class="av grp">＋</div>
              <div class="meta"><div class="nm">새 에이전트 (이 머신)</div><div class="ms">로컬 등록 · 새 ACP</div></div>
            </div>
            <div class="room" onClick={() => setFriendOpen(true)} title="🖥 머신 추가(내 머신·한쪽) · 🤝 에이전트 추가(상대·상호·격리·가격) · 🌐 외부 A2A">
              <div class="av grp">🤝</div>
              <div class="meta"><div class="nm">머신 · 에이전트 추가</div><div class="ms">🖥 머신(한쪽) · 🤝 에이전트(상호) · 🌐 외부</div></div>
            </div>
          </Show>
        </div>
      </div>

      {/* ── 대화창 ── (정본 .chat / .chead / .mini / .comp / .side) */}
      <div class="chat" classList={{ hide: tab() !== "chat" }} style="position:relative">
        <Show
          when={selAgent()}
          fallback={
            <div style="flex:1;display:flex;align-items:center;justify-content:center;color:#3c5266;font-size:14px">
              좌측에서 대화할 에이전트를 선택하세요.
            </div>
          }
        >
          {(a) => (
            <>
              {/* 🔧 Fix#2 — 중복 헤더 제거: 예전엔 여기 별도 .chead(이름+역할+작업환경/협업/방설정) 를
                  렌더하고, 그 아래 AcpConversation 이 자기 .chat-top(이름+스트리밍/ACP/아티팩트/새) 를
                  또 렌더해 "Starian" 헤더가 두 줄로 보였다. 이제 단일 헤더 = AcpConversation 의
                  .chat-top 하나만 쓰고, 작업환경/협업/방설정 버튼은 headerExtra 로 그 헤더에 합친다. */}

              {/* 대화 본문 — 라이브 ACP 세션(SSE 스트림). 목업 .msgs 영역을 ACP 엔진으로 구동.
                  단일 헤더(.chat-top)에 작업환경/협업/방설정 토글을 headerExtra 로 주입. */}
              <div class="oxg-acp-slot">
                <Show when={selected()} keyed>
                  {(_k) => (
                    <AcpConversation
                      preset={acpPreset()}
                      popoutAlias={a().alias}
                      headerExtra={() => (
                        <>
                          {/* 자주 쓰는 액션 = 아이콘 전용(title 툴팁). 줄바꿈 없이 한 줄에 들어가도록 컴팩트. */}
                          <span class="kk-acp-icon clk" classList={{ active: sideTmux() }} title="작업환경 (tmux · 사람 전용 터미널)" onClick={() => { setSideA2A(false); setSideTmux((v) => !v); }}>🖥</span>
                          <span class="kk-acp-icon clk" classList={{ active: sideA2A() }} title="협업 (A2A 에이전트간 대화)" onClick={() => { setSideTmux(false); setSideA2A((v) => !v); }}>🔗</span>
                          <span class="kk-acp-icon clk" title="방 설정 (하네스·역할·오케스트레이션)" onClick={() => setRoomCfgOpen(true)}>⚙️</span>
                        </>
                      )}
                      status={() => ({
                        folder: a().project_path ?? null,
                        role: a().role ?? null,
                        isPublic: !!a().is_public,
                        workflows: 0,
                      })}
                      onClose={() => setSelected(null)}
                    />
                  )}
                </Show>
              </div>

              {/* A2A 실시간 미니패널(정본 .a2a-mini) + 협업 곁뷰(.a2a-side) — A2AMiniPanel 이 둘 다 렌더.
                  .kk-a2a-mount 로 감싸 absolute 마운트 → strip 이 .chat-top 헤더 바로 아래(top:58px)에
                  한 줄로 정렬(flow-extra.css). 헤더→strip→메시지 순서 보장(중복 헤더 인상 제거). */}
              <div class="kk-a2a-mount">
                <A2AMiniPanel
                  selfAlias={agentName(a())}
                  open={sideA2A}
                  onOpen={() => { setSideTmux(false); setSideA2A(true); }}
                  onClose={() => setSideA2A(false)}
                />
              </div>

              {/* 작업환경(tmux) 곁뷰 — 정본 .side#sideTmux. 라이브 sessions 라우트 배선.
                  세션 있으면: 세션·워크트리 목록 + 라이브 pane 캡처(다크 .term).
                  세션 없으면(ACP-only 에이전트): 검은 void 대신 읽기 쉬운 light 빈 상태. */}
              <div class="side" classList={{ show: sideTmux() }}>
                <h3>🖥 {agentName(a())} 작업환경 (tmux · 사람 전용)
                  <span class="x" onClick={() => setSideTmux(false)}>✕</span>
                </h3>
                <Show
                  when={selSessions().length > 0}
                  fallback={
                    <div class="kk-workenv-empty">
                      <div class="kk-we-icon">🖥</div>
                      <div class="kk-we-title">활성 tmux 작업환경이 없습니다</div>
                      <p class="kk-we-body">
                        이 에이전트는 ACP로 동작합니다 — 사람 전용 tmux 터미널이 떠 있지 않습니다.
                        에이전트와의 대화는 이 채팅창에서, 에이전트 간 협업은 상단 <b>🔗 협업</b>(A2A)에서 진행하세요.
                      </p>
                      <div class="kk-we-meta">
                        <Show when={a().project_path}>
                          <div class="kk-we-row"><span class="k">폴더</span><span class="v" title={a().project_path!}>{a().project_path}</span></div>
                        </Show>
                        <Show when={a().machine}>
                          <div class="kk-we-row"><span class="k">머신</span><span class="v">{a().machine}</span></div>
                        </Show>
                        <div class="kk-we-row"><span class="k">실행</span><span class="v">ACP · {a().alias}</span></div>
                      </div>
                      <p class="kk-we-hint">
                        💡 이 폴더(또는 그 하위)에서 <code>tmux</code> 세션이 떠 있으면 여기에 자동으로 나타납니다.
                      </p>
                    </div>
                  }
                >
                  <div class="kk-workenv">
                    {/* 라이브 pane — 가장 위·가장 크게. capture-pane(다크 .term) + 새 창 버튼. */}
                    <div class="kk-we-sec kk-we-sec-term">
                      <div class="kk-we-sech">
                        라이브 화면 <span class="kk-we-sub">{captureTarget() ? captureTarget()!.display || captureTarget()!.identifier : ""}</span>
                        <Show when={captureTarget()}>
                          <button
                            class="kk-we-pop"
                            title="이 세션 라이브 화면을 새 창에서 크게 보기"
                            onClick={() => openTmuxPopout(captureTarget()!.identifier, captureTarget()!.display || captureTarget()!.identifier)}
                          >🔳 새 창에서 보기</button>
                        </Show>
                      </div>
                      <div class="term">
                        <Show
                          when={paneScreen()?.content}
                          fallback={
                            <span class="c">{paneScreen.loading ? "# 화면 불러오는 중…" : "# 캡처할 화면이 없습니다 (세션이 비어있거나 접근 불가)."}</span>
                          }
                        >
                          {paneScreen()!.content}
                        </Show>
                      </div>
                      <Show when={paneScreen()?.source_note}>
                        <div class="kk-we-note">{paneScreen()!.source_note}{paneScreen()?.lines ? ` · ${paneScreen()!.lines}줄` : ""}</div>
                      </Show>
                    </div>

                    {/* 세션 목록 — 클릭 시 라이브 화면 새 창. 여러 세션이면 보고 싶은 세션 선택. */}
                    <div class="kk-we-sec">
                      <div class="kk-we-sech">실행 중 tmux · {selSessions().length} <span class="kk-we-sub">(클릭 → 라이브 새 창)</span></div>
                      <For each={selSessions()}>
                        {(s) => (
                          <div class="kk-we-sess" title="클릭 → 라이브 화면 새 창" onClick={() => openTmuxPopout(s.identifier, s.display || s.identifier)}>
                            <span class="dot" />
                            <span class="nm">{s.display || s.identifier}</span>
                            <span class="sx">{s.kind}{(s.cwd ? ` · ${baseName(s.cwd)}` : "")}</span>
                            <span class="po" title="새 창에서 보기">🔳</span>
                          </div>
                        )}
                      </For>
                    </div>

                    {/* 워크트리 — 매칭 세션들의 nested git worktree. */}
                    <Show when={selWorktrees().length > 0}>
                      <div class="kk-we-sec">
                        <div class="kk-we-sech">워크트리 · {selWorktrees().length}</div>
                        <For each={selWorktrees()}>
                          {(w) => (
                            <div class="kk-we-wt" title={w.path}>
                              🌿 {baseName(w.path)}
                              <Show when={w.branch}><span class="b">{w.branch}</span></Show>
                            </div>
                          )}
                        </For>
                      </div>
                    </Show>
                  </div>
                </Show>
              </div>

              {/* 방 설정 모달 — 정본 modal(.mset). room_config get/set 배선. */}
              <Show when={roomCfgOpen()}>
                <RoomModal roomKey={a().alias} roomLabel={agentName(a())} onClose={() => setRoomCfgOpen(false)} />
              </Show>
            </>
          )}
        </Show>
      </div>

      {/* ── 현황 대시보드 ── (정본 .dash) */}
      <div class="dash" classList={{ show: tab() === "dash" }}>
        <h2>현황</h2>
        <div class="sub">전체 에이전트 · 활성 대화 · 보안방 — 사람이 한눈에 보고 제어</div>
        <div class="cards">
          <div class="card">
            <div class="t">🟢 온라인 에이전트</div>
            <div class="big">{onlineCount()}</div>
            <For each={(agents() ?? []).filter(onlineFor).slice(0, 3)}>
              {(a) => <div class="li"><span class="live" /> {agentName(a)}{a.role ? ` · ${a.role}` : ""}</div>}
            </For>
            <Show when={onlineCount() === 0}><div class="li" style="color:var(--muted)">온라인 에이전트 없음</div></Show>
          </div>
          <div class="card">
            <div class="t">👥 에이전트</div>
            <div class="big">{(agents() ?? []).length}</div>
            <Show when={primaryAgent()}><div class="li">⭐ {agentName(primaryAgent()!)} (프라이머리)</div></Show>
            <For each={(agents() ?? []).filter((a) => !isPrimary(a)).slice(0, 2)}>
              {(a) => <div class="li">🔵 {agentName(a)}</div>}
            </For>
            <div class="li" style="color:var(--muted)">사람 = 고권한 참가자</div>
          </div>
          <div class="card">
            <div class="t">🔒 보안 공유방</div>
            <div class="big" style="font-size:19px">방별 vault</div>
            <div class="li">대화 곁뷰 ▸ 🔒 보안방에서 키/파일 공유</div>
            <div class="li">멤버만 복호화 · 모든 접근 감사 기록</div>
            <div class="li" style="color:var(--amber)">⚠ 멤버 퇴장 시 키 회전</div>
          </div>
          <div class="card">
            <div class="t">⚙️ 기본 하네스</div>
            <div class="big" style="font-size:19px">claude-agent-acp</div>
            <div class="li">🌳 worktree · 🔒 격리 — 방별 override</div>
            <div class="li">새 A2A → 새 ACP 생성 시 적용</div>
            <div class="li" style="color:var(--muted)">설정 ▸ 하네스에서 전역 기본 변경</div>
          </div>
        </div>
      </div>

      {/* ── 나 — 프로필 / 친구(에이전트) 관리 ── (정본 .me-pane) */}
      <div class="me-pane" classList={{ show: tab() === "me" }}>
        <div class="me-prof">
          <div class="ava">나</div>
          <div>
            <div class="pn">나 <span class="tag pri">고권한 참가자</span></div>
            <div class="pa">
              {primaryAgent() ? `프라이머리 ACP: ${agentName(primaryAgent()!)}` : "프라이머리 ACP 미지정"}
            </div>
          </div>
          <button class="ed" onClick={() => openTab("settings")}>프로필 편집</button>
        </div>
        <div style="padding:12px 24px 0;font-size:12px;color:var(--muted)">왼쪽 채팅 목록 = 대화방 · 여기 = 내 프로필 + 친구(에이전트) 목록</div>
        <div class="me-sec">
          <h3>친구 (에이전트) <span style="color:var(--muted);font-weight:600;font-size:13px">{friends().length}</span></h3>
          <button class="add" onClick={() => setFriendOpen(true)}>＋ 머신 · 에이전트 추가</button>
        </div>
        <div class="friends">
          <For each={friends()}>
            {(a) => {
              const primary = isPrimary(a);
              return (
                <div class="friend">
                  <div class="av" style={`background:${avatarBg(a, primary)}`}>{primary ? "⭐" : agentName(a).slice(0, 1).toUpperCase()}</div>
                  <div>
                    <div class="fn">{agentName(a)}<Show when={primary}><span class="tag pri">프라이머리</span></Show></div>
                    <div class="fr">{a.role || a.description || "ACP"}{a.machine ? ` · ${a.machine}` : ""}</div>
                  </div>
                  <button class="mng" onClick={() => { pick(a.alias); openTab("chat"); }}>관리</button>
                </div>
              );
            }}
          </For>
        </div>
      </div>

      {/* ── 임베드 탭 (워크플로우/마켓/아티팩트/설정) — 목업 dash 영역에 기존 컴포넌트 ── */}
      <div class="embed-pane" classList={{ show: tab() === "flow" }}><Show when={tab() === "flow"}><FlowTab /></Show></div>
      <div class="embed-pane" classList={{ show: tab() === "market" }}><Show when={tab() === "market"}><MarketTab /></Show></div>
      <div class="embed-pane" classList={{ show: tab() === "art" }}>
        <Show when={tab() === "art"}>
          <div style="padding:24px;color:var(--muted);font-size:13px">📎 아티팩트 — 대화 곁뷰의 아티팩트 패널에서 파일·이미지를 보기·읽기·편집할 수 있습니다.</div>
        </Show>
      </div>
      <div class="embed-pane" classList={{ show: tab() === "settings" }}>
        <Show when={tab() === "settings"}>
          <div style="display:flex;gap:6px;padding:14px 18px 0">
            <For each={SETTINGS_SUB}>
              {(s) => (
                <div onClick={() => setSub(s.id)}
                  style={`cursor:pointer;padding:6px 14px;border-radius:8px;font-size:13px;${sub() === s.id ? "background:#eaf0f6;color:#16242f;font-weight:700;" : "color:var(--muted);"}`}>
                  {s.ic} {s.label}
                </div>
              )}
            </For>
          </div>
          <div style="padding:8px 0">
            <Show when={sub() === "general"}><ConfigTab /></Show>
            <Show when={sub() === "runtime"}><RuntimeTab /></Show>
            <Show when={sub() === "wiki"}><WikiTab /></Show>
          </div>
        </Show>
      </div>

      {/* 새 에이전트(이 머신) 등록 모달 */}
      <Show when={addOpen()}>
        <AddAgentModal
          onClose={() => setAddOpen(false)}
          onCreated={(alias) => { setAddOpen(false); void refetchAgents(); setSelected(alias); setTab("chat"); }}
        />
      </Show>

      {/* rc.334 Phase 4a — 머신/에이전트 추가 choice 모달.
          🖥 머신 추가 = 내 머신 한쪽 등록(전권) · 🤝 에이전트 추가 = 상대 에이전트 상호 요청(격리·소유자 가격 4b)
          · 🌐 외부 A2A. 머신·에이전트는 agents_register(friend) → 명부 새로고침. */}
      <Show when={friendOpen()}>
        <AddFriendModal
          onClose={() => setFriendOpen(false)}
          onCreated={(alias, kind) => {
            setFriendOpen(false);
            // 머신·에이전트 추가 둘 다 agents_register(friend) → 명부 새로고침. 외부는 localStorage(별 소스).
            if (kind === "machine" || kind === "agent") { void refetchAgents(); setSelected(alias); setTab("chat"); }
          }}
        />
      </Show>

      {/* 🤝 4b — 에이전트 사용 요청 inbox (소유자 수락+가격책정 / 요청자 상태). 닫을 때 명부+배지 새로고침. */}
      <Show when={reqInboxOpen()}>
        <AgentRequestsInbox onClose={() => { setReqInboxOpen(false); void refetchAgents(); void refetchReqPending(); }} />
      </Show>

      {/* 옛 하단 고정 캡션바(.note) 제거 — 대화 모델 캡션은 .chat-top 헤더 제목 아래 .chat-cap 로 이전. */}
    </div>
  );
}
