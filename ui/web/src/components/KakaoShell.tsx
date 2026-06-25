import { createSignal, createResource, createMemo, createEffect, onMount, onCleanup, For, Show } from "solid-js";
import { invoke, acpFetch } from "../api/client";
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
import { type DetectedSession, type SessionsDto, normPath, isTooBroadPath, expandHome, detectHome, aliasMatchesSession } from "./agentSessions";

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
  session_identifier?: string | null;
  is_public?: boolean | null;
  perm_mode?: string | null;
  model?: string | null;
  thinking?: string | null;
  execution_mode?: string | null;
  unread?: number | null;
}
interface PeerDto {
  alias: string;
  last_seen?: string;
  machine?: string;
  address?: string;
  display_name?: string | null;
  role?: string | null;
  cwd?: string | null;
  session_identifier?: string | null;
  session_status?: string | null;
  canonical_address?: string | null;
  canonical_name?: string | null;
  quarantined?: boolean;
}
// 현황 그리드 정본 로스터 entry — 백엔드 GET /v1/gui/roster 가 머신정규화·병합·dedup 후 반환.
//   canonical identity 당 1행. 프론트 merge 계층 없이 그대로 GridRow 로 매핑(thin renderer).
interface RosterEntryDto {
  canonical_address: string | null;
  primary_alias: string;          // 라우팅 alias (행의 alias = 액션/peer_send 키)
  display_name: string | null;
  role: string | null;
  machine: string | null;         // 이미 seoul/zalman 으로 정규화됨 (프론트 재정규화 금지)
  cwd: string | null;
  session_identifier: string | null; // 활성 세션 (sid)
  aliases: string[];              // 그룹 내 모든 소스 alias
  is_peer: boolean;
  has_agent: boolean;
  has_tmux: boolean;
  quarantined: boolean;
  status?: string | null;          // rc.360 — 생명주기: active / stopped / dead
  // ── Phase A 통합 현황 그리드 — 백엔드가 이미 제공하는 재무·평판 필드(스네이크). ──
  //   백엔드 RosterEntryDto 가 직렬화하지만 프론트 매핑에서 누락하던 것들을 그리드로 노출.
  wallet_balance?: number | null;  // 지갑잔액(전체 단위, 백엔드가 micro→단위 환산)
  income?: number | null;          // 수입
  expense?: number | null;         // 지출
  rating?: number | null;          // 별점(external_reputation.avg_rating)
  review_count?: number | null;    // 평가 수
  views?: number | null;           // 인지도(조회수) — Phase B: agent_metrics.views 백킹
  // ── Phase B 통합 현황 그리드 — agent_profiles 백킹(토큰단가/샘플). ──
  token_price?: number | null;     // 토큰단가(1M) — 외부 사용 시 1M 토큰당 단가
  sample_text?: string | null;     // 샘플 텍스트
  sample_url?: string | null;      // 샘플 URL(파일 또는 랜딩페이지)
}
interface MessageDto { id: string; sender: string; body: string; timestamp: string; conversation_id: string }

const agentName = (a: { display_name?: string | null; alias: string }) =>
  (a.display_name && a.display_name.trim()) || a.alias;

// ── Phase A 현황 그리드 — 숫자 셀 표시. null → "—", 소수 최대 4자리(불필요한 0 제거). ──
const fmtNum = (n: number | null | undefined): string =>
  (n == null || !Number.isFinite(n)) ? "—" : String(Math.round(n * 1e4) / 1e4);

// ai_type → 목업 .av 배경(목업의 하드코딩 색을 라이브 ai_type 으로 매핑).
const AI_BG: Record<string, string> = {
  claude: "#d97757", codex: "#10a37f", gemini: "#4285f4", ollama: "#5b5b66", hermes: "#7c5cff",
};
function avatarBg(a: AgentRow, isPrimary: boolean): string {
  if (isPrimary) return "linear-gradient(135deg,#ffd84d,#ff9e2c)";
  return (a.ai_type && AI_BG[a.ai_type.toLowerCase()]) || "#7c8ba1";
}

