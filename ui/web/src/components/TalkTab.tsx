import { createSignal, createResource, createMemo, createEffect, For, Show } from "solid-js";
import { invoke } from "../api/client";
import { AcpConversation, aiTypeToAdapter, type AcpPreset } from "./AcpConversation";
import { AddAgentModal } from "./AddAgentModal";
import { AddFriendModal, loadExternalFriends, type ExternalFriend } from "./AddFriendModal";
import { ProviderLogo, providerKey } from "./ProviderLogo";
import {
  computeUnregisteredSessions,
  isTooBroadPath,
  normPath,
  type DetectedSession,
  type SessionsDto,
} from "./agentSessions";

// 대화 탭 — 카카오톡 정본 목업(_mockups/kakao-mockup.html) 충실 이식.
// 좌: 분류 그룹화 명부(👑 프라이머리 / 📌 상단 고정 / 📁 프로젝트 / ⚙️ 특수) + llm-type 아바타색
//     + 마지막 메시지 미리보기/시각(messages_recent 파생).
// 우: 선택 에이전트의 ACP 세션(<AcpConversation preset=…>) — 대화는 전적으로 ACP가 구동.
//     레거시 peer_send(tmux-inject) 대화 경로는 제거됨(ACP 단일화, Phase 4-4).
// 데이터: agents_list(분류·ai_type·그룹) · peers_list(online) · messages_recent(미리보기)
//         · sessions(tmux/worktree) · workflows_list(워크플로우). 동적 only — 가짜 데이터 없음.

// agents_list row (AgentsTab.tsx 와 동일 contract — 재정의해 둠).
interface AgentRow {
  alias: string;
  role?: string | null;
  description?: string | null;
  group_name?: string | null;
  project_path?: string | null;
  messenger_enabled?: boolean;
  classification?: string | null;
  execution_mode?: string | null;
  ai_type?: string | null;
  is_public?: boolean | null;
  machine?: string | null;
  display_name?: string | null;
  perm_mode?: string | null;
  model?: string | null;
  thinking?: string | null;
  unread?: number | null;
}

// 표시 이름 — display_name 있으면 그것, 없으면 alias.
const agentName = (a: { display_name?: string | null; alias: string }) =>
  (a.display_name && a.display_name.trim()) || a.alias;

interface PeerDto {
  alias: string;
  last_seen?: string;
  machine?: string;
}

// tailnet 라우트(GET /v1/gui/tailnet/devices, client.ts tailnet_devices) → { devices: [...] }.
// AddFriendModal 과 동일 contract + rc 확장 guiUrl(각 장치의 OpenXgram GUI base URL, 포트 설치별).
// guiUrl 은 백엔드가 곧 추가(다른 에이전트 작업 중) — 없을 수 있으니 optional + 폴백 처리.
interface TailnetDeviceDto {
  name: string;
  ip: string;
  online?: boolean;
  self?: boolean;
  guiUrl?: string | null;
}

// DetectedSession / SessionsDto 는 ./agentSessions 에서 import(AgentsTab 와 공유 단일 출처).

// workflows_list 라우트(FlowTab 과 동일 contract). orchestrator 로 이 에이전트 참여 여부 판정.
interface WorkflowDto {
  id: string;
  name: string;
  description?: string | null;
  enabled?: boolean | null;
  orchestrator?: string | null;
  cron_expr?: string | null;
}

interface MessageDto {
  id: string;
  session_id: string;
  sender: string;
  body: string;
  timestamp: string;
  conversation_id: string;
}

const AI_COLOR: Record<string, string> = {
  claude: "c-claude", codex: "c-codex", gemini: "c-gemini", ollama: "c-ollama", hermes: "c-hermes",
};

function avatarColor(ai?: string | null): string {
  return (ai && AI_COLOR[ai.toLowerCase()]) || "c-group";
}

// 분류 → 정본 목업 그룹 헤더 라벨.
const CLASS_GROUPS = [
  { key: "primary", title: "👑 통합관리자 · 프라이머리" },
  { key: "pinned", title: "📁 상단 고정" },
  { key: "project", title: "📁 프로젝트 에이전트" },
  { key: "special", title: "⚙️ 특수·시스템 (깨움 선택)" },
];

// ── 로컬/친구 분류 (마스터 확정 개념) ──────────────────────────────────────
// 이 머신(server-seoul) = 로컬 에이전트만(파일트리·ACP 완전 작동).
// 다른 머신/외부 = "친구" — A2A/peer 로 통신, 그쪽 primary 가 자기 에이전트 처리.
//
// 로컬 판정: project_path 가 `/home/llm/` 하위(이 머신 HOME) OR machine 이 비었거나 현재 머신.
// 친구 판정: project_path 가 다른 경로(`/home/pasia/` 등) OR machine 이 원격 머신값.
const LOCAL_HOME_PREFIX = "/home/llm";

// machine 필드가 현재 머신을 가리키는가(또는 비었는가). 현재 머신 alias/hostname 후보 목록과 비교.
function isLocalMachineField(machine: string | null | undefined, selfNames: string[]): boolean {
  const m = (machine ?? "").trim().toLowerCase();
  if (!m) return true; // 비었으면 로컬로 간주(기존 로컬 에이전트는 machine 미설정).
  // "서울"/"seoul"/"local"/hostname/alias 등 현재 머신을 가리키면 로컬.
  if (m === "local" || m === "서울" || m === "seoul") return true;
  return selfNames.some((n) => n && (m === n || m.includes(n) || n.includes(m)));
}

// project_path 가 이 머신 HOME(/home/llm) 하위면 로컬 폴더.
function isLocalPath(projectPath: string | null | undefined): boolean {
  const p = (projectPath ?? "").trim();
  if (!p) return false; // 경로 정보 없으면 path 단독으로 로컬 단정 안 함(machine 으로 판정).
  return p === LOCAL_HOME_PREFIX || p.startsWith(LOCAL_HOME_PREFIX + "/");
}

// 로컬 에이전트 여부 — classification==="friend" 면 무조건 친구(명시 친구 등록).
// 그 외엔: machine 이 원격값이면 친구, project_path 가 다른 머신 경로면 친구. 둘 다 아니면 로컬.
function isLocalAgent(a: { machine?: string | null; project_path?: string | null; classification?: string | null }, selfNames: string[]): boolean {
  if ((a.classification ?? "") === "friend") return false; // 명시 친구.
  // 로컬/친구는 machine 필드로만 판정. project_path 기반 휴리스틱(하드코딩 /home/llm)은 제거 —
  //   머신마다 유저 홈이 다르다(서울 /home/llm, 잘만 /home/pasia). 그 하드코딩 때문에 잘만이
  //   자기 /home/pasia 에이전트를 친구로 오분류해 로컬 로스터에서 사라지던 버그(CLAUDE.md #7 위반).
  //   machine 이 비었거나 현재 머신 = 로컬, 원격 머신값 = 친구.
  if (!isLocalMachineField(a.machine, selfNames)) return false;
  return true;
}