// ── 라이브 term ANSI 렌더러 (FIX 2) ──
//   tmux capture-pane -e 출력의 SGR(\x1b[...m) 색 코드를 실제 색 span 으로 변환.
//   그 외 escape(커서이동·OSC·기타 CSI)는 화면에 garbage 로 보이지 않게 strip.
//   다크 term 배경(#0c1116)에 잘 보이는 팔레트로 매핑.
const ANSI_16: string[] = [
  "#1c1f24", "#f0726a", "#5fd07a", "#e6c668", "#6fb3ff", "#c98ff0", "#5fd0c8", "#c9d4de", // 0-7 normal
  "#5b6b7a", "#ff857c", "#74e890", "#f4dd86", "#8fc6ff", "#dba6ff", "#74e8e0", "#eef3f8", // 8-15 bright
];
// xterm 256 -> hex (16-231 색 큐브, 232-255 grayscale)
function ansi256(n: number): string {
  if (n < 16) return ANSI_16[n];
  if (n >= 232) { const v = 8 + (n - 232) * 10; return `rgb(${v},${v},${v})`; }
  const i = n - 16;
  const r = Math.floor(i / 36), g = Math.floor((i % 36) / 6), b = i % 6;
  const c = (x: number) => (x === 0 ? 0 : 55 + x * 40);
  return `rgb(${c(r)},${c(g)},${c(b)})`;
}
interface AnsiStyle { fg?: string; bg?: string; bold?: boolean; dim?: boolean; italic?: boolean; underline?: boolean }
interface AnsiSeg { text: string; style: AnsiStyle }
// SGR 파라미터 배열을 현재 style 에 적용.
function applySgr(st: AnsiStyle, params: number[]): AnsiStyle {
  const ns = { ...st };
  for (let i = 0; i < params.length; i++) {
    const p = params[i];
    if (p === 0) { return {}; }                       // reset
    else if (p === 1) ns.bold = true;
    else if (p === 2) ns.dim = true;
    else if (p === 3) ns.italic = true;
    else if (p === 4) ns.underline = true;
    else if (p === 22) { ns.bold = false; ns.dim = false; }
    else if (p === 23) ns.italic = false;
    else if (p === 24) ns.underline = false;
    else if (p >= 30 && p <= 37) ns.fg = ansi256(p - 30);
    else if (p >= 90 && p <= 97) ns.fg = ansi256(p - 90 + 8);
    else if (p >= 40 && p <= 47) ns.bg = ansi256(p - 40);
    else if (p >= 100 && p <= 107) ns.bg = ansi256(p - 100 + 8);
    else if (p === 39) ns.fg = undefined;
    else if (p === 49) ns.bg = undefined;
    else if (p === 38 || p === 48) {
      const isFg = p === 38;
      if (params[i + 1] === 5) { const col = ansi256(params[i + 2] ?? 0); if (isFg) ns.fg = col; else ns.bg = col; i += 2; }
      else if (params[i + 1] === 2) { const col = `rgb(${params[i + 2] ?? 0},${params[i + 3] ?? 0},${params[i + 4] ?? 0})`; if (isFg) ns.fg = col; else ns.bg = col; i += 4; }
    }
  }
  return ns;
}
// 원시 ANSI 문자열 -> 스타일 세그먼트 배열. SGR 만 해석, 나머지 escape 는 제거.
function parseAnsi(raw: string): AnsiSeg[] {
  const segs: AnsiSeg[] = [];
  let style: AnsiStyle = {};
  let buf = "";
  const flush = () => { if (buf) { segs.push({ text: buf, style }); buf = ""; } };
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (ch === "\x1b") {
      const next = raw[i + 1];
      if (next === "[") {
        // CSI ... final-byte
        let j = i + 2;
        while (j < raw.length && !/[A-Za-z]/.test(raw[j])) j++;
        const final = raw[j];
        const body = raw.slice(i + 2, j);
        if (final === "m") { flush(); const params = body === "" ? [0] : body.split(";").map((s) => parseInt(s, 10) || 0); style = applySgr(style, params); }
        // 그 외 CSI(커서이동·지우기 등) 는 무시(strip)
        i = j;
      } else if (next === "]") {
        // OSC ... BEL(\x07) 또는 ST(\x1b\\) 까지 strip
        let j = i + 2;
        while (j < raw.length && raw[j] !== "\x07" && !(raw[j] === "\x1b" && raw[j + 1] === "\\")) j++;
        if (raw[j] === "\x1b") j++;
        i = j;
      } else {
        // 단독 escape — strip 1바이트
        i += 1;
      }
    } else {
      buf += ch;
    }
  }
  flush();
  return segs;
}
function segCss(s: AnsiStyle): string {
  const parts: string[] = [];
  if (s.fg) parts.push(`color:${s.fg}`);
  if (s.bg) parts.push(`background:${s.bg}`);
  return parts.join(";");
}
function segClass(s: AnsiStyle): string {
  const c: string[] = [];
  if (s.bold) c.push("ansi-b");
  if (s.dim) c.push("ansi-dim");
  if (s.italic) c.push("ansi-i");
  if (s.underline) c.push("ansi-u");
  return c.join(" ");
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
  const [peers, { refetch: refetchPeers }] = createResource<PeerDto[]>(() => invoke("peers_list"), { initialValue: [] });
  const [recent] = createResource<MessageDto[]>(() => invoke("messages_recent", { limit: 100 }), { initialValue: [] });
  // 작업환경(tmux) 곁뷰 + 로컬/원격 머신 판정 데이터 소스 — sessions 라우트(이 머신 tmux+워크트리·machine 정보). 동적 only.
  const [sessions, { refetch: refetchSessions }] = createResource<SessionsDto | null>(() => invoke("sessions"), { initialValue: null });
  // 현황 그리드 정본 소스 — 백엔드 roster 엔드포인트(병합·dedup·머신정규화 완료). 그리드는 thin renderer.
  const [roster, { refetch: refetchRoster }] = createResource<RosterEntryDto[]>(() => invoke("roster"), { initialValue: [] });

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

  // 이 머신 라벨 — sessions().machine 우선, 폴백 "이 머신".
  //   ⚠️ unifiedRows(아래 createMemo)가 즉시평가 시 이를 호출하므로 *반드시 그 위*에 선언.
  //   (이전엔 786행에 있어 const TDZ → "Cannot access before initialization" 마운트 크래시.)
  const localMachineName = createMemo(() =>
    sessions()?.machine?.alias || sessions()?.machine?.hostname || "이 머신",
  );

  // (STEP B 머신명 정규화 canonMachine/normMachine 제거 — 백엔드 roster 가 machine 을
  //   이미 seoul/zalman 으로 정규화해서 주므로 프론트 정규화 계층 불필요.)

  // ── rc P2 현황 그리드 — peers + (peer 로 안 잡힌) 모든 tmux 세션 병합 ──────────────
  //   peers() = 정본 신원(canonical_address/name·quarantined 포함, Task1 백엔드).
  //   거기에 어느 peer 의 session_identifier 와도 안 묶인 tmux 세션을 standalone 행으로 합쳐
  //   "모든 tmux 리스트업 + 전체 행 액션"(스펙) 을 만족. sessions().sessions(SessionsDto) 가
  //   실 소스 — 이 머신 tmux. tmux root 만(window entry 제외, identifier ':' 2개 이하).
  // ── rc P2.5 통합 데이터그리드 — peer + tmux + acp 를 한 정렬 가능 표로 ───────────────
  //   세 소스를 GridRow 로 평탄화: peer(정본·편집/액션 가능) · tmux(standalone, 종료/재시작/새창)
  //   · acp(sessions().sessions kind="xgram_session" + 원격 ACP 신원, 읽기 전용).
  //   sessions() 는 단일 머신(SessionsDto.machine). ACP 세션의 머신 = 이 머신(localMachineName),
  //   원격 ACP 신원은 agents().machine 라벨로 표시. peer 행 머신 = peer.machine ?? 로컬.
  type GridKind = "peer" | "tmux" | "acp";
  interface GridRow {
    kind: GridKind;
    alias: string;            // 액션 호출 키(= peer 의 전체 alias = A2A 라우팅 키. 없으면 agent/세션 alias)
    name: string;             // 표시 이름
    canonical: string | null; // 정본 주소(peer 만)
    machine: string;          // 머신
    sid: string | null;       // 세션 id
    role: string | null;
    cwd: string | null;
    status: string | null;    // active / attached / null
    editable: boolean;        // 이름·역할 인라인 편집 가능(peer + acp — agent_profiles 신원 보유)
    quarantined: boolean;     // standalone dimming
    hasAgentRecord: boolean;  // has_agent(agent_profiles) → agents_delete 가능
    // ── 신원 통합 capability 플래그(roster 의 소스 플래그 → 종류 셀) ──
    isPeer: boolean;          // peer 레코드 보유 → peer_delete 가능
    hasTmux: boolean;         // 로컬 tmux 세션 존재
    // ── Phase A 통합 현황 그리드 — 등록상태 + 재무·평판 컬럼(백엔드 roster 제공). ──
    registered: boolean;      // 등록상태(peer 또는 agent_profiles 신원 보유)
    walletBalance: number | null; // 지갑잔액
    income: number | null;        // 수입
    expense: number | null;       // 지출
    rating: number | null;        // 별점(avg_rating)
    reviewCount: number | null;   // 평가 수
    views: number | null;         // 인지도(조회수 — Phase B agent_metrics 백킹)
    tokenPrice: number | null;    // 토큰단가(1M) — Phase B agent_profiles 백킹(인라인 편집)
    sampleText: string | null;    // 샘플 텍스트 — Phase B
    sampleUrl: string | null;     // 샘플 URL(파일/랜딩) — Phase B
  }
  // ── rc 현황 그리드 — 백엔드 roster 엔드포인트를 직접 GridRow 로 매핑(thin renderer) ──────
  //   이전엔 여기서 peers()+agents()+sessions() 를 normSid/byCanon/bySid/byAlias 로 프론트 병합했다.
  //   이제 백엔드 GET /v1/gui/roster 가 canonical identity 당 1행으로 병합·dedup·머신정규화까지 끝내고
  //   RosterEntryDto[] 를 준다 → 프론트 merge 계층 전체 제거. 매핑은 1:1 straight-across.
  //   머신은 백엔드가 이미 seoul/zalman 으로 정규화 → 셀에서 r.machine 그대로 표시(프론트 normMachine 미적용).
  const unifiedRows = createMemo<GridRow[]>(() => {
    const entries = (roster() ?? []) as RosterEntryDto[];
    return entries.map((e): GridRow => {
      // 종류(kind) — capability 플래그 조합. kindLabel/kindTitle 합성용 대표 kind.
      const kind: GridKind = e.is_peer ? "peer" : e.has_agent ? "acp" : "tmux";
      return {
        kind,
        alias: e.primary_alias,
        name: e.display_name ?? e.primary_alias,
        canonical: e.canonical_address ?? null,
        machine: e.machine ?? "",         // 백엔드 정규화 완료 — 그대로
        sid: e.session_identifier ?? null,
        role: e.role ?? null,
        cwd: e.cwd ?? null,
        // rc.360 — 백엔드 생명주기 status(active/stopped/dead)를 그대로 사용.
        //   폴백: status 미제공(구버전 데몬) 시 sid 유무로 active/stopped 추론.
        status: e.status ?? (e.session_identifier ? "active" : "stopped"),
        // 편집(이름·역할 인라인) — peer 또는 agent_profiles 신원 보유 행만(순수 tmux 제외).
        editable: e.is_peer || e.has_agent,
        quarantined: !!e.quarantined,
        hasAgentRecord: e.has_agent,      // agents_delete 가능
        isPeer: e.is_peer,                // peer_delete 가능
        hasTmux: e.has_tmux,              // 로컬 tmux 세션 존재
        // ── Phase A — 등록상태 + 재무·평판(백엔드 roster 가 이미 제공, 매핑만 추가) ──
        registered: e.is_peer || e.has_agent, // 등록상태(peer/agent 신원 보유)
        walletBalance: e.wallet_balance ?? null,
        income: e.income ?? null,
        expense: e.expense ?? null,
        rating: e.rating ?? null,
        reviewCount: e.review_count ?? null,
        views: e.views ?? null,
        // ── Phase B — 토큰단가/샘플(agent_profiles 백킹) ──
        tokenPrice: e.token_price ?? null,
        sampleText: e.sample_text ?? null,
        sampleUrl: e.sample_url ?? null,
      };
    });
  });

  // 종류 셀 라벨/툴팁 — 소스 조합(peer/agent/tmux) → 합성 라벨(peer · peer·acp · acp · tmux).
  const kindLabel = (r: GridRow): string => {
    const parts: string[] = [];
    if (r.isPeer) parts.push("peer");
    if (r.hasAgentRecord) parts.push("acp");
    if (r.hasTmux && !r.isPeer && !r.hasAgentRecord) parts.push("tmux");
    return parts.length ? parts.join("·") : r.kind;
  };
  const kindTitle = (r: GridRow): string => {
    const t: string[] = [];
    if (r.isPeer) t.push("P2P peer(정본 신원)");
    if (r.hasAgentRecord) t.push("ACP 에이전트(agent_profiles)");
    if (r.hasTmux) t.push("로컬 tmux 세션");
    return t.join(" + ") || r.kind;
  };
  // 정렬 상태 — 다중컬럼(multi-column). sortKeys = 우선순위 순서 배열.
  //   기본: 머신(로컬=seoul 먼저) → 폴더(cwd) asc. 서울 데몬이므로 서울 행이 먼저 묶이고 폴더 정렬.
  //   헤더 클릭 = 그 컬럼을 단독 primary 키로(이미 primary면 asc↔desc 토글).
  //   Shift+클릭 = 그 컬럼을 보조/3차 정렬키로 추가(이미 있으면 dir 토글). 다중 정렬 메커니즘.
  type SortCol = "status" | "name" | "alias" | "canonical" | "machine" | "kind" | "sid" | "role" | "cwd";
  type SortDir = "asc" | "desc";
  interface SortKey { col: SortCol; dir: SortDir; }
  const [sortKeys, setSortKeys] = createSignal<SortKey[]>([
    { col: "machine", dir: "asc" },
    { col: "cwd", dir: "asc" },
  ]);
  const KIND_ORDER: Record<GridKind, number> = { peer: 0, tmux: 1, acp: 2 };
  // 헤더 클릭 핸들러 — shift 여부로 분기.
  const onSort = (col: SortCol, ev?: MouseEvent) => {
    const shift = !!ev?.shiftKey;
    setSortKeys((prev) => {
      const idx = prev.findIndex((k) => k.col === col);
      if (shift) {
        // Shift+클릭 — 보조키 추가 또는 기존 키 dir 토글(우선순위는 유지).
        if (idx >= 0) {
          const next = [...prev];
          next[idx] = { col, dir: next[idx].dir === "asc" ? "desc" : "asc" };
          return next;
        }
        return [...prev, { col, dir: "asc" }];
      }
      // 일반 클릭 — 그 컬럼을 단독 primary 로. 이미 단독 primary면 dir 토글.
      if (prev.length === 1 && prev[0].col === col) {
        return [{ col, dir: prev[0].dir === "asc" ? "desc" : "asc" }];
      }
      return [{ col, dir: "asc" }];
    });
  };
  // 정렬 우선순위 인디케이터 — ▲/▼ + 작은 순번 위첨자(¹²³). 비활성 컬럼은 빈 문자열.
  const SUP = ["¹", "²", "³", "⁴", "⁵", "⁶", "⁷", "⁸", "⁹"];
  const sortInd = (col: SortCol) => {
    const keys = sortKeys();
    const idx = keys.findIndex((k) => k.col === col);
    if (idx < 0) return "";
    const arrow = keys[idx].dir === "asc" ? "▲" : "▼";
    const rank = keys.length > 1 ? (SUP[idx] ?? `${idx + 1}`) : "";
    return ` ${arrow}${rank}`;
  };
  // 머신 컬럼 비교 — 로컬 머신(이 데몬=seoul)을 rank 0 으로 먼저, 나머지는 알파벳순.
  //   3번째 머신이 생기거나 이름이 바뀌어도 로컬이 항상 앞서도록 local-first 구현.
  //   로컬 판정은 기존 isLocalMachine(selfMachineNames 기반 — "seoul"/"server-seoul"/hostname 매칭) 재사용.
  const machineRank = (m: string): [number, string] => {
    const mm = (m || "").toLowerCase();
    return [isLocalMachine(m) ? 0 : 1, mm];
  };
  const sortedRows = createMemo<GridRow[]>(() => {
    const keys = sortKeys();
    const cmpStr = (a: string, b: string) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });
    const cmpCol = (col: SortCol, x: GridRow, y: GridRow): number => {
      switch (col) {
        case "kind": return KIND_ORDER[x.kind] - KIND_ORDER[y.kind];
        case "status": { const rk = (s: string | null) => (s === "active" ? 0 : s === "stopped" ? 1 : 2); return rk(x.status) - rk(y.status); }
        case "canonical": return cmpStr(x.canonical ?? "", y.canonical ?? "");
        case "name": return cmpStr(x.name ?? "", y.name ?? "");
        case "alias": return cmpStr(x.alias ?? "", y.alias ?? "");
        case "machine": {
          const [rx, sx] = machineRank(x.machine);
          const [ry, sy] = machineRank(y.machine);
          return rx !== ry ? rx - ry : cmpStr(sx, sy); // 로컬 우선 → 나머지 알파벳
        }
        case "sid": return cmpStr(x.sid ?? "", y.sid ?? "");
        case "role": return cmpStr(x.role ?? "", y.role ?? "");
        case "cwd": return cmpStr(x.cwd ?? "", y.cwd ?? "");
      }
      return 0;
    };
    const rows = [...unifiedRows()];
    rows.sort((x, y) => {
      for (const k of keys) {
        const c = cmpCol(k.col, x, y) * (k.dir === "asc" ? 1 : -1);
        if (c !== 0) return c;
      }
      // 최종 안정 보조키 = alias(결정적 순서 보장).
      return cmpStr(x.alias ?? "", y.alias ?? "");
    });
    return rows;
  });

  // ── rc.362 통합 그리드 일괄 관리 — 행 체크박스 선택 + 일괄 삭제 / 일괄 종료 ──────────
  //   선택 키 = 행의 안정 id. 여기서는 r.alias 사용(roster 행마다 유일 = 액션 키 = 정렬 최종 보조키).
  //   roster refetch 후에도 alias 는 동일 신원에 안정적이므로 선택이 유지된다(사라진 행만 자연 탈락).
  //   Set<string> 을 signal 로 보관(불변 교체로 반응성). 헤더 체크박스 = 현재 필터/정렬된 sortedRows 전체.
  const [gridSel, setGridSel] = createSignal<Set<string>>(new Set());
  const rowId = (r: GridRow) => r.alias;
  const isSelected = (r: GridRow) => gridSel().has(rowId(r));
  const toggleRow = (r: GridRow) => {
    const id = rowId(r);
    setGridSel((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };
  // 현재 보이는(sortedRows) 행 전체 선택/해제. 모두 선택돼 있으면 해제, 아니면 모두 선택.
  const visibleIds = createMemo<string[]>(() => sortedRows().map(rowId));
  const allVisibleSelected = createMemo<boolean>(() => {
    const ids = visibleIds();
    return ids.length > 0 && ids.every((id) => gridSel().has(id));
  });
  const someVisibleSelected = createMemo<boolean>(() => visibleIds().some((id) => gridSel().has(id)));
  const toggleSelectAll = () => {
    setGridSel((prev) => {
      if (allVisibleSelected()) {
        // 보이는 것만 해제(필터 밖 선택은 보존)
        const next = new Set(prev);
        for (const id of visibleIds()) next.delete(id);
        return next;
      }
      const next = new Set(prev);
      for (const id of visibleIds()) next.add(id);
      return next;
    });
  };
  const clearSelection = () => setGridSel(new Set<string>());
  // 선택된 GridRow 배열(현재 unifiedRows 에 실재하는 것만 — 사라진 행 제외).
  const selectedRows = createMemo<GridRow[]>(() => {
    const sel = gridSel();
    return sortedRows().filter((r) => sel.has(rowId(r)));
  });
  const selectedCount = createMemo<number>(() => selectedRows().length);

  // 일괄 작업 진행 상태 — 진행 중이면 버튼 비활성 + 진행 텍스트.
  const [bulkBusy, setBulkBusy] = createSignal(false);
  const [bulkProgress, setBulkProgress] = createSignal<string>("");

  // 단일 행 삭제 — 종류별 라우팅(per-row 삭제와 동일 엔드포인트). 확인창 없음(일괄에서 한 번에 확인).
  //   is_peer → peer_delete · has_agent → agents_delete · 순수 tmux → skip(삭제 대상 아님).
  async function bulkDeleteOne(r: GridRow): Promise<void> {
    if (r.isPeer) {
      await invoke("peer_delete", { alias: r.alias });
    } else if (r.hasAgentRecord) {
      await invoke("agents_delete", { alias: r.alias });
    } else {
      throw new Error("삭제 가능한 레코드 없음(순수 tmux)");
    }
  }
  // 단일 행 종료 — per-row 종료와 동일 경로. aoe-acp: → ACP close · 그 외 → session_kill.
  async function bulkTerminateOne(r: GridRow): Promise<void> {
    const id = r.sid;
    if (!id) throw new Error("활성 세션 없음");
    if (id.startsWith("aoe-acp:")) {
      const sessionId = id.slice("aoe-acp:".length);
      await acpFetch("DELETE", `/sessions/${encodeURIComponent(sessionId)}`);
    } else {
      await invoke("session_kill", { identifier: id });
    }
  }
  // 일괄 삭제 — 선택된 행 각각을 종류별 엔드포인트로(per-row 와 동일). 순차 실행 + 결과 집계.
  async function bulkDelete() {
    const rows = selectedRows();
    if (rows.length === 0) return;
    // 순수 tmux(삭제 불가) 분리.
    const deletable = rows.filter((r) => r.isPeer || r.hasAgentRecord);
    const skipped = rows.length - deletable.length;
    const msg = `선택한 ${rows.length}개 중 ${deletable.length}개를 삭제합니다.`
      + (skipped > 0 ? `\n(순수 tmux ${skipped}개는 삭제 대상이 아니어서 건너뜁니다 — 종료를 사용하세요)` : "")
      + `\n\n(되돌릴 수 없습니다) 계속하시겠습니까?`;
    if (!window.confirm(msg)) return;
    setBulkBusy(true);
    let ok = 0; const fails: string[] = [];
    for (let i = 0; i < deletable.length; i++) {
      const r = deletable[i];
      setBulkProgress(`삭제 중… ${i + 1}/${deletable.length} (${r.name})`);
      try { await bulkDeleteOne(r); ok++; }
      catch (e) { fails.push(`${r.name} (${r.alias}): ${(e as Error).message}`); }
    }
    setBulkProgress("");
    setBulkBusy(false);
    await refetchRoster(); await refetchPeers(); await refetchAgents(); await refetchSessions();
    clearSelection();
    let summary = `일괄 삭제 완료 — ${ok}성공 ${fails.length}실패`;
    if (skipped > 0) summary += ` · ${skipped} 건너뜀(tmux)`;
    if (fails.length > 0) summary += `\n\n실패:\n- ${fails.join("\n- ")}`;
    window.alert(summary);
  }
  // 일괄 종료 — 선택된 행 중 활성 세션(sid) 보유분만. 세션 없는 행은 skip(건너뜀 수 보고).
  async function bulkTerminate() {
    const rows = selectedRows();
    if (rows.length === 0) return;
    const live = rows.filter((r) => !!r.sid);
    const skipped = rows.length - live.length;
    const msg = `선택한 ${rows.length}개 중 ${live.length}개 세션을 종료합니다.`
      + (skipped > 0 ? `\n(활성 세션 없는 ${skipped}개는 건너뜁니다)` : "")
      + `\n\n종료는 세션을 멈출 뿐 목록에서 삭제하지 않습니다(상태→stopped). 계속하시겠습니까?`;
    if (!window.confirm(msg)) return;
    setBulkBusy(true);
    let ok = 0; const fails: string[] = [];
    for (let i = 0; i < live.length; i++) {
      const r = live[i];
      setBulkProgress(`종료 중… ${i + 1}/${live.length} (${r.name})`);
      try { await bulkTerminateOne(r); ok++; }
      catch (e) { fails.push(`${r.name} (${r.alias}): ${(e as Error).message}`); }
    }
    setBulkProgress("");
    setBulkBusy(false);
    await refetchRoster(); await refetchAgents(); await refetchSessions();
    clearSelection();
    let summary = `일괄 종료 완료 — ${ok}성공 ${fails.length}실패`;
    if (skipped > 0) summary += ` · ${skipped} 건너뜀(세션 없음)`;
    if (fails.length > 0) summary += `\n\n실패:\n- ${fails.join("\n- ")}`;
    window.alert(summary);
  }

  // 인라인 편집 상태 — { alias, field }. 이름·역할 셀 클릭 시 input 으로 전환.
  //   커밋: peer_set_name / peer_set_role → refetchPeers. Enter/blur 커밋, Esc 취소, 빈/불변 = no-op.
  const [editing, setEditing] = createSignal<{ alias: string; field: "name" | "role" | "tokenPrice" } | null>(null);
  const isEditing = (alias: string, field: "name" | "role" | "tokenPrice") => {
    const e = editing();
    return !!e && e.alias === alias && e.field === field;
  };
  async function commitInlineEdit(row: GridRow, field: "name" | "role", value: string) {
    setEditing(null);
    const next = value.trim();
    // 비교 기준: peer 면 원본 필드, acp 등은 표시값(row.name/role). peer_set_name/role 은
    //   peers + agent_profiles 양쪽을 갱신하므로 acp 신원도 동일 호출로 편집된다.
    const cur = field === "name" ? row.name : (row.role ?? "");
    if (next === "" || next === cur) return; // 빈/불변 = no-op.
    setActing(row.alias);
    try {
      await invoke(field === "name" ? "peer_set_name" : "peer_set_role",
        field === "name" ? { alias: row.alias, name: next } : { alias: row.alias, role: next });
      await refetchRoster(); await refetchPeers(); await refetchAgents();
    } catch (e) {
      window.alert(`${field === "name" ? "이름" : "역할"} 변경 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }

  // Phase B — 토큰단가 인라인 편집 커밋. 빈 문자열=null(미설정), 숫자=값. 불변=no-op.
  //   백엔드 PATCH /v1/gui/agents/{alias}/token-price (body.token_price_per_million). registered 행만.
  async function commitTokenPrice(row: GridRow, value: string) {
    setEditing(null);
    const raw = value.trim();
    let next: number | null;
    if (raw === "") {
      next = null;
    } else {
      const n = Number(raw);
      if (!Number.isFinite(n) || n < 0) { window.alert("토큰단가는 0 이상 숫자만 가능합니다"); return; }
      next = n;
    }
    if (next === row.tokenPrice) return; // 불변 = no-op.
    setActing(row.alias);
    try {
      await invoke("agents_set_token_price", { alias: row.alias, token_price_per_million: next });
      await refetchRoster();
    } catch (e) {
      window.alert(`토큰단가 변경 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }

  // Phase B — 샘플 편집(텍스트/URL). prompt 로 간소 입력(전체 에디터는 Phase C).
  //   POST /v1/gui/agents/{alias}/sample (sample_text/sample_url). registered 행만.
  async function editSample(row: GridRow) {
    const text = window.prompt("샘플 텍스트(설명/스니펫) — 비우면 미설정:", row.sampleText ?? "");
    if (text === null) return; // 취소.
    const url = window.prompt("샘플 URL(파일 또는 랜딩페이지) — 비우면 미설정:", row.sampleUrl ?? "");
    if (url === null) return; // 취소.
    setActing(row.alias);
    try {
      await invoke("agents_set_sample", {
        alias: row.alias,
        sample_text: text.trim() === "" ? null : text.trim(),
        sample_url: url.trim() === "" ? null : url.trim(),
      });
      await refetchRoster();
    } catch (e) {
      window.alert(`샘플 변경 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }

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


  // 세션 cwd 들에서 home 루트(`/home/<user>`)를 추정 — project_path 의 tilde(`~`) 확장용.
  const sessionHome = createMemo<string>(() =>
    detectHome((sessions()?.sessions ?? []).map((s) => s.cwd)),
  );

  // 에이전트의 project_path 를 절대경로로 정규화(tilde 확장 + 끝슬래시 제거). 빈 값이면 "".
  const agentCwd = (pp: string | null | undefined): string => {
    const raw = (pp ?? "").trim();
    if (!raw) return "";
    return normPath(expandHome(raw, sessionHome()));
  };

  // 등록 에이전트들의 project_path 집합(tilde 확장 후) — descendant 세션의 longest-prefix 귀속 판정용.
  const registeredCwds = createMemo<Set<string>>(() => {
    const s = new Set<string>();
    for (const a of agents() ?? []) {
      const p = agentCwd(a.project_path);
      if (p) s.add(p);
    }
    return s;
  });

  // 선택 에이전트의 tmux 세션 — cwd 매칭(절대경로, prefix-aware, longest-prefix) 우선 +
  //   정규화 alias substring 매칭 보조. 매칭되는 모든 세션 반환(첫 것만 X).
  const selSessions = createMemo<DetectedSession[]>(() => {
    const aliasRaw = selected() ?? "";
    if (!aliasRaw) return [];
    const convoCwd = agentCwd(selAgent()?.project_path);
    const regs = registeredCwds();
    const all = sessions()?.sessions ?? [];
    return all.filter((s) => {
      if (s.kind !== "tmux") return false;
      // peer 가 미러링한 원격 뷰(`peer:<alias>:tmux:…`)는 로컬 세션과 중복 — 제외.
      if ((s.identifier ?? "").startsWith("peer:")) return false;
      const sCwd = s.cwd ? normPath(s.cwd.trim()) : "";
      // 1순위: cwd 매칭(가장 신뢰도 높음). 세션 cwd == 폴더 또는 폴더 하위.
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
      // 2순위(보조): 정규화 alias 가 세션 이름에 substring 포함("starianset" ∈ "aoe_starianset_…").
      //   단, 세션 cwd 가 *다른* 등록 에이전트의 폴더(==/하위)에 더 구체적으로 귀속되면
      //   alias substring 오매칭을 막는다(예: alias "Star"⊂"starianset" 인데 cwd 는 starian-set).
      if (sCwd) {
        for (const r of regs) {
          if (r === convoCwd) continue;
          if ((sCwd === r || sCwd.startsWith(r + "/")) && r.length > convoCwd.length) return false;
        }
      }
      return aliasMatchesSession(aliasRaw, s);
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
  const [paneScreen, { refetch: refetchPane }] = createResource(
    () => captureTarget()?.identifier ?? null,
    async (identifier) => {
      try {
        const r = await invoke("session_screen", { identifier });
        return r as { content?: string; lines?: number; source_note?: string } | null;
      } catch { return null; }
    },
  );

  // 🔧 Fix#123-2 — 작업환경 세션 행 새로고침: 해당 세션 화면(capture)을 다시 가져온다.
  //   세션 목록도 함께 refetch 하여 죽은 세션이 사라지게.
  async function refreshSession(_s: DetectedSession) {
    await Promise.all([refetchPane(), refetchSessions()]);
  }

  // 🔧 Fix#123-1 — 라이브 term 을 패널 폭에 가로스크롤 없이 맞춤(80컬럼 TUI · 박스드로잉 보존).
  //   inner(.term-fit) 의 실제 콘텐츠 폭이 컨테이너 폭을 넘으면 transform:scale 로 균일 축소.
  //   균일 스케일이라 박스라인/정렬이 깨지지 않는다(pre-wrap 미사용). 콘텐츠/리사이즈마다 재계산.
  let termBox: HTMLDivElement | undefined;
  let termFit: HTMLDivElement | undefined;
  const [termScale, setTermScale] = createSignal(1);
  function fitTerm() {
    if (!termBox || !termFit) return;
    // zoom 1 기준 자연 폭 측정(zoom 은 transform 과 달리 레이아웃 박스·scrollWidth 를 함께 줄여
    //   가로 오버플로 자체를 제거한다 — Chromium GUI 타깃). 먼저 1 로 두고 콘텐츠 자연 폭 읽기.
    termFit.style.zoom = "1";
    const natural = termFit.scrollWidth;
    // 가용 폭 = term 콘텐츠 박스(clientWidth) − 좌우 패딩 − 안전여유. clientWidth 는 패딩 포함이라 패딩을 뺀다.
    const cs = getComputedStyle(termBox);
    const padX = (parseFloat(cs.paddingLeft) || 0) + (parseFloat(cs.paddingRight) || 0);
    const avail = termBox.clientWidth - padX - 4;
    let s = 1;
    if (natural > avail && natural > 0) s = Math.max(0.45, avail / natural);
    setTermScale(s);
    termFit.style.zoom = String(s);
  }
  // 콘텐츠(캡처) 바뀔 때마다 + 곁뷰 열릴 때 재맞춤.
  createEffect(() => { paneScreen(); sideTmux(); requestAnimationFrame(() => requestAnimationFrame(fitTerm)); });
  onMount(() => {
    if (typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(() => fitTerm());
      onCleanup(() => ro.disconnect());
      // termBox 가 마운트되면 관찰 시작.
      createEffect(() => { if (termBox) ro.observe(termBox); });
    }
  });

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

  // ── rc.338 현황 탭 — 머신별 TMUX·ACP 세션 기스트 + tmux 종료(kill) ──────────────
  //   요청(마스터): "매 머신에 있는 TMUX·ACP 의 기스트 + TMUX 종료". 깔끔한 목록(표 X).
  //   재사용: sessions()(tmux, machine 포함) + agents()(등록 ACP 에이전트=대화 신원).
  //   머신 그룹핑은 로스터와 동일 로직(isLocalMachine / 머신 라벨). 현재 실세션은 이 머신만.
  //   (localMachineName 정의는 unifiedRows 위로 이동 — createMemo 즉시평가 TDZ 회피. 아래 참조.)
  // (통합 데이터그리드가 localTmuxSessions·acpByMachine 를 흡수 — unifiedRows/sortedRows 참조)
  // tmux 세션 종료 — confirm → POST kill → 성공 시 sessions refetch. (대화 곁뷰 tmux 패널에서 사용)
  const [killing, setKilling] = createSignal<string | null>(null);
  async function killTmux(s: DetectedSession) {
    const label = s.display || s.identifier;
    if (!window.confirm(`이 tmux 세션을 종료하시겠습니까?\n\n${label}\n\n(되돌릴 수 없습니다)`)) return;
    setKilling(s.identifier);
    try {
      await invoke("session_kill", { identifier: s.identifier });
      await refetchSessions();
    } catch (e) {
      window.alert(`tmux 종료 실패: ${(e as Error).message}`);
    } finally {
      setKilling(null);
    }
  }

  // ── list-peer 로스터 액션 — 종료 / 재시작(kill+재spawn) / spawn ──────────────
  //   재사용: session_kill(identifier) + 신규 session_restart / agent_spawn (GUI route).
  //   동작 후 agents()·sessions() refetch 로 현황 동적 갱신(룰: list-peer 와 동적 연동).
  const [acting, setActing] = createSignal<string | null>(null);
  async function killAgent(a: AgentRow) {
    const id = a.session_identifier;
    if (!id) { window.alert("세션 id 없음 — 종료할 활성 세션이 없습니다."); return; }
    if (!window.confirm(`이 에이전트 세션을 종료하시겠습니까?\n\n${agentName(a)} (${a.alias})\n\n(되돌릴 수 없습니다)`)) return;
    setActing(a.alias);
    try {
      // rc.360 — 종류별 종료 경로 분기:
      //   aoe-acp:<id> → ACP close (DELETE /v1/acp/sessions/{id}). tmux 가 아니므로 session_kill 불가.
      //   그 외(tmux:<name> / bare) → session_kill(로컬 tmux only, 데몬이 게이트).
      if (id.startsWith("aoe-acp:")) {
        const sessionId = id.slice("aoe-acp:".length);
        await acpFetch("DELETE", `/sessions/${encodeURIComponent(sessionId)}`);
      } else {
        await invoke("session_kill", { identifier: id });
      }
      await refetchRoster(); await refetchAgents(); await refetchSessions();
    } catch (e) {
      window.alert(`종료 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }
  // ── rc.360 Spawn — 중지/사망 행을 다시 띄운다 ──────────────────────────────
  //   ACP 신원(agent_profiles 보유) → agent_spawn(POST /agents/{alias}/spawn, find-or-create).
  //   peer-only(신원 레코드 없음) → 동일 라우트 시도(데몬이 alias→adapter/cwd 해석). 실패 시 메시지.
  //   openxgram 이 peer_send/ask_agent 호출 시 ACP find-or-create 로도 자동 재spawn 되지만,
  //   콘솔에서 수동 트리거 제공(스펙: ▶️ Spawn).
  async function spawnAgent(r: GridRow) {
    if (!window.confirm(`이 에이전트의 ACP 세션을 새로 띄우시겠습니까?\n\n${r.name} (${r.alias})`)) return;
    setActing(r.alias);
    try {
      await invoke("agent_spawn", { alias: r.alias });
      await refetchRoster(); await refetchAgents(); await refetchSessions();
    } catch (e) {
      window.alert(`spawn 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }
  // ── rc 현황 그리드 — 미등록 tmux 행을 에이전트로 등록(스펙: "현황그리드에서 모든 TMUX가 보이면 거기서 등록") ──
  //   재사용: agents_register(POST /v1/gui/agents) — AddAgentModal 과 동일 invoke. agent_capabilities + agent_profiles 기록.
  //   대상: 순수 tmux 행(hasTmux && !hasAgentRecord && !isPeer). r.machine/r.cwd 가 있으면 전달.
  async function registerTmux(r: GridRow) {
    if (!window.confirm(`이 TMUX 세션을 에이전트로 등록하시겠습니까?\n\n${r.name} (${r.alias})`)) return;
    setActing(r.alias);
    try {
      await invoke("agents_register", {
        alias: r.alias,
        role: "agent",
        ai_type: "claude",
        classification: "project",
        machine: r.machine || null,
        project_path: r.cwd || null,
        messenger_enabled: true,
      });
      await refetchRoster(); await refetchAgents(); await refetchSessions();
    } catch (e) {
      window.alert(`등록 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }
  // ── Phase A 금액추가(topup) — 마스터 → 서브 지갑 즉시 충전. ──
  //   재사용: MarketTab 과 동일한 wallet_topup(agent_id, amount_micro) invoke(POST /v1/gui/wallets/topup).
  //   roster 의 alias(primary_alias) = sub_wallets.agent_id. prompt 로 USD 입력 → micro 환산.
  async function topupRow(r: GridRow) {
    const raw = window.prompt(`충전할 금액(USD)을 입력하세요\n\n${r.name} (${r.alias})`, "1");
    if (raw == null) return;
    const usd = Number(raw);
    if (!Number.isFinite(usd) || usd <= 0) { window.alert("0 보다 큰 숫자를 입력하세요."); return; }
    setActing(r.alias);
    try {
      await invoke("wallet_topup", { agent_id: r.alias, amount_micro: Math.floor(usd * 1_000_000) });
      await refetchRoster();
    } catch (e) {
      window.alert(`금액추가 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }
  async function restartAgent(a: AgentRow) {
    const id = a.session_identifier;
    if (!id) { window.alert("세션 id 없음 — 재시작할 활성 세션이 없습니다. spawn 으로 새로 띄우세요."); return; }
    if (!window.confirm(`이 에이전트 세션을 재시작(종료 후 재생성)하시겠습니까?\n\n${agentName(a)} (${a.alias})`)) return;
    setActing(a.alias);
    try {
      await invoke("session_restart", { identifier: id, alias: a.alias });
      await refetchRoster(); await refetchAgents(); await refetchSessions();
    } catch (e) {
      window.alert(`재시작 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }
  // ── rc P2 현황 그리드 행 액션 — 삭제(목록에서 제거) / 이름(대화명) 편집 / 새 창 ──────────
  //   재사용: peer_delete(DELETE /peers/{alias}) · peer_set_name(PATCH /peers/{alias}/name).
  //   동작 후 refetchPeers 로 현황 동적 갱신(정본 신원 전파).
  //   roster 엔드포인트는 full PeerDto 를 주지 않으므로 alias 기반으로 동작(peer_delete 는 alias 만 필요).
  async function deletePeer(alias: string, label?: string) {
    if (!window.confirm(`이 항목을 목록에서 삭제하시겠습니까?\n\n${label ?? alias}\n\n(되돌릴 수 없습니다)`)) return;
    setActing(alias);
    try {
      await invoke("peer_delete", { alias });
      await refetchRoster(); await refetchPeers();
    } catch (e) {
      window.alert(`삭제 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }
  //   acp / 에이전트 레코드(agent_profiles) 삭제 — agents_delete(POST /agents/{alias}) → gui_agents_delete.
  //   peer 레코드 삭제(deletePeer)와 별개. 동작 후 agents·peers refetch.
  async function deleteAgent(alias: string) {
    if (!window.confirm(`이 에이전트(신원)를 명부에서 삭제하시겠습니까?\n\n${alias}\n\n(agent_profiles 레코드 제거 · 되돌릴 수 없습니다)`)) return;
    setActing(alias);
    try {
      await invoke("agents_delete", { alias });
      await refetchRoster(); await refetchAgents(); await refetchPeers();
    } catch (e) {
      window.alert(`에이전트 삭제 실패: ${(e as Error).message}`);
    } finally { setActing(null); }
  }
  // (editPeerName·editPeerRole 의 prompt 방식 → 통합 그리드 셀 인라인 편집 commitInlineEdit 로 대체)
  // 그리드 행 "새창" — 행마다 그 행 신원에 스코프된 팝업을 연다.
  //   App.tsx 가 실제 소비하는 파라미터만 사용(?peer= 는 무시됨 → 같은 풀 GUI 가 뜨던 버그).
  //   세션(sid) 있으면 tmux 라이브 팝업(?tmux=identifier&label), 없으면 대화 팝업(?chat=alias).
  //   ⚠️ sid(roster session_identifier)·DetectedSession identifier 둘 다 `tmux:aoe_X` 형식이고
  //      session_screen / WS 엔드포인트가 이 prefix 를 요구한다(raw `aoe_X` → "unsupported identifier").
  //      그래서 sid 를 그대로 ?tmux= 에 넘긴다(openTmuxPopout 과 동일 — strip 하면 안 됨).
  //   창 이름을 행별 고유(sid 우선, 없으면 alias)로 만들어 서로 다른 행이 서로 다른 창을 갖게 한다.
  //   → window.open(url, name) 으로 URL 을 직접 연다(빈 창 후 location.href 대입 X — 일부 브라우저서
  //      about:blank 에 머무는 경합 회피). 같은 행 재클릭 = 명명 타깃 재사용(중복 창 없음).
  //   팝업 차단 시 현재 탭을 가로채지 않는다(location.href X) — 사용자에게 허용 요청만.
  function openPeerWindow(r: { alias: string; sid?: string | null; name?: string | null }) {
    const display = r.name || r.alias;
    const url = r.sid
      ? `${location.origin}${location.pathname}?tmux=${encodeURIComponent(r.sid)}&label=${encodeURIComponent(display)}`
      : `${location.origin}${location.pathname}?chat=${encodeURIComponent(r.alias)}`;
    // 행 고유 창 이름 — 세션 행은 sid 기준(같은 sid 재클릭 시 그 창 재사용), 그 외 alias 기준.
    const winName = `oxg_${r.sid ?? r.alias}`;
    const w = window.open(url, winName, "width=820,height=620");
    if (!w) {
      window.alert("팝업이 차단되었습니다 — 이 사이트의 팝업을 허용해 주세요.");
      return;
    }
    try { window.focus(); } catch { /* noop */ }
  }

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
              <div class="side tmux-side-panel" classList={{ show: sideTmux() }}>
                {/* 🔧 Fix#123-3 — 닫기 ✕ 가 대화 헤더(.chat-top z:30) 아이콘 클러스터에 가려지던 문제.
                    협업 곁뷰와 동일 fix(rc.334): 패널을 z:35로 올리고, 닫기 버튼을 헤더 좌측(order:-1)에
                    z:31로 배치해 항상 클릭을 받게 한다. */}
                <h3>
                  <span class="tmux-side-close" title="작업환경 닫기" onClick={() => setSideTmux(false)}>‹ 닫기</span>
                  <span class="tmux-side-title">🖥 {agentName(a())} 작업환경 (tmux · 사람 전용)</span>
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
                      <div class="term" ref={(el) => (termBox = el)}>
                        {/* 🔧 Fix#123-1 — 80컬럼 TUI(박스드로잉)를 패널 폭에 가로스크롤 없이 맞춘다.
                            inner .term-fit 을 transform:scale 로 균일 축소(정렬/박스라인 보존) — fitTermScale effect. */}
                        <div class="term-fit" ref={(el) => (termFit = el)} style={{ zoom: termScale() }}>
                          <Show
                            when={paneScreen()?.content}
                            fallback={
                              <span class="c">{paneScreen.loading ? "# 화면 불러오는 중…" : "# 캡처할 화면이 없습니다 (세션이 비어있거나 접근 불가)."}</span>
                            }
                          >
                            {/* ANSI SGR 색 코드를 실제 색 span 으로 렌더 — raw escape garbage 제거 (FIX 2) */}
                            <For each={parseAnsi(paneScreen()!.content!)}>
                              {(seg) => <span class={segClass(seg.style)} style={segCss(seg.style)}>{seg.text}</span>}
                            </For>
                          </Show>
                        </div>
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
                            {/* 🔧 Fix#123-2 — 새로고침(화면 재캡처) + 종료(kill). detach 는 ATTACHED 클라이언트 전용 →
                                여기 나열되는 detached 세션엔 N/A 라 생략. kill 은 현황 탭과 동일 라우트(session_kill) 재사용. */}
                            <button
                              class="kk-we-act refresh"
                              title="이 세션 라이브 화면 새로고침 (detach 는 attached 클라이언트 전용이라 미제공)"
                              onClick={(e) => { e.stopPropagation(); void refreshSession(s); }}
                            >🔄</button>
                            <button
                              class="kk-we-act kill"
                              title="이 tmux 세션 종료"
                              disabled={killing() === s.identifier}
                              onClick={(e) => { e.stopPropagation(); void killTmux(s); }}
                            >🗑</button>
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

        {/* ── rc P2.5 통합 현황 데이터그리드 — peer + tmux + acp 한 정렬·인라인편집 표 ──
            컬럼 헤더 클릭 = 정렬(asc↔desc, ▲/▼). 이름·역할 셀 클릭 = 인라인 편집(peer 만).
            모든 행 = 동일 4버튼(새창·종료·재시작·삭제), 능력별 활성/비활성으로 슬롯 정렬. */}
        <div style="display:flex;align-items:center;gap:10px;padding:4px 24px 0">
          <h2 style="padding:0">🧩 통합 현황 그리드</h2>
          <button class="killbtn" style="margin:0;color:#37424d" title="로스터·세션 다시 불러오기" disabled={roster.loading || sessions.loading} onClick={() => { void refetchRoster(); void refetchSessions(); }}>{(roster.loading || sessions.loading) ? "⏳ 새로고침 중…" : "🔄 새로고침"}</button>
        </div>
        <div class="sub">모든 peer · tmux · ACP 세션을 한 표로 — 헤더 클릭=정렬(다시 클릭 ▲↔▼), <b>Shift+클릭=정렬 단계 추가</b>(머신 ▲¹ 폴더 ▲² …) · 이름·역할 셀 클릭=인라인 편집(peer·ACP) · 모든 행 동일 4액션(새창·종료·재시작·삭제), 능력별 활성/비활성 · <b>왼쪽 체크박스로 다중 선택 → 일괄 삭제·종료</b></div>
        {/* ── rc.362 일괄 작업 바 — ≥1행 선택 시 노출. 선택 수 + 일괄 삭제 / 일괄 종료. ── */}
        <Show when={selectedCount() > 0}>
          <div class="dg-bulkbar">
            <span class="dg-bulk-count">✔ {selectedCount()}개 선택됨</span>
            <span class="dg-bulk-prog">{bulkProgress()}</span>
            <div class="dg-bulk-acts">
              <button class="dg-bulk-btn del" disabled={bulkBusy()} title="선택한 행 일괄 삭제 — peer/agent 레코드 제거(되돌릴 수 없음). 순수 tmux 는 건너뜀." onClick={() => void bulkDelete()}>🗑 일괄 삭제</button>
              <button class="dg-bulk-btn term" disabled={bulkBusy()} title="선택한 행의 활성 세션 일괄 종료 — 세션만 멈춤(목록 유지). 세션 없는 행은 건너뜀." onClick={() => void bulkTerminate()}>⏹ 일괄 종료</button>
              <button class="dg-bulk-btn" disabled={bulkBusy()} title="선택 해제" onClick={clearSelection}>✕ 선택 해제</button>
            </div>
          </div>
        </Show>
        <div class="dgrid">
          {/* 헤더 — 클릭 정렬 */}
          <div class="dg-row dg-head">
            <span class="dg-chk" title={allVisibleSelected() ? "보이는 행 전체 선택 해제" : "보이는 행 전체 선택"} onClick={(e) => e.stopPropagation()}>
              <input type="checkbox" checked={allVisibleSelected()}
                ref={(el) => { el.indeterminate = !allVisibleSelected() && someVisibleSelected(); }}
                onChange={toggleSelectAll} />
            </span>
            {/* ── Phase A 통합 현황 그리드 컬럼 순서 ──
                순번/상태/이름/역할/세션id/PATH/머신/종류/등록상태/토큰단가(1M)/지갑잔액/수입/지출/정본주소/샘플/별점/평가/인지도/액션.
                토큰단가(인라인 편집)·샘플(링크/보기+✎)·인지도(views) = Phase B 백킹 완료. 신규 컬럼 정렬 미적용. */}
            <span title="순번">#</span>
            <span onClick={(e) => onSort("status", e)} title="상태순 정렬 (Shift+클릭=정렬 단계 추가)">상태{sortInd("status")}</span>
            <span onClick={(e) => onSort("name", e)} title="이름순 정렬 (Shift+클릭=정렬 단계 추가)">이름{sortInd("name")}</span>
            <span onClick={(e) => onSort("role", e)} title="역할순 정렬 (Shift+클릭=정렬 단계 추가)">역할{sortInd("role")}</span>
            <span onClick={(e) => onSort("sid", e)} title="세션id순 정렬 (Shift+클릭=정렬 단계 추가)">세션id{sortInd("sid")}</span>
            <span onClick={(e) => onSort("cwd", e)} title="PATH(폴더)순 정렬 (Shift+클릭=정렬 단계 추가)">PATH{sortInd("cwd")}</span>
            <span onClick={(e) => onSort("machine", e)} title="머신순 정렬 — 로컬(seoul) 먼저 (Shift+클릭=정렬 단계 추가)">머신{sortInd("machine")}</span>
            <span onClick={(e) => onSort("kind", e)} title="종류순 정렬 (Shift+클릭=정렬 단계 추가)">종류{sortInd("kind")}</span>
            <span title="등록상태 — peer/agent 신원 보유 여부">등록상태</span>
            <span title="토큰단가(1M) — 외부 사용 시 1M 토큰당 단가(클릭 편집)">토큰단가(1M)</span>
            <span title="지갑잔액">지갑잔액</span>
            <span title="수입">수입</span>
            <span title="지출">지출</span>
            <span onClick={(e) => onSort("canonical", e)} title="정본주소순 정렬 (Shift+클릭=정렬 단계 추가)">정본주소{sortInd("canonical")}</span>
            <span title="샘플 — 파일/랜딩 URL 링크 또는 텍스트 보기(✎ 편집)">샘플</span>
            <span title="별점(평균 평점)">별점</span>
            <span title="평가 수">평가</span>
            <span title="인지도(조회 수) — agent_metrics.views">인지도</span>
            <span style="justify-content:flex-end">액션</span>
          </div>
          <Show when={sortedRows().length > 0} fallback={<div class="dg-row"><span /><span /><span class="dg-ro" style="grid-column:4/-1">표시할 peer · tmux · ACP 가 없습니다</span></div>}>
            <For each={sortedRows()}>
              {(r, i) => (
                <div class="dg-row" classList={{ "dg-row-sel": isSelected(r) }} style={r.quarantined ? "opacity:.78" : ""}>
                  {/* rc.362 행 선택 체크박스 — alias 키. 일괄 삭제·종료 대상. */}
                  <span class="dg-chk" title="이 행 선택">
                    <input type="checkbox" checked={isSelected(r)} onChange={() => toggleRow(r)} />
                  </span>
                  {/* 순번 — 현재 정렬 순서 기준 1..N */}
                  <span style="color:var(--muted)">{i() + 1}</span>
                  {/* 상태 점 — rc.360 3색: active=초록 · stopped=회색 · dead=빨강 */}
                  <span title={r.status === "active" ? "active — 라이브 세션" : r.status === "dead" ? "dead — 세션 사라짐(spawn 으로 재기동)" : "stopped — 세션 없음(spawn 으로 기동)"}>
                    <span class="dot" style={`display:inline-block;width:8px;height:8px;border-radius:50%;background:${r.status === "active" ? "var(--green)" : r.status === "dead" ? "#e5484d" : "var(--muted)"}`} />
                    <span style="margin-left:5px;font-size:10px;color:var(--muted)">{r.status === "active" ? "active" : r.status === "dead" ? "dead" : "stopped"}</span>
                  </span>
                  {/* 이름 — peer 면 셀 클릭 인라인 편집 */}
                  <Show when={isEditing(r.alias, "name")} fallback={
                    <span class={r.editable ? "dg-edit" : ""} title={r.editable ? "클릭하여 이름 편집" : r.name}
                      onClick={() => { if (r.editable) setEditing({ alias: r.alias, field: "name" }); }}>{r.editable ? <span class="dg-edit-tx">{r.name}</span> : r.name}</span>
                  }>
                    <span><input class="dg-in" autofocus value={r.name}
                      onKeyDown={(e) => { if (e.key === "Enter") void commitInlineEdit(r, "name", e.currentTarget.value); else if (e.key === "Escape") setEditing(null); }}
                      onBlur={(e) => void commitInlineEdit(r, "name", e.currentTarget.value)} /></span>
                  </Show>
                  {/* 역할 — peer 면 셀 클릭 인라인 편집 */}
                  <Show when={isEditing(r.alias, "role")} fallback={
                    <span class={r.editable ? "dg-edit" : ""} title={r.editable ? "클릭하여 역할 편집" : (r.role ?? "")}
                      onClick={() => { if (r.editable) setEditing({ alias: r.alias, field: "role" }); }}>{r.editable ? <span class="dg-edit-tx">{r.role ?? "—"}</span> : (r.role ?? "—")}</span>
                  }>
                    <span><input class="dg-in" autofocus value={r.role ?? ""}
                      onKeyDown={(e) => { if (e.key === "Enter") void commitInlineEdit(r, "role", e.currentTarget.value); else if (e.key === "Escape") setEditing(null); }}
                      onBlur={(e) => void commitInlineEdit(r, "role", e.currentTarget.value)} /></span>
                  </Show>
                  {/* 세션 id */}
                  <span title={r.sid ?? ""}>{r.sid ?? "—"}</span>
                  {/* PATH(폴더) */}
                  <span title={r.cwd ?? ""}>{r.cwd ?? "—"}</span>
                  {/* 머신 — 백엔드 roster 가 이미 seoul/zalman 으로 정규화 → 그대로 표시 */}
                  <span title={r.machine}>{r.machine || "—"}</span>
                  {/* 종류 배지 — 소스 조합(peer/agent/tmux 플래그) 기준 라벨 */}
                  <span class="dg-kind" style={r.isPeer ? "background:#e6f0fb;color:#2c5a8f" : r.hasAgentRecord ? "background:#eef1f4;color:#6a727c" : "background:#f0ece6;color:#8f6a2c"} title={kindTitle(r)}>{kindLabel(r)}</span>
                  {/* 등록상태 배지 — 등록=초록 · 미등록=회색 */}
                  <span class="dg-kind" style={r.registered ? "background:#e6f5ea;color:#2e7d32" : "background:#eef1f4;color:#6a727c"} title={r.registered ? "등록 — peer/agent 신원 보유" : "미등록 — 순수 tmux"}>{r.registered ? "등록" : "미등록"}</span>
                  {/* 토큰단가(1M) — Phase B: registered 행만 인라인 편집(빈값=미설정 null) */}
                  <Show when={isEditing(r.alias, "tokenPrice")} fallback={
                    <span class={r.registered ? "dg-edit" : ""} style={r.registered ? "" : "color:var(--muted)"}
                      title={r.registered ? "클릭하여 토큰단가(1M) 편집 — 외부 사용 시 1M 토큰당 단가" : "미등록 — 토큰단가 설정 불가"}
                      onClick={() => { if (r.registered) setEditing({ alias: r.alias, field: "tokenPrice" }); }}>
                      {r.registered ? <span class="dg-edit-tx">{fmtNum(r.tokenPrice)}</span> : "—"}</span>
                  }>
                    <span><input class="dg-in" type="number" min="0" step="any" autofocus value={r.tokenPrice ?? ""}
                      onKeyDown={(e) => { if (e.key === "Enter") void commitTokenPrice(r, e.currentTarget.value); else if (e.key === "Escape") setEditing(null); }}
                      onBlur={(e) => void commitTokenPrice(r, e.currentTarget.value)} /></span>
                  </Show>
                  {/* 지갑잔액 */}
                  <span title="지갑잔액">{fmtNum(r.walletBalance)}</span>
                  {/* 수입 */}
                  <span title="수입">{fmtNum(r.income)}</span>
                  {/* 지출 */}
                  <span title="지출">{fmtNum(r.expense)}</span>
                  {/* 정본 주소 배지(앞6…뒤4) — peer 만 */}
                  <span style={`font-size:10.5px;font-family:ui-monospace,Menlo,monospace;color:${r.canonical ? "#5a7fb0" : "var(--muted)"};background:${r.canonical ? "#eef4fa" : "transparent"};border-radius:6px;padding:1px 6px;justify-self:start`} title={r.canonical ?? "정본 주소 없음"}>{r.canonical ? `${r.canonical.slice(0, 6)}…${r.canonical.slice(-4)}` : "—"}</span>
                  {/* 샘플 — Phase B: URL 있으면 링크, 텍스트만 있으면 "보기"(tooltip), 둘 다 없으면 "—". registered 행은 ✎ 편집(prompt) */}
                  <span title={r.sampleUrl ?? r.sampleText ?? "샘플 미설정"}>
                    <Show when={r.sampleUrl} fallback={
                      <Show when={r.sampleText} fallback={<span style="color:var(--muted)">—</span>}>
                        <span style="cursor:help;color:#5a7fb0" title={r.sampleText ?? ""}>보기</span>
                      </Show>
                    }>
                      <a href={r.sampleUrl ?? "#"} target="_blank" rel="noreferrer" style="color:#2c5a8f" onClick={(e) => e.stopPropagation()}>샘플</a>
                    </Show>
                    <Show when={r.registered}>
                      <span class="dg-edit" style="margin-left:6px;cursor:pointer;color:var(--muted)" title="샘플 편집(텍스트/URL)" onClick={() => void editSample(r)}>✎</span>
                    </Show>
                  </span>
                  {/* 별점 — avg_rating */}
                  <span title="별점(평균 평점)">{r.rating == null ? "—" : `★ ${fmtNum(r.rating)}`}</span>
                  {/* 평가 수 */}
                  <span title="평가 수">{r.reviewCount == null ? "—" : `(${r.reviewCount})`}</span>
                  {/* 인지도(조회 수) — Phase B: agent_metrics.views(없으면 —) */}
                  <span title="인지도(조회 수)">{fmtNum(r.views)}</span>
                  {/* 액션 — 모든 행 동일 버튼(능력별 활성/비활성). 슬롯 고정 → 컬럼 세로 정렬. */}
                  <span class="dg-acts">
                    {/* 새창 — 항상 가능(순수 프론트) */}
                    <button class="killbtn" style="color:#37424d" title={r.sid ? "새 창 — tmux 라이브" : "새 창 — 대화"} disabled={acting() === r.alias} onClick={() => openPeerWindow({ alias: r.alias, sid: r.sid, name: r.name })}>🗗 새창</button>
                    {/* 종료 — 활성 세션(sid) 있을 때만 */}
                    <button class="killbtn" style="color:#e5484d" title={r.sid ? "세션 종료" : "활성 세션 없음"} disabled={acting() === r.alias || !r.sid} onClick={() => killAgent({ alias: r.alias, session_identifier: r.sid } as any)}>🗑 종료</button>
                    {/* 재시작 — 활성 세션(sid) 있을 때만(kill+재spawn / ACP 재생성) */}
                    <button class="killbtn" style="color:#37424d" title={r.sid ? "세션 재시작(kill+재spawn)" : "활성 세션 없음"} disabled={acting() === r.alias || !r.sid} onClick={() => restartAgent({ alias: r.alias, session_identifier: r.sid } as any)}>🔄 재시작</button>
                    {/* Spawn — rc.360. 중지/사망(active 아님) + ACP 신원(또는 peer) 일 때 활성. ACP find-or-create 로 재기동 */}
                    <button class="killbtn" style="color:#2c5a8f" title={r.status === "active" ? "이미 active — 새 세션 불필요" : "ACP 세션 새로 띄우기(spawn)"} disabled={acting() === r.alias || r.status === "active" || !(r.hasAgentRecord || r.isPeer)} onClick={() => void spawnAgent(r)}>▶️ spawn</button>
                    {/* 금액추가(topup) — 에이전트 행(agent/peer)만. wallet_topup 재사용(마스터 → 서브 충전) */}
                    <button class="killbtn" style="color:#2e7d32" title={(r.hasAgentRecord || r.isPeer) ? "이 에이전트 지갑에 금액 추가(충전)" : "에이전트 행만 충전 가능"} disabled={acting() === r.alias || !(r.hasAgentRecord || r.isPeer)} onClick={() => void topupRow(r)}>💰＋ 금액추가</button>
                    {/* 금액이전(에이전트→메인) — 백엔드 이전 엔드포인트 없음 → Phase C 비활성 플레이스홀더 */}
                    <button class="killbtn" style="color:#6a727c" title="금액이전(서브→메인) — Phase C" disabled>↪ 금액이전</button>
                    {/* 등록 — 미등록 순수 tmux 행만(스펙: 현황그리드에서 TMUX 직접 등록). agents_register 로 agent_profiles 신원 부여 */}
                    <Show when={r.hasTmux && !r.hasAgentRecord && !r.isPeer}>
                      <button class="killbtn" style="color:#2e7d32" title="이 TMUX 세션을 에이전트로 등록" disabled={acting() === r.alias} onClick={() => void registerTmux(r)}>📝 등록</button>
                    </Show>
                    {/* 삭제 — is_peer 면 peer_delete, has_agent 면 agents_delete, 순수 tmux 면 비활성 */}
                    <Show when={r.isPeer} fallback={
                      <button class="killbtn" style="color:#e5484d"
                        title={r.hasAgentRecord ? "에이전트(신원) 삭제" : "순수 tmux 세션은 종료로 제거"}
                        disabled={acting() === r.alias || !r.hasAgentRecord}
                        onClick={() => { if (r.hasAgentRecord) void deleteAgent(r.alias); }}>🗑 삭제</button>
                    }>
                      <button class="killbtn" style="color:#e5484d" title="목록에서 삭제(peer)" disabled={acting() === r.alias} onClick={() => void deletePeer(r.alias, r.name)}>🗑 삭제</button>
                    </Show>
                  </span>
                </div>
              )}
            </For>
          </Show>
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
          // 🆕 "새 에이전트 (이 머신)" 선택 → 친구 모달 닫고 기존 AddAgentModal(폴더/모델/설정 생성 흐름) 오픈.
          onPickNewLocal={() => { setFriendOpen(false); setAddOpen(true); }}
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