// Messenger.tsx connTier 와 동일: 1h 이내 online.
function isOnline(lastSeen?: string): boolean {
  if (!lastSeen) return false;
  const t = Date.parse(lastSeen);
  if (Number.isNaN(t)) return false;
  return Date.now() - t < 60 * 60 * 1000;
}

function fmtClock(iso: string): string {
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  } catch {
    return "";
  }
}

// 명부 미리보기 시각: 오늘이면 HH:MM, 어제면 "어제", 그 외 M/D.
function fmtPreviewTime(iso: string): string {
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    const now = new Date();
    const sameDay = d.toDateString() === now.toDateString();
    if (sameDay) return fmtClock(iso);
    const yest = new Date(now);
    yest.setDate(now.getDate() - 1);
    if (d.toDateString() === yest.toDateString()) return "어제";
    return `${d.getMonth() + 1}/${d.getDate()}`;
  } catch {
    return "";
  }
}

export function TalkTab(props: { onJumpToSettings?: () => void; onRoomChange?: (open: boolean) => void }) {
  const [agents, { refetch: refetchAgents, mutate: mutateAgents }] = createResource<AgentRow[]>(() => invoke("agents_list"));
  const [addOpen, setAddOpen] = createSignal(false);
  // 👥 친구 추가 모달 — 종류(머신/외부 A2A) 선택. addOpen(에이전트 추가)와 별개.
  const [friendOpen, setFriendOpen] = createSignal(false);
  // 외부 A2A 친구(localStorage 영속) — 머신 친구는 agents_list 에 들어오지만 외부는 별도 소스.
  const [extFriends, setExtFriends] = createSignal<ExternalFriend[]>(loadExternalFriends());
  function reloadExtFriends() { setExtFriends(loadExternalFriends()); }
  // 상세 패널 "세션 재시작" 트리거 — 증가시키면 AcpConversation 이 세션을 닫고 재구동.
  const [restartTick, setRestartTick] = createSignal(0);
  // 우측 패널 "📥 가져오기" 트리거 — 증가시키면 AcpConversation 이 promptImport()(붙여넣기→me 버블) 실행.
  const [importTick, setImportTick] = createSignal(0);
  // 우측 패널 가져오기/보내기 상태 표시(복사됨 토스트).
  const [ioMsg, setIoMsg] = createSignal<string | null>(null);
  function flashIo(m: string) { setIoMsg(m); setTimeout(() => setIoMsg(null), 2200); }

  // 📋 다른 LLM 에게 줄 "가져오기 지침" — 그대로 복사해 외부 앱 LLM 에 주면, OpenXgram 가져오기에
  // 붙여넣기 좋은 형식으로 작업/대화를 정리해 내놓는다. (실제 import 는 우측 📥 가져오기 버튼.)
  const IMPORT_INSTRUCTIONS =
    "아래 형식으로 지금까지의 작업/대화를 정리해 출력해줘. 그대로 복사해 OpenXgram 가져오기에 붙여넣겠다.\n\n" +
    "## [목표]\n(이 작업의 최종 목표)\n\n" +
    "## [진행상황]\n(지금까지 한 일·현재 상태)\n\n" +
    "## [결정사항]\n(확정된 결정·선택)\n\n" +
    "## [다음 단계]\n(앞으로 해야 할 일)\n\n" +
    "## [관련 파일]\n(건드린/볼 파일 경로 목록)\n";

  // 클립보드 복사(비보안 컨텍스트/권한 거부는 토스트로 사유 안내). navigator.clipboard 우선.
  async function copyToClipboard(text: string, okMsg: string) {
    try {
      await navigator.clipboard.writeText(text);
      flashIo(okMsg);
    } catch {
      flashIo("⚠ 복사 실패 — 클립보드 권한/보안 컨텍스트 확인");
    }
  }

  // 📤 현재 대화를 마크다운 텍스트로 정리해 복사 — 외부 앱 LLM 에 붙여넣어 이어가기.
  // 출처=acp_conv_list(권위 DB, 영속 대화). 각 메시지를 **사용자:**/**에이전트:** + 시각으로.
  async function exportConversationText(alias: string, name: string) {
    try {
      const rows = await invoke<{ role: string; text: string; created_at?: string }[]>("acp_conv_list", { key: alias });
      if (!Array.isArray(rows) || rows.length === 0) { flashIo("내보낼 대화가 없습니다."); return; }
      const fmtTs = (ca?: string): string => {
        if (!ca) return "";
        const d = new Date(ca);
        if (Number.isNaN(d.getTime())) return "";
        const p = (n: number) => String(n).padStart(2, "0");
        return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
      };
      const lines: string[] = [`# ${name} — OpenXgram 대화 내보내기`, ""];
      for (const r of rows) {
        const ts = fmtTs(r.created_at);
        const body = (r.text ?? "").trim();
        if (!body) continue;
        if (r.role === "me") lines.push(`**사용자**${ts ? ` (${ts})` : ""}: ${body}`, "");
        else if (r.role === "agent") lines.push(`**에이전트**${ts ? ` (${ts})` : ""}: ${body}`, "");
        else if (r.role === "note") lines.push(`> ${body}`, "");
        // tool/plan(JSON)은 과정이므로 내보내기 본문에선 생략(외부 앱 LLM 가독성 우선).
      }
      await copyToClipboard(lines.join("\n"), `📤 대화 ${rows.length}건 복사됨 — 외부 앱에 붙여넣으세요`);
    } catch {
      flashIo("⚠ 대화 불러오기 실패");
    }
  }
  // 상세(info) 패널 너비 — 좌측 핸들 드래그로 조절(데스크톱). CSS 변수 --info-w 로 적용.
  const [infoWidth, setInfoWidth] = createSignal(250);
  function startInfoResize(e: MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = infoWidth();
    const onMove = (ev: MouseEvent) => {
      // 패널이 우측에 있어 왼쪽으로 드래그하면 넓어짐.
      setInfoWidth(Math.max(200, Math.min(680, startW + (startX - ev.clientX))));
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  // 대화명 편집 — 상세 패널에서. null=비편집, 문자열=편집중 값.
  const [renameVal, setRenameVal] = createSignal<string | null>(null);
  const [renameBusy, setRenameBusy] = createSignal(false);
  async function saveRename(alias: string) {
    const v = (renameVal() ?? "").trim();
    setRenameBusy(true);
    try {
      await invoke("agent_profile_set", { alias, display_name: v || alias });
      await refetchAgents();
      setRenameVal(null);
    } catch (e) {
      // 실패해도 편집 닫음(에러는 콘솔).
      console.error("rename failed", e);
      setRenameVal(null);
    } finally {
      setRenameBusy(false);
    }
  }

  // tmux 라이브 열기 — 새 창(?tmux=identifier). 창 재사용 시에도 명시적 이동(흰화면 방지).
  function openTmuxPopout(identifier: string, display: string) {
    const url = `${location.origin}${location.pathname}?tmux=${encodeURIComponent(identifier)}&label=${encodeURIComponent(display)}`;
    const w = window.open("", `oxgtmux_${identifier}`, "width=820,height=620");
    if (!w) { location.href = url; return; }
    w.location.href = url;
    w.focus();
  }
  const [peers] = createResource<PeerDto[]>(() => invoke("peers_list"), { initialValue: [] });
  const [recent] = createResource<MessageDto[]>(() => invoke("messages_recent", { limit: 100 }), { initialValue: [] });
  // 정보 패널 소스 — sessions(이 머신 tmux+워크트리) · workflows(orchestrator 매칭). 동적 only.
  const [sessions] = createResource<SessionsDto | null>(() => invoke("sessions"), { initialValue: null });
  const [workflows] = createResource<WorkflowDto[]>(() => invoke("workflows_list"), { initialValue: [] });
  // tailnet 장치 — 친구 머신명 칩 클릭 시 그 머신의 OpenXgram GUI(guiUrl)로 이동하기 위함.
  //   라우트 미배포/tailscale 부재면 graceful([] 유지) → 칩 클릭은 IP 폴백으로 동작.
  const [tailnetDevices] = createResource<TailnetDeviceDto[]>(
    async () => {
      try {
        const r = await invoke<{ devices?: TailnetDeviceDto[] }>("tailnet_devices", {});
        return Array.isArray(r?.devices) ? r.devices : [];
      } catch {
        return []; // graceful — 칩 클릭 폴백 경로가 처리.
      }
    },
    { initialValue: [] },
  );

  const [selected, setSelected] = createSignal<string | null>(null);
  const [mobileChat, setMobileChat] = createSignal(false);
  // 대화방(전체화면) 열림 여부를 셸로 통지 → 카톡처럼 대화방에선 하단 네비 숨김(에이전트 카드 목록에서만 노출).
  createEffect(() => props.onRoomChange?.(mobileChat()));
  // ACP picker 모드 — 어댑터를 직접 고르는 진입(에이전트 미선택). preset 경로와 별개.
  const [acpMode, setAcpMode] = createSignal(false);
  // 정보 사이드 패널(폴더·tmux·워크트리·워크플로우) 열림. tmux/worktree pill 클릭 → 토글, ✕ → 닫힘.
  const [infoOpen, setInfoOpen] = createSignal(false);

  // peers_list → alias 별 last_seen / machine 조회용 맵.
  const peerMap = createMemo(() => {
    const m = new Map<string, PeerDto>();
    for (const p of peers() ?? []) m.set(p.alias.toLowerCase(), p);
    return m;
  });

  // messages_recent → alias 별 마지막 메시지(미리보기/시각) 파생.
  // 매칭: sender 가 alias 와 일치하거나, conversation_id 가 alias 를 포함.
  const lastMsgByAlias = createMemo(() => {
    const map = new Map<string, MessageDto>();
    const consider = (key: string, m: MessageDto) => {
      const k = key.toLowerCase();
      const cur = map.get(k);
      if (!cur || Date.parse(m.timestamp) > Date.parse(cur.timestamp)) map.set(k, m);
    };
    const aliases = (agents() ?? []).map((a) => a.alias);
    for (const m of recent() ?? []) {
      const s = (m.sender || "").toLowerCase();
      const cid = (m.conversation_id || "").toLowerCase();
      for (const a of aliases) {
        const al = a.toLowerCase();
        if (s === al || s === `peer:${al}` || cid.includes(al)) consider(a, m);
      }
    }
    return map;
  });

  // 현재 머신 식별 — sessions().machine.alias/hostname. 로컬/친구 분류 기준.
  // 기본값 "server-seoul"(이 머신) — sessions 미로드 시 폴백.
  const selfMachineNames = createMemo<string[]>(() => {
    const m = sessions()?.machine;
    const names = ["server-seoul"];
    if (m?.alias) names.push(m.alias.toLowerCase());
    if (m?.hostname) names.push(m.hostname.toLowerCase());
    return [...new Set(names.filter(Boolean).map((n) => n.toLowerCase()))];
  });

  // 로컬 에이전트만 — 분류 그룹(👑/📌/📁/⚙️)에 들어감.
  const localAgents = createMemo<AgentRow[]>(() =>
    (agents() ?? []).filter((a) => isLocalAgent(a, selfMachineNames())),
  );
  // 친구 에이전트 — 다른 머신/외부(machine 원격 OR project_path 가 다른 경로 OR classification=friend).
  const friendAgents = createMemo<AgentRow[]>(() =>
    (agents() ?? []).filter((a) => !isLocalAgent(a, selfMachineNames())),
  );
  // 외부 A2A 친구(localStorage) → AgentRow 형태로 어댑팅(친구 섹션에 머신 친구와 함께 표시).
  const externalFriendRows = createMemo<AgentRow[]>(() =>
    extFriends().map((f) => ({
      alias: f.alias,
      role: "외부 A2A",
      description: f.url,
      classification: "friend",
      machine: f.url, // 표시·라우팅용 — 우측 패널이 a2a_send target 으로 쓸 값.
      ai_type: null,
    })),
  );

  // 분류 그룹화 — AgentsTab 와 동일 분류 키. pinned 는 별도 소스가 없어 비활성(빈 그룹 자동 숨김).
  // 로컬 에이전트만 그룹화(친구는 별도 "👥 친구" 섹션).
  const grouped = createMemo(() => {
    const by: Record<string, AgentRow[]> = { primary: [], pinned: [], project: [], special: [] };
    for (const a of localAgents()) {
      const cls = a.classification && by[a.classification] ? a.classification : "project";
      by[cls].push(a);
    }
    // 각 그룹 내부: online 먼저, 그다음 마지막 메시지 최신순.
    const ts = (a: AgentRow) => {
      const m = lastMsgByAlias().get(a.alias.toLowerCase());
      return m ? Date.parse(m.timestamp) : 0;
    };
    const onl = (a: AgentRow) => (isOnline(peerMap().get(a.alias.toLowerCase())?.last_seen) ? 1 : 0);
    const ur = (a: AgentRow) => a.unread ?? 0;
    void onl; void ur; // (정렬 단순화로 미사용 — 온라인 점·안읽음 배지는 렌더에서 직접 사용)
    for (const k of Object.keys(by)) {
      // 최신순(마지막 메시지 최근 먼저) — 단순·안정. 안읽음은 배지로만 표시(정렬 안 흔듦).
      by[k].sort((x, y) => ts(y) - ts(x));
    }
    return by;
  });

  // ➕ "추가되지 않은 에이전트" — detect_tmux(sessions) 의 tmux 세션 중 어느 에이전트의
  //   project_path(cwd) 와도 안 맞는 것 = 미등록. noise(데몬 자기 세션·null·시스템 세션) 제외.
  //   각 항목 클릭 → AddAgentModal 을 cwd prefill 로 열어 대화명(alias) 부여 = 등록.
  const registeredCwds = createMemo<Set<string>>(() => {
    const s = new Set<string>();
    for (const a of agents() ?? []) {
      const p = (a.project_path ?? "").trim();
      if (p) s.add(normPath(p));
    }
    return s;
  });

  // isMeaningfulSession 은 ./agentSessions 에서 import(공유). 아래는 그 위에서 미등록 세션 도출.
  const unregisteredSessions = createMemo<DetectedSession[]>(() =>
    computeUnregisteredSessions(
      sessions()?.sessions ?? [],
      (agents() ?? []).map((a) => a.project_path ?? "").filter(Boolean),
    ),
  );

  // AddAgentModal prefill(미등록 tmux "추가" 클릭 시 cwd 채워 열기). null 이면 일반 추가.
  const [addPrefillFolder, setAddPrefillFolder] = createSignal<string | null>(null);
  function addFromSession(s: DetectedSession) {
    setAddPrefillFolder(s.cwd ? s.cwd.trim() : null);
    setAddOpen(true);
  }

  // 선택 에이전트 — 로컬/친구 머신은 agents_list, 외부 A2A 친구는 externalFriendRows 에서 찾는다.
  const selAgent = createMemo(() => {
    const sel = selected();
    if (!sel) return null;
    return (agents() ?? []).find((a) => a.alias === sel)
      ?? externalFriendRows().find((a) => a.alias === sel)
      ?? null;
  });

  // 선택된 것이 친구(원격 머신 OR 외부 A2A)인가 — 우측 패널에서 로컬 ACP/파일트리 회피 판정.
  const selIsFriend = createMemo<boolean>(() => {
    const a = selAgent();
    if (!a) return false;
    if (extFriends().some((f) => f.alias === a.alias)) return true; // 외부 A2A.
    return !isLocalAgent(a, selfMachineNames()); // 원격 머신.
  });

  // 선택 에이전트 → ACP preset(어댑터/cwd/실행모드/라벨). 우측 대화방을 ACP 세션으로 구동.
  //   adapter   = ai_type 매핑(claude→claude-agent-acp, codex→codex-acp, gemini→gemini, 그 외 기본)
  //   cwd       = project_path (없으면 daemon 기본)
  //   execMode  = execution_mode (없으면 on_demand)
  const acpPreset = createMemo<AcpPreset | null>(() => {
    const a = selAgent();
    if (!a) return null;
    // 친구(원격·외부)는 로컬 ACP 세션을 구동하지 않는다 → 로컬 파일트리 404 회피.
    //   대신 우측에 "원격 친구 — A2A로 통신" 안내(아래 friend 패널). 대화/위임은 A2A 로.
    if (selIsFriend()) return null;
    return {
      adapter: aiTypeToAdapter(a.ai_type),
      cwd: a.project_path ?? null,
      execMode: a.execution_mode ?? null,
      label: a.alias, // convKey(영속화 키) — 안정적 alias 유지.
      displayName: agentName(a), // 헤더 표시용(대화명).
      classification: a.classification ?? null, // primary 면 권한 기본 bypass.
      machine: a.machine ?? null, // 원격이면 데몬이 SSH 로 그 머신에서 ACP spawn.
      permMode: a.perm_mode ?? null, // 에이전트별 영속 컴포저 설정(없으면 컴포저 기본값).
      model: a.model ?? null,
      thinking: a.thinking ?? null,
    };
  });

  // selected 변경 시 정보 패널 닫음(다른 에이전트로 이동하면 상태 초기화).
  createEffect(() => { selected(); setInfoOpen(false); });

  // 선택 에이전트의 tmux 세션 — rc.281: 현재 대화의 **cwd(작업 폴더)** 매칭 우선 + alias 매칭 보조.
  //   대화(AcpConversation)는 선택 에이전트의 project_path 를 cwd 로 구동(line 259). 그 cwd 에서
  //   실행 중인 tmux 세션을 정보 패널에 보여줘야 한다(반복 지적 문제). 이전엔 alias 만 비교해
  //   세션명≠alias 면 안 보였음. 이제 세션 cwd(`#{pane_current_path}`) == 대화 cwd 면 표시.
  //   tmux kind 만. 매칭 없으면 빈 배열 → 패널 빈 상태 힌트. (normPath 는 ./agentSessions import.)
  const selSessions = createMemo<DetectedSession[]>(() => {
    const alias = (selected() ?? "").toLowerCase();
    if (!alias) return [];
    const convoCwd = normPath((selAgent()?.project_path ?? "").trim());
    const regs = registeredCwds();
    const all = sessions()?.sessions ?? [];
    return all.filter((s) => {
      if (s.kind !== "tmux") return false;
      // 1) cwd 매칭 — 세션 pane cwd == 현재 대화 폴더, 또는 그 폴더의 하위 폴더(descendant).
      //    단, descendant 는 "가장 가까운(최장 prefix) 등록 에이전트"에게만 귀속한다.
      //    상위 폴더 에이전트(예: Starian)가 하위 다른 에이전트의 tmux/worktree 를
      //    전부 흡수하던 버그(prefix-ownership leak) 방지 — longest-prefix match.
      const sCwd = s.cwd ? normPath(s.cwd.trim()) : "";
      if (convoCwd && sCwd) {
        if (sCwd === convoCwd) return true; // 정확히 같은 폴더 → 항상 내 것.
        // convoCwd 가 홈루트급(/home/llm 등)이면 descendant 흡수 금지 — star 가 홈 아래
        // orphan tmux 를 전부 빨아들이던 문제 방지(미등록 섹션과 동일 원리). exact 만 허용.
        if (sCwd.startsWith(convoCwd + "/") && !isTooBroadPath(convoCwd)) {
          // 이 세션 cwd 의 더 가까운(긴) 조상 폴더를 가진 다른 등록 에이전트가 있으면 그쪽 것.
          let closest = convoCwd;
          for (const r of regs) {
            if (r === convoCwd) continue;
            if ((sCwd === r || sCwd.startsWith(r + "/")) && r.length > closest.length) closest = r;
          }
          if (closest === convoCwd) return true; // 내가 가장 가까운 조상 → 내 것.
          // else: 더 구체적인 에이전트 폴더 존재 → 여기선 제외(그 에이전트에 표시됨).
        }
      }
      // 2) alias 매칭(보조) — cwd 미보유(구버전·원격) 또는 세션명=alias 인 경우 폴백.
      const aid = (s.agent_id ?? "").toLowerCase();
      const disp = (s.display ?? "").toLowerCase();
      const ident = (s.identifier ?? "").toLowerCase();
      return aid === alias || disp === alias || ident === alias || ident === `tmux:${alias}`;
    });
  });

  // 선택 에이전트 워크트리 — 매칭된 세션들의 nested worktrees 합집합(path 기준 dedup).
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

  // 폴더 끝 세그먼트만 짧게(목업 .sn 처럼). 전체 경로는 title 로.
  const baseName = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

  // 참여 중 워크플로우 — orchestrator 가 이 에이전트인 것만(실제 소유 필드). 없으면 빈 배열.
  const selWorkflows = createMemo<WorkflowDto[]>(() => {
    const alias = (selected() ?? "").toLowerCase();
    if (!alias) return [];
    return (workflows() ?? []).filter((w) => (w.orchestrator ?? "").toLowerCase() === alias);
  });

  // 세션 시작 시각 — 목업 "claude · 9:02~" 의 시각 부분.
  const sessStart = (s: DetectedSession) => fmtClock(s.created_at ?? s.last_active_at ?? "");

  // 에이전트 선택 → 대화 = ACP 세션(preset). 좌측 명부 row 클릭으로 진입.
  function pick(alias: string) {
    setAcpMode(false);
    setSelected(alias);
    setMobileChat(true);
    // 대화 열람 = 읽음 처리(백엔드 기록). 전체 refetchAgents 는 금지 — 로스터 재렌더 +
    // 재정렬로 선택 에이전트가 스크롤에서 사라지는 버그. 대신 로컬 mutate 로 해당 에이전트
    // unread=0 만 즉시 반영 → 배지가 클릭 즉시 사라진다(폴링 없어 '다음 갱신'이 안 와서 1로 멈추던 버그 fix).
    void invoke("acp_conv_read", { key: alias }).catch(() => {});
    mutateAgents((prev) => (prev ?? []).map((a) => (a.alias === alias ? { ...a, unread: 0 } : a)));
  }

  // ⚡ 에이전트 미리 정하지 않고 ACP 어댑터 picker 진입(기존 경로 유지).
  function openAcp() {
    setAcpMode(true);
    setMobileChat(true);
  }

  // IP 형태 판별(IPv4) — guiUrl 폴백에서 "http://IP:포트/gui/" 로 직접 열 수 있는지.
  const isIpLike = (s: string) => /^\d{1,3}(\.\d{1,3}){3}$/.test(s.trim());

  // 🖥 친구 머신명 칩 클릭 — 그 친구 머신의 OpenXgram GUI 페이지를 새 탭으로 연다.
  //   매칭: a.machine(IP 또는 이름)을 tailnetDevices 에서 ip/name 정확·포함 비교로 찾는다.
  //   1) 매칭 device 에 guiUrl 있으면 그걸 연다(포트는 설치마다 다르므로 권위 소스).
  //   2) 없으면 폴백 — machine 이 IP 형태면 기본 포트(47302)로 시도 + 토스트 안내,
  //      아니면(이름만) 토스트로 안내만(절대 깨지지 않게). e.stopPropagation 으로 카드 선택과 분리.
  function openFriendGui(e: MouseEvent, machine: string | null | undefined) {
    e.stopPropagation();
    const m = (machine ?? "").trim();
    if (!m) return;
    const ml = m.toLowerCase();
    const devs = tailnetDevices() ?? [];
    const dev =
      devs.find((d) => (d.ip ?? "").trim() === m || (d.name ?? "").trim().toLowerCase() === ml) ??
      devs.find((d) => {
        const ip = (d.ip ?? "").trim();
        const nm = (d.name ?? "").trim().toLowerCase();
        return (ip && (ip.includes(m) || m.includes(ip))) || (nm && (nm.includes(ml) || ml.includes(nm)));
      });
    const guiUrl = (dev?.guiUrl ?? "").trim();
    if (guiUrl) {
      window.open(guiUrl, "_blank");
      return;
    }
    // 폴백 — 포트 자동탐지 실패.
    if (isIpLike(m)) {
      window.open(`http://${m}:47302/gui/`, "_blank");
      flashIo("포트 자동탐지 실패 — 기본 포트(47302)로 시도");
    } else {
      flashIo(`'${m}' GUI 주소를 찾지 못했습니다 — tailnet 장치 목록에 없음`);
    }
  }

  // .st 미리보기: 마지막 메시지 본문, 없으면 역할/설명.
  function preview(a: AgentRow): string {
    const m = lastMsgByAlias().get(a.alias.toLowerCase());
    if (m && m.body) return m.body.replace(/\n+/g, " ").trim();
    return a.role || a.description || a.machine || "—";
  }
  function previewTime(a: AgentRow): string {
    const m = lastMsgByAlias().get(a.alias.toLowerCase());
    return m ? fmtPreviewTime(m.timestamp) : "";
  }

  // llm-type 태그 라벨: ai_type, 공개면 "공개".
  function tagLabel(a: AgentRow): string | null {
    return a.ai_type || null;
  }

  return (
    <div class={`kk-talk${mobileChat() ? " mchat" : ""}`}>
      {/* ── 좌측 명부 (정본: side-top + search + 분류 group-title + row) ── */}
      <div class="kk-talk-roster">
        <div class="side-top">
          <h1>OpenXgram</h1>
          <button class="add-btn" onClick={() => setAddOpen(true)}>＋ 에이전트 추가</button>
        </div>
        <div class="search">🔍 에이전트·대화 검색</div>

        <div class="list">
          <Show when={!agents.loading} fallback={<div class="empty">불러오는 중…</div>}>
            <Show when={!agents.error} fallback={<div class="empty">명부를 불러오지 못했습니다.<br />데몬 연결을 확인하세요.</div>}>
              <Show
                when={(agents() ?? []).length > 0}
                fallback={<div class="empty">등록된 에이전트가 없습니다.<br /><b>에이전트</b> 탭에서 등록하세요.</div>}
              >
                <For each={CLASS_GROUPS}>
                  {(g) => (
                    <Show when={(grouped()[g.key] ?? []).length > 0}>
                      <div class="group-title">
                        {g.title} <span class="gt-sub">({grouped()[g.key].length})</span>
                      </div>
                      <For each={grouped()[g.key]}>
                        {(a) => {
                          const online = () => isOnline(peerMap().get(a.alias.toLowerCase())?.last_seen);
                          return (
                            <div
                              class={`row${selected() === a.alias ? " active" : ""}${g.key === "primary" ? " primary" : ""}`}
                              onClick={() => pick(a.alias)}
                            >
                              <div class={`ava ${g.key === "primary" ? "c-primary" : avatarColor(a.ai_type)}`}>
                                {g.key === "primary" ? "👑" : a.alias.slice(0, 1).toUpperCase()}
                                <span class={`dot${online() ? " on" : ""}`} />
                              </div>
                              {/* 컴팩트 2줄 카드:
                                  1줄 = 대화명(=alias, 큰 글씨) · AI모델 칩 · 역할 배지 · (우측) 안읽음 배지
                                  2줄 = 최신 입력내용 미리보기(말줄임) · (우측) 시각
                                  대화명=alias 일관(신원). 위임·통신이 이 alias 로 됨. */}
                              <div class="meta">
                                <div class="kk-card-l1">
                                  <span class="nm" title={`@${a.alias}`}>{agentName(a)}</span>
                                  <ProviderLogo provider={providerKey(a)} />
                                  {/* 로컬 카드는 머신 태그 없음 — 로컬이 기본값이라 태그는 노이즈.
                                      머신 태그(🖥)는 "다른 머신(친구)"라는 예외만 표시(친구 카드 전용). */}
                                  <Show when={a.role && a.role.trim()}>
                                    <span class="kk-card-role" title="역할">{a.role!.trim()}</span>
                                  </Show>
                                  <Show when={a.is_public}><span class="tag">공개</span></Show>
                                  <Show when={(a.unread ?? 0) > 0}>
                                    <span class="kk-unread kk-card-unread">{(a.unread ?? 0) > 99 ? "99+" : a.unread}</span>
                                  </Show>
                                </div>
                                <div class="kk-card-l2">
                                  <span class="st">{preview(a)}</span>
                                  <Show when={previewTime(a)}>
                                    <span class="kk-card-time">{previewTime(a)}</span>
                                  </Show>
                                </div>
                              </div>
                            </div>
                          );
                        }}
                      </For>
                    </Show>
                  )}
                </For>
                {/* 추가되지 않은 에이전트(미등록 tmux) 섹션은 '에이전트 탭'으로 이동 — 대화 탭은 대화만. */}

                {/* 👥 친구 (다른 머신·외부) — 머신 친구(agents_list 의 원격) + 외부 A2A(localStorage).
                    원격이라 로컬 파일트리 없음 → 선택 시 우측에 "A2A로 통신" 안내. */}
                <Show when={friendAgents().length + externalFriendRows().length > 0}>
                  <div class="group-title">
                    👥 친구 (다른 머신·외부) <span class="gt-sub">({friendAgents().length + externalFriendRows().length})</span>
                  </div>
                  <For each={[...friendAgents(), ...externalFriendRows()]}>
                    {(a) => {
                      const online = () => isOnline(peerMap().get(a.alias.toLowerCase())?.last_seen);
                      const isExt = () => extFriends().some((f) => f.alias === a.alias);
                      return (
                        <div
                          class={`row${selected() === a.alias ? " active" : ""}`}
                          onClick={() => pick(a.alias)}
                        >
                          <div class={`ava ${avatarColor(a.ai_type)}`}>
                            {isExt() ? "🌐" : "🖥"}
                            <span class={`dot${online() ? " on" : ""}`} />
                          </div>
                          <div class="meta">
                            <div class="kk-card-l1">
                              <span class="nm" title={`@${a.alias}`}>{agentName(a)}</span>
                              {/* 머신명 칩 — 외부 A2A 는 🌐 외부, 머신 친구는 🖥 머신명(a.machine).
                                  "원격" 텍스트 태그 대체. machine 없으면 칩 생략. */}
                              <Show when={isExt() || (a.machine && a.machine.trim())}>
                                <Show
                                  when={!isExt()}
                                  fallback={
                                    <span class="kk-card-mach" title="외부 A2A 에이전트">🌐 외부</span>
                                  }
                                >
                                  {/* 머신명 칩 클릭 → 그 친구 머신의 OpenXgram GUI 열기(guiUrl/폴백).
                                      stopPropagation 으로 카드 선택(대화 열기)과 분리. */}
                                  <span
                                    class="kk-card-mach"
                                    style="cursor:pointer;"
                                    title="OpenXgram 페이지 열기"
                                    onClick={(e) => openFriendGui(e, a.machine)}
                                  >
                                    🖥 {a.machine!.trim()}
                                  </span>
                                </Show>
                              </Show>
                              <Show when={a.role && a.role.trim()}>
                                <span class="kk-card-role" title="역할">{a.role!.trim()}</span>
                              </Show>
                              <Show when={(a.unread ?? 0) > 0}>
                                <span class="kk-unread kk-card-unread">{(a.unread ?? 0) > 99 ? "99+" : a.unread}</span>
                              </Show>
                            </div>
                            <div class="kk-card-l2">
                              <span class="st" title={a.machine ?? ""}>{isExt() ? "외부 A2A — AgentCard URL 로 통신" : "다른 머신 — A2A/peer 통신"}</span>
                              <Show when={previewTime(a)}>
                                <span class="kk-card-time">{previewTime(a)}</span>
                              </Show>
                            </div>
                          </div>
                        </div>
                      );
                    }}
                  </For>
                </Show>

                {/* ➕ 친구 추가 — 종류 선택(머신 / 외부 A2A) 모달. */}
                <div class="row kk-unreg-row" title="다른 머신·외부 에이전트를 친구로 추가" onClick={() => setFriendOpen(true)}>
                  <div class="ava c-group">＋<span class="dot" /></div>
                  <div class="meta">
                    <div class="kk-card-l1">
                      <span class="nm">➕ 친구 추가</span>
                    </div>
                    <div class="kk-card-l2">
                      <span class="st">머신 · 외부 A2A</span>
                    </div>
                  </div>
                </div>
              </Show>
            </Show>
          </Show>
        </div>
      </div>

      {/* ── 우측 대화방 ──
          1) acpMode  : 어댑터 picker(에이전트 미리 안 정함)
          2) 에이전트 선택 : 그 에이전트의 ACP 세션(preset 구동·스트리밍)
          ACP 가 유일한 대화 메커니즘 — 레거시 peer_send 경로는 제거됨(Phase 4-4). */}
      <Show when={acpMode()}>
        <AcpConversation onClose={() => { setAcpMode(false); setMobileChat(false); }} />
      </Show>
      <Show when={!acpMode() && acpPreset()}>
        {/* key=alias 로 에이전트 전환 시 컴포넌트 재마운트 → 새 ACP 세션 구동 */}
        <Show when={selected()} keyed>
          {(_alias) => (
            <AcpConversation
              preset={acpPreset()}
              popoutAlias={selAgent()?.alias ?? null}
              restartTrigger={restartTick}
              importTrigger={importTick}
              status={() => ({
                folder: selAgent()?.project_path ?? null,
                role: selAgent()?.role ?? null,
                isPublic: !!selAgent()?.is_public,
                workflows: selWorkflows().length,
              })}
              onClose={() => setMobileChat(false)}
              // ⌗ 상태 토글을 ACP 헤더 pill 행(.meta-r) 왼쪽에 인라인 배치 →
              // 스트리밍/⚡ACP/✕닫기 pill 과 겹치지 않음(절대 배치 제거).
              headerExtra={() => (
                <span class="pill clk" onClick={() => setInfoOpen((v) => !v)}>
                  ⌗ 작업환경{selSessions().length + selWorktrees().length > 0 ? ` ${selSessions().length + selWorktrees().length}` : ""}
                </span>
              )}
            />
          )}
        </Show>
      </Show>
      {/* 친구(원격·외부) 선택 — 로컬 ACP/파일트리 시도 금지. A2A 로 통신. */}
      <Show when={!acpMode() && selIsFriend() && selAgent()}>
        {(a) => <FriendPanel agent={a()} isExternal={extFriends().some((f) => f.alias === a().alias)} onClose={() => setMobileChat(false)} />}
      </Show>
      <Show when={!acpMode() && !selIsFriend() && !acpPreset()}>
        <div class="kk-talk-chat">
          <div class="kk-talk-blank">좌측에서 대화할 에이전트를 선택하세요.</div>
        </div>
      </Show>

      {/* ── 정보 사이드 패널 (정본 #info) — 선택 에이전트의 폴더·tmux·워크트리·워크플로우.
          토글은 ACP 헤더(.meta-r)에 인라인 ⌗ 상태 pill 로 배치됨(headerExtra). 기본 닫힘.
          열리면 우측 300px 슬라이드인으로 대화창 우측 일부만 오버레이(전체 가리지 않음). ── */}
      <Show when={selAgent()}>
        {(a) => (
          <div class={`info${infoOpen() ? " show" : ""}`} style={`--info-w:${infoWidth()}px`}>
            <div class="kk-info-resize" onMouseDown={startInfoResize} title="너비 조절 (드래그)" />
            <div class="info-head">
              <Show
                when={renameVal() !== null}
                fallback={
                  <span class="t">
                    {agentName(a())} · 상태
                    <span class="kk-rename-btn" title="대화명 수정" onClick={() => setRenameVal(agentName(a()))}>✏</span>
                  </span>
                }
              >
                <span class="kk-rename-edit">
                  <input
                    value={renameVal() ?? ""}
                    disabled={renameBusy()}
                    placeholder="대화명"
                    onInput={(e) => setRenameVal(e.currentTarget.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") void saveRename(a().alias); if (e.key === "Escape") setRenameVal(null); }}
                  />
                  <button disabled={renameBusy()} onClick={() => void saveRename(a().alias)}>저장</button>
                  <button disabled={renameBusy()} onClick={() => setRenameVal(null)}>취소</button>
                </span>
              </Show>
              <span class="x" onClick={() => setInfoOpen(false)}>✕</span>
            </div>

            {/* 세션 제어 — 닫기는 헤더 대신 여기서. 닫으면 재구동되어 대화창 복귀. */}
            <div class="info-actions">
              <button class="kk-restart-btn" onClick={() => setRestartTick((n) => n + 1)} title="ACP 세션을 닫고 다시 구동(대화 복원)">
                ↻ 세션 재시작
              </button>
            </div>

            {/* ── 가져오기/보내기 — 외부 앱 LLM(ChatGPT 등) 과의 대화 핸드오프.
                자주 안 쓰는 기능이라 컴포저에서 빼고 여기로. 내부 에이전트 위임은 컴포저 ⇢ 모달.
                ① 📋 가져오기 지침 복사 — 외부 LLM 에게 줄 "정리해줘" 지침(붙여넣기 좋은 형식 유도)
                ② 📥 가져오기 — 외부에서 정리한 내용을 붙여넣어 현재 대화에 들임(promptImport, me 버블)
                ③ 📤 대화 텍스트 복사 — 현재 대화를 마크다운으로 복사해 외부 앱에 이어가기. ── */}
            <div>
              <h3>가져오기 / 보내기</h3>
              <div style="display:flex;flex-direction:column;gap:6px;">
                <button
                  class="kk-io-btn"
                  style="text-align:left;padding:7px 10px;border:1px solid #2a2f3a;border-radius:8px;background:#1a1d24;color:#cfe3d6;cursor:pointer;font-size:12px;"
                  title="외부 앱 LLM 에게 줄 '작업 정리' 지침을 클립보드에 복사"
                  onClick={() => void copyToClipboard(IMPORT_INSTRUCTIONS, "📋 가져오기 지침 복사됨 — 외부 LLM 에 붙여넣어 정리시키세요")}
                >📋 가져오기 지침 복사</button>
                <button
                  class="kk-io-btn"
                  style="text-align:left;padding:7px 10px;border:1px solid #2a2f3a;border-radius:8px;background:#1a1d24;color:#cfe3d6;cursor:pointer;font-size:12px;"
                  title="다른 LLM/에이전트 작업을 붙여넣어 현재 대화에 들이기"
                  onClick={() => setImportTick((n) => n + 1)}
                >📥 가져오기 (붙여넣기)</button>
                <button
                  class="kk-io-btn"
                  style="text-align:left;padding:7px 10px;border:1px solid #2a2f3a;border-radius:8px;background:#1a1d24;color:#cfe3d6;cursor:pointer;font-size:12px;"
                  title="현재 대화를 마크다운으로 복사 → 외부 앱(ChatGPT 등)에 붙여넣어 이어가기"
                  onClick={() => void exportConversationText(a().alias, agentName(a()))}
                >📤 대화 텍스트 복사</button>
                <Show when={ioMsg()}>
                  <div style="font-size:11px;color:#7fc99a;line-height:1.4;">{ioMsg()}</div>
                </Show>
              </div>
            </div>

            <div>
              <h3>폴더</h3>
              <Show when={a().project_path} fallback={<div class="folder">—</div>}>
                <div class="folder" title={a().project_path!}>{a().project_path}</div>
              </Show>
            </div>

            <div>
              <h3>
                실행 중 tmux · {selSessions().length}{" "}
                <span style="font-weight:500;color:#b6bcc6">(클릭 → 라이브 열기)</span>
              </h3>
              <Show
                when={selSessions().length > 0}
                fallback={<div class="info-empty">이 머신에서 감지된 tmux 세션이 없습니다.</div>}
              >
                <For each={selSessions()}>
                  {(s) => (
                    <div class="sess" style="cursor:pointer;" title="클릭 → 라이브 화면 새 창" onClick={() => openTmuxPopout(s.identifier, s.display || s.identifier)}>
                      <span class="sd" />
                      <span class="sn">{s.display || s.identifier}</span>
                      <span class="sx">{s.kind}{sessStart(s) ? ` · ${sessStart(s)}~` : ""}</span>
                    </div>
                  )}
                </For>
              </Show>
            </div>

            <div>
              <h3>워크트리 · {selWorktrees().length}</h3>
              <Show
                when={selWorktrees().length > 0}
                fallback={<div class="info-empty">git 워크트리가 없습니다.</div>}
              >
                <For each={selWorktrees()}>
                  {(w) => (
                    <div class="wt" title={w.path}>
                      🌿 {baseName(w.path)}
                      <Show when={w.branch}><span class="b">{w.branch}</span></Show>
                    </div>
                  )}
                </For>
              </Show>
            </div>

            <div>
              <h3>참여 중 워크플로우 · {selWorkflows().length}</h3>
              <Show
                when={selWorkflows().length > 0}
                fallback={<div class="info-empty">참여 중인 워크플로우가 없습니다.</div>}
              >
                <For each={selWorkflows()}>
                  {(w) => (
                    <div class="sess">
                      <span class="sd" />
                      <span class="sn" style="font-family:inherit;">{w.name}</span>
                      <span class="sx">{w.enabled === false ? "중지됨" : "활성"}</span>
                    </div>
                  )}
                </For>
              </Show>
            </div>

            <div style="font-size:11px;color:#b6bcc6;line-height:1.5;">
              정보·설정 수정은 <b>에이전트 탭</b>에서.
            </div>
          </div>
        )}
      </Show>

      {/* 에이전트 추가 모달 — ＋ 버튼으로 열림. 만들면 로스터 새로고침 + 자동 선택. */}
      <Show when={addOpen()}>
        <AddAgentModal
          prefillFolder={addPrefillFolder()}
          onClose={() => { setAddOpen(false); setAddPrefillFolder(null); }}
          onCreated={(alias) => {
            setAddOpen(false);
            setAddPrefillFolder(null);
            void refetchAgents();
            setSelected(alias);
          }}
        />
      </Show>

      {/* 👥 친구 추가 모달 — 종류(머신/외부 A2A) 선택. 머신=agents_register 재사용, 외부=localStorage. */}
      <Show when={friendOpen()}>
        <AddFriendModal
          onClose={() => setFriendOpen(false)}
          onCreated={(alias, kind) => {
            setFriendOpen(false);
            if (kind === "machine") void refetchAgents();
            else reloadExtFriends(); // 외부 A2A — localStorage 재로드.
            setSelected(alias);
          }}
        />
      </Show>
    </div>
  );
}

// 친구(원격 머신·외부 A2A) 대화 패널 — 로컬 ACP/파일트리 없이 A2A 로 통신.
//   target = 외부면 AgentCard URL(machine 필드에 저장), 머신 친구면 alias(내부 alias 라우팅).
//   a2a_send(target, task, from_agent) — endpoint 생략 시 데몬 기본(외부 URL or 내부 alias).
function FriendPanel(props: { agent: AgentRow; isExternal: boolean; onClose: () => void }) {
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [log, setLog] = createSignal<{ role: "me" | "agent" | "err"; text: string }[]>([]);
  // 외부 A2A: target = AgentCard URL(machine 에 저장됨). 머신 친구: target = alias(내부 라우팅).
  const target = () => (props.isExternal ? (props.agent.machine ?? "").trim() : props.agent.alias);

  async function send() {
    const text = draft().trim();
    if (!text || busy()) return;
    setBusy(true);
    setLog((l) => [...l, { role: "me", text }]);
    setDraft("");
    try {
      const r = await invoke<{ result?: { text?: string } }>("a2a_send", {
        target: target(), task: text, from_agent: "Starian",
      });
      const ans = r?.result?.text?.trim() || "(응답 텍스트 없음)";
      setLog((l) => [...l, { role: "agent", text: ans }]);
    } catch (e) {
      setLog((l) => [...l, { role: "err", text: `A2A 전송 실패: ${(e as Error)?.message ?? e}` }]);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="kk-talk-chat">
      <div class="kk-friend-panel" style="display:flex;flex-direction:column;height:100%;">
        <div style="padding:14px 16px;border-bottom:1px solid #2a2f3a;">
          <div style="font-size:15px;font-weight:600;color:#cfe3d6;">
            {(props.isExternal ? "🌐 " : "🖥 ") + agentName(props.agent)}
            <span
              title={props.isExternal ? "외부 A2A 에이전트" : "다른 머신의 에이전트(친구)"}
              style="margin-left:8px;display:inline-flex;align-items:center;gap:4px;padding:2px 10px;border-radius:999px;font-size:11px;font-weight:600;letter-spacing:0.2px;vertical-align:middle;background:rgba(124,150,255,0.16);color:#aebfff;border:1px solid rgba(124,150,255,0.30);"
            >
              <span style="width:5px;height:5px;border-radius:999px;background:#7c96ff;display:inline-block;"></span>
              {props.isExternal ? "외부 A2A" : (props.agent.machine || "다른 머신")}
            </span>
          </div>
          <div style="font-size:12px;color:#9aa1ad;margin-top:6px;line-height:1.5;">
            {props.isExternal
              ? "외부 A2A 에이전트 — 로컬 파일트리 없음. AgentCard URL 로 A2A 통신합니다."
              : "다른 머신의 에이전트 — 로컬 파일트리 없음. 그쪽 primary 가 자기 에이전트를 처리합니다. A2A/peer 로 통신하세요."}
          </div>
          <div style="font-size:11px;color:#6b7280;margin-top:4px;" title={target()}>대상: {target() || "(주소 미상)"}</div>
        </div>
        <div style="flex:1;overflow-y:auto;padding:14px 16px;display:flex;flex-direction:column;gap:8px;">
          <Show when={log().length === 0}>
            <div style="color:#6b7280;font-size:13px;text-align:center;margin-top:24px;">
              아래에 메시지를 입력해 A2A 로 위임·대화하세요.
            </div>
          </Show>
          <For each={log()}>
            {(m) => (
              <div style={`align-self:${m.role === "me" ? "flex-end" : "flex-start"};max-width:78%;padding:8px 11px;border-radius:10px;font-size:13px;line-height:1.5;white-space:pre-wrap;` +
                (m.role === "me" ? "background:#2f5d3a;color:#eafff0;" : m.role === "err" ? "background:#4a2222;color:#ffd6d6;" : "background:#1a1d24;color:#cfe3d6;border:1px solid #2a2f3a;")}>
                {m.text}
              </div>
            )}
          </For>
        </div>
        <div style="padding:12px 14px;border-top:1px solid #2a2f3a;display:flex;gap:8px;">
          <textarea
            class="ctl"
            style="flex:1;resize:none;height:42px;font-size:13px;"
            placeholder={target() ? "A2A 메시지·위임 내용…" : "주소가 없어 전송 불가"}
            value={draft()}
            disabled={!target() || busy()}
            onInput={(e) => setDraft(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void send(); } }}
          />
          <button class="btn-go" style="white-space:nowrap;" disabled={!target() || busy() || !draft().trim()} onClick={() => void send()}>
            {busy() ? "전송 중…" : "⇢ 보내기"}
          </button>
        </div>
      </div>
    </div>
  );
}
