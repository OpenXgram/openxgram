import { createSignal, createEffect, createMemo, createResource, onCleanup, For, Show } from "solid-js";
import { acpFetch, acpStream, invoke } from "../api/client";

// ai_type(에이전트 명부의 LLM 종류) → ACP 어댑터 이름 매핑.
// daemon registry(openxgram_acp::registry)의 어댑터 키와 1:1. 미인식 → claude 기본.
const AI_TYPE_TO_ADAPTER: Record<string, string> = {
  claude: "claude-agent-acp",
  codex: "codex-acp",
  gemini: "gemini",
};
export function aiTypeToAdapter(aiType?: string | null): string {
  const k = (aiType ?? "").toLowerCase();
  return AI_TYPE_TO_ADAPTER[k] ?? "claude-agent-acp";
}

// 미리 정한 에이전트로 ACP 세션을 구동할 때의 props(TalkTab roster 선택용).
// 생략 시(picker 경로) 기존 에이전트 선택 화면을 그대로 사용.
export interface AcpPreset {
  // ACP 어댑터 이름(aiTypeToAdapter 결과). 이것으로 spawn.
  adapter: string;
  // 작업 디렉토리. null/undefined 면 daemon 기본 cwd.
  cwd?: string | null;
  // 실행 모드(always|on_demand|heartbeat). 생략 시 on_demand.
  execMode?: string | null;
  // 대화 헤더에 표시할 라벨(에이전트 alias). 생략 시 adapter 이름. convKey(영속화)로도 사용 — 안정적 유지.
  label?: string | null;
  // 대화명(표시 이름). 헤더에 label 대신 노출(convKey 는 label 유지). 생략 시 label.
  displayName?: string | null;
  // 분류(primary/project/special). primary 면 권한 기본값 = bypassPermissions(전체 도구 권한).
  classification?: string | null;
  // cross-machine — 에이전트 머신. 원격이면 데몬이 ACP 어댑터를 SSH 로 그 머신에서 spawn.
  machine?: string | null;
  // 에이전트별 영속 컴포저 설정(재부팅 유지). 없으면 기본값(bypassPermissions/default/high).
  permMode?: string | null;
  model?: string | null;
  thinking?: string | null;
}

// ACP 대화방 (Phase B-3) — 로컬 ACP 에이전트 subprocess 를 daemon `/v1/acp/*` 로
// 구동하고 `session/update` SSE 를 카카오톡 정본 대화 UI(.msgs/.me/.agent/.toolcall/
// pre.code/composer)로 렌더. peer 대화(TalkTab)와 동일한 마크업·CSS 재사용 —
// 데이터 출처만 peer_send/peer_conversation 대신 ACP 스트림.
//
// daemon 계약(daemon_gui_acp.rs)과 1:1:
//   POST /sessions            body {agent, cwd, executionMode?}  → {sessionId, agent, cwd, executionMode, spawned}
//   POST /sessions/{id}/prompt body {text}                       → {stopReason, updates}
//   GET  /sessions/{id}/stream  (SSE event: session_update)      → session/update params
//   POST /sessions/{id}/cancel                                   → session/cancel 결과
//   DELETE /sessions/{id}                                        → close + reap
//
// session/update payload(types.rs SessionUpdate, sessionUpdate 태그 snake_case):
//   agent_message_chunk / agent_thought_chunk / user_message_chunk → {content:{type:"text",text}}
//   tool_call / tool_call_update → {toolCallId,title,kind,status,content[]}
//   plan → {entries:[{content,priority,status}]}

// ACP 세션 기본 작업 디렉토리 — 라벨 상수(흩뿌리지 않음). 향후 설정값으로 대체 가능.
const DEFAULT_ACP_CWD = "/home/llm/projects/starian-set/openxgram";

interface AgentInfo {
  name: string;
  installed: boolean;
}

// 대화 버블 모델 — peer 대화와 동일한 시각 표현으로 매핑.
type Bubble =
  | { id: number; kind: "me"; text: string; time: string }
  | { id: number; kind: "agent"; segs: Seg[]; time: string }
  | { id: number; kind: "tool"; toolId: string; title: string; status: string; time: string }
  | { id: number; kind: "plan"; entries: { content: string; status: string }[]; time: string }
  | { id: number; kind: "note"; text: string; time: string };

type Seg = { kind: "text"; text: string } | { kind: "code"; text: string };

// spawn() 인자 — preset/picker 진입 + 칩 변경 재구동 공용.
type SpawnArgs = {
  cwd?: string | null;
  execMode?: string | null;
  label?: string | null;
  keepHistory?: boolean;
};

// 에이전트 본문을 펜스드 코드블록 기준으로 text/code 분해 (TalkTab.segmentBody 와 동일 정책).
function segmentText(body: string): Seg[] {
  const out: Seg[] = [];
  const fence = /```[\w-]*\n?([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = fence.exec(body)) !== null) {
    if (m.index > last) {
      const t = body.slice(last, m.index).replace(/^\n+|\n+$/g, "");
      if (t) out.push({ kind: "text", text: t });
    }
    out.push({ kind: "code", text: m[1].replace(/\n$/, "") });
    last = fence.lastIndex;
  }
  if (last < body.length) {
    const t = body.slice(last).replace(/^\n+|\n+$/g, "");
    if (t) out.push({ kind: "text", text: t });
  }
  if (out.length === 0) out.push({ kind: "text", text: body });
  return out;
}

function nowClock(): string {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// content 블록 배열에서 text 추출 (ACP ContentBlock {type:"text",text} 위주, diff/resource 폴백).
function blocksToText(content: unknown): string {
  if (!Array.isArray(content)) return "";
  const parts: string[] = [];
  for (const b of content) {
    if (b && typeof b === "object") {
      const o = b as Record<string, unknown>;
      if (typeof o.text === "string") parts.push(o.text);
      else if (o.content && typeof o.content === "object") {
        const c = o.content as Record<string, unknown>;
        if (typeof c.text === "string") parts.push(c.text);
      }
    }
  }
  return parts.join("\n");
}

export function AcpConversation(props: {
  onClose: () => void;
  preset?: AcpPreset | null;
  // 선택: 우상단 pill 행 왼쪽에 끼워넣을 추가 토글(예: TalkTab 의 ⌗ 상태 패널 토글).
  // 헤더 행에 인라인 배치되어 스트리밍/ACP/닫기 pill 과 겹치지 않는다.
  headerExtra?: () => unknown;
  // 설정 시 헤더에 "⤢ 새 창" 버튼 노출 → 이 alias 의 대화만 별도 창(팝업)으로.
  // 팝업 자신은 이 prop 없이 렌더되어 버튼이 안 보임(중첩 방지).
  popoutAlias?: string | null;
  // 상세 패널의 "세션 재시작/닫기" — accessor 반환값이 증가하면 세션을 닫고 재구동.
  restartTrigger?: () => number;
  // 우측 정보 패널의 "📥 가져오기" — accessor 반환값이 증가하면 promptImport() 실행.
  // (가져오기 버튼은 컴포저에서 빠지고 우측 패널로 이동했지만, 가져온 내용을 'me' 버블 +
  //  acp_conv_add 로 현재 대화에 들이는 로직은 이 컴포넌트가 그대로 보유 — 트리거만 외부.)
  importTrigger?: () => number;
  // status line 데이터 — 폴더/역할/공개여부/연결 워크플로우 수(모델·토큰은 내부 값 사용).
  status?: () => { folder?: string | null; role?: string | null; isPublic?: boolean; workflows?: number };
}) {
  // 대화창만 별도 창으로. 절대 URL + 연 뒤 항상 명시적 이동(재사용된 빈/옛 창의 흰화면 방지).
  function openPopout(alias: string) {
    const url = `${location.origin}${location.pathname}?chat=${encodeURIComponent(alias)}`;
    const w = window.open("", `oxgchat_${alias}`, "width=480,height=820");
    if (!w) {
      location.href = url; // 팝업 차단 → 같은 창 fallback.
      return;
    }
    w.location.href = url; // 새 창이든 재사용 창이든 항상 올바른 URL 로드.
    w.focus();
  }
  const [agents, setAgents] = createSignal<AgentInfo[] | null>(null);
  const [agentsErr, setAgentsErr] = createSignal<string | null>(null);
  const [sessionId, setSessionId] = createSignal<string | null>(null);
  const [activeAgent, setActiveAgent] = createSignal<string | null>(null);
  const [spawnErr, setSpawnErr] = createSignal<string | null>(null);
  const [bubbles, setBubbles] = createSignal<Bubble[]>([]);
  const [draft, setDraft] = createSignal("");

  // ── 컴포저 칩: 권한 모드 / 모델 / thinking. 클릭 → 드롭다운 → ACP 세션 spawn 옵션에 반영. ──
  // 에이전트별 영속 설정(재부팅 유지)을 preset 에서 로드. 없으면 기본값.
  // 권한 기본값 = bypassPermissions (마스터 지시) — 에이전트가 기본으로 bash 등 실제 작업 수행.
  const [permMode, setPermMode] = createSignal(props.preset?.permMode || "bypassPermissions");
  const [model, setModel] = createSignal(props.preset?.model || "default");
  const [thinking, setThinking] = createSignal(props.preset?.thinking || "high");
  const [openMenu, setOpenMenu] = createSignal<"perm" | "model" | "think" | null>(null);

  // ── 컴포저 좌측 버튼: @ 파일참조 / 슬래시명령 / 📎 첨부. ──
  const [leftMenu, setLeftMenu] = createSignal<"at" | "slash" | null>(null);
  const [fileList, setFileList] = createSignal<string[] | null>(null); // @ 파일 picker (cwd 상대경로)
  const [fileFilter, setFileFilter] = createSignal("");
  // 슬래시 명령 = 에이전트의 실제 availableCommands(ACP available_commands_update).
  // 하드코딩 아님 — 세션에서 동적으로 받음(deep-research, code-review, brainstorming 등).
  const [availCmds, setAvailCmds] = createSignal<{ name: string; description?: string }[]>([]);
  const [slashFilter, setSlashFilter] = createSignal("");

  const PERM_OPTS = [
    { v: "default", label: "Default (확인)" },
    { v: "plan", label: "Plan (읽기전용)" },
    { v: "acceptEdits", label: "Accept Edits" },
    { v: "bypassPermissions", label: "🛡 Bypass Permissions" },
  ];
  // 백엔드(daemon_gui_acp.rs)가 칩값→유효 모델 id 로 매핑: opus→claude-opus-4-8,
  // sonnet→claude-sonnet-4-6, haiku→claude-haiku-4-5. 직접 타이핑(잘못된 "claude-opus-4.8")
  // 대신 드롭다운에서 고르게 한다. default=어댑터 기본.
  const MODEL_OPTS = [
    { v: "default", label: "Default (recommended)" },
    { v: "opus", label: "Opus 4.8" },
    { v: "sonnet", label: "Sonnet 4.6" },
    { v: "haiku", label: "Haiku 4.5" },
  ];
  // 동적 모델 목록 — OpenRouter(/v1/gui/models). 하드코딩 아님: claude-fable-5·codex/gpt 등 자동.
  const [orModels] = createResource<{ id: string; provider: string; name: string }[]>(async () => {
    try {
      const r = await invoke<{ models: { id: string; provider: string; name: string }[] }>("models_list");
      return r?.models ?? [];
    } catch {
      return [];
    }
  });
  const [modelFilter, setModelFilter] = createSignal("");
  const THINK_OPTS = [
    { v: "off", label: "Off (없음)" },
    { v: "low", label: "Low" },
    { v: "medium", label: "Medium" },
    { v: "high", label: "High" },
    { v: "ultra", label: "Ultra (Max)" },
  ];
  const labelOf = (opts: { v: string; label: string }[], v: string) =>
    opts.find((o) => o.v === v)?.label ?? v;

  // 모델별 컨텍스트 윈도우 + 대략 단가($/Mtok) — usage 표시(목업 `60k/1.00M (6%) · $..`)용.
  const MODEL_META: Record<string, { ctx: number; rate: number; name: string }> = {
    default: { ctx: 1000000, rate: 15, name: "Opus 4.8" }, // Opus 4.8 = 1M 컨텍스트
    haiku: { ctx: 200000, rate: 1, name: "Haiku 4.5" },
    sonnet: { ctx: 1000000, rate: 3, name: "Sonnet 4.6" },
    opus: { ctx: 1000000, rate: 15, name: "Opus 4.8" },
  };
  const fmtTok = (n: number) =>
    n >= 1_000_000 ? `${(n / 1_000_000).toFixed(2)}M` : n >= 1000 ? `${Math.round(n / 1000)}k` : `${n}`;
  // 어댑터가 표준 usage 를 안 주므로 대화 텍스트 누적 기반 토큰 추정(실데이터 기반·결정적).
  const estTokens = createMemo(() => {
    let chars = draft().length;
    for (const b of bubbles()) {
      if (b.kind === "agent") chars += b.segs.reduce((a, s) => a + s.text.length, 0);
      else if (b.kind === "me" || b.kind === "note") chars += b.text.length;
      else if (b.kind === "tool") chars += b.title.length;
      else if (b.kind === "plan") chars += b.entries.reduce((a, e) => a + e.content.length, 0);
    }
    return Math.round(chars / 4);
  });
  // 어댑터 usage_update({size,used}) 실데이터 — 있으면 추정보다 우선.
  const [usedTok, setUsedTok] = createSignal(0);
  const [ctxSize, setCtxSize] = createSignal(0);
  const usageLabel = createMemo(() => {
    const m = MODEL_META[model()] ?? MODEL_META.default;
    const used = usedTok() > 0 ? usedTok() : estTokens();
    const ctx = ctxSize() > 0 ? ctxSize() : m.ctx;
    const pct = Math.min(100, Math.round((used / ctx) * 100));
    const cost = (used / 1_000_000) * m.rate;
    return `${fmtTok(used)}/${fmtTok(ctx)} (${pct}%) · $${cost.toFixed(4)}`;
  });
  // 프리셋이면 이름, 커스텀 모델 id 면 그 id 를 그대로 표시.
  const modelName = createMemo(() => MODEL_META[model()]?.name ?? model());

  // 파일·문서 첨부 — 실제 경로로 저장 후 절대경로를 프롬프트에 삽입.
  // (이전엔 attachment://<hash> 텍스트만 넣어 에이전트가 파일을 못 읽었음. 에이전트는 URI 해석 불가 →
  //  서버 <data_dir>/drops/ 에 저장하고 절대경로를 넣어, 같은 머신 에이전트가 그 경로로 직접 읽게 함.)
  // 클릭(📎/@) + 드래그앤드롭 공용.
  const [dragOver, setDragOver] = createSignal(false);
  function uploadFile(f: File) {
    const reader = new FileReader();
    reader.onload = async () => {
      const b64 = (reader.result as string).split(",")[1] ?? "";
      try {
        const res = await invoke<{ path?: string }>(
          "session_dropfile",
          { identifier: convKey(), filename: f.name, content_b64: b64, machine: props.preset?.machine ?? null },
        );
        const ref = res?.path ? `📎 ${f.name} → ${res.path}` : "⚠ 첨부 경로 없음";
        setDraft(draft() ? `${draft()}\n${ref}` : ref);
      } catch (e) {
        setDraft(`${draft()}\n⚠ 첨부 실패: ${e instanceof Error ? e.message : String(e)}`);
      }
    };
    reader.readAsDataURL(f);
  }
  function attachFile() {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.onchange = (ev: Event) => {
      const fs = (ev.target as HTMLInputElement)?.files;
      if (fs) for (const f of Array.from(fs)) uploadFile(f);
    };
    input.click();
  }
  function onDrop(e: DragEvent) {
    e.preventDefault();
    setDragOver(false);
    const fs = e.dataTransfer?.files;
    if (fs && fs.length) for (const f of Array.from(fs)) uploadFile(f);
  }

  // ── @ 파일참조 picker — 세션 cwd 의 fs/tree 를 평탄화해 상대경로 목록 제공. ──
  function currentCwd(): string {
    return (lastSpawn?.opts?.cwd as string | undefined) || DEFAULT_ACP_CWD;
  }
  function flattenFiles(node: unknown, cwd: string, out: string[]) {
    if (!node || typeof node !== "object") return;
    const n = node as Record<string, unknown>;
    if (n.is_dir === false && typeof n.path === "string") {
      const rel = n.path.startsWith(cwd) ? n.path.slice(cwd.length).replace(/^\//, "") : (n.path as string);
      out.push(rel);
    }
    if (Array.isArray(n.children)) for (const c of n.children) flattenFiles(c, cwd, out);
  }
  async function toggleAt() {
    if (leftMenu() === "at") {
      setLeftMenu(null);
      return;
    }
    setLeftMenu("at");
    if (fileList() == null) {
      try {
        const cwd = currentCwd();
        const tree = await invoke<unknown>("fs_tree", { path: cwd, depth: 3 });
        const out: string[] = [];
        flattenFiles(tree, cwd, out);
        setFileList(out.sort().slice(0, 500));
      } catch {
        setFileList([]);
      }
    }
  }
  function insertAt(rel: string) {
    const d = draft();
    setDraft(d ? `${d.replace(/\s*$/, "")} @${rel} ` : `@${rel} `);
    setLeftMenu(null);
    setFileFilter("");
  }
  // 실제 슬래시 명령 삽입 — `/name `(에이전트가 전송 시 실행). 빈 draft 면 그대로.
  function insertSlash(name: string) {
    const cmd = `/${name} `;
    const d = draft();
    // 입력이 "/부분명령" 형태(셀렉터를 타이핑으로 띄운 경우)면 그 부분을 완성 명령으로 치환,
    // 아니면 끝에 추가(버튼으로 연 경우).
    if (/^\/\S*$/.test(d)) setDraft(cmd);
    else setDraft(d && !d.endsWith(" ") ? `${d} ${cmd}` : `${d}${cmd}`);
    setLeftMenu(null);
    setSlashFilter("");
  }
  const [busy, setBusy] = createSignal(false); // 세션 생성/프롬프트 진행 중
  const [streaming, setStreaming] = createSignal(false);
  // 재연결한 SSE 로 진행 중 턴의 청크가 들어오면 true → '입력중' 표시. 다른 창 갔다 와서
  // busy 가 false(새 마운트)여도 서버 턴이 살아있으면 이걸로 입력중을 보여준다. conv_persisted 에 해제.
  const [recvActive, setRecvActive] = createSignal(false);
  // 응답 중(busy)에 입력한 후속 메시지 대기열 — 현재 턴 종료 시 순서대로 자동 전송.
  const [queue, setQueue] = createSignal<string[]>([]);
  // 런타임(하네스) 설정 — 이 에이전트별(alias) 설정, 없으면 전역 기본값(백엔드 폴백).
  const [rtCfg] = createResource(() =>
    invoke<any>("runtime_config_get", props.preset?.label ? { alias: props.preset.label } : {})
      .then((r) => r?.config).catch(() => null));
  let memInjected = false; // 세션당 1회 OpenXgram 메모리 주입(첫 프롬프트).

  let nextId = 1;
  // 마지막 spawn 인자 — 칩 변경 시 같은 에이전트로 새 옵션 재구동(대화 보존)에 사용.
  let lastSpawn: { agent: string; opts?: SpawnArgs } | null = null;
  // true resume: 복원/재구동 후 첫 프롬프트에 1회 주입할 이전 대화 맥락(전송 텍스트에만, UI/DB엔 미표시).
  let pendingContext: string | null = null;
  let stopStream: (() => void) | null = null;

  // 에이전트별 컴포저 설정을 백엔드(agent_profiles)에 영속 — 새로고침·재부팅 후에도 유지.
  // preset.label = alias. 다음 대화 열 때 acpPreset 이 이 값을 다시 로드한다.
  function persistComposer() {
    const alias = props.preset?.label;
    if (!alias) return;
    void invoke("agent_composer_set", {
      alias, perm_mode: permMode(), model: model(), thinking: thinking(),
    }).catch(() => {});
  }

  // 칩 선택 → 신호 갱신 + 메뉴 닫기. 세션이 이미 있으면 새 옵션으로 재구동(내역 보존).
  function selectChip(kind: "perm" | "model" | "think", v: string) {
    if (kind === "perm") setPermMode(v);
    else if (kind === "model") {
      // 직접 입력 — 모델 id(예: claude-fable-5) 받아서 사용. 새 모델 코드수정 불필요.
      if (v === "__custom__") {
        const cur = MODEL_META[model()] ? "" : model();
        const id = window.prompt("모델 id 입력 (예: claude-opus-4-8, claude-fable-5)", cur);
        setOpenMenu(null);
        if (!id || !id.trim()) return;
        setModel(id.trim());
        persistComposer();
        if (sessionId() && lastSpawn) {
          pushBubble({ id: nextId++, kind: "note", text: `· 모델 변경(${id.trim()}) → 세션 재구동`, time: nowClock() });
          void spawn(lastSpawn.agent, { ...(lastSpawn.opts ?? {}), keepHistory: true });
        }
        return;
      }
      setModel(v);
    } else setThinking(v);
    setOpenMenu(null);
    persistComposer();
    if (sessionId() && lastSpawn) {
      const what = kind === "perm" ? "권한 모드" : kind === "model" ? "모델" : "thinking";
      pushBubble({ id: nextId++, kind: "note", text: `· ${what} 변경 → 세션 재구동`, time: nowClock() });
      void spawn(lastSpawn.agent, { ...(lastSpawn.opts ?? {}), keepHistory: true });
    }
  }
  // 현재 진행 중인 에이전트 turn 버블 id (chunk 누적용) + tool_call id→bubble id 매핑.
  let curAgentBubbleId: number | null = null;
  const toolBubbleByCall = new Map<string, number>();

  let msgsRef: HTMLDivElement | undefined;
  function scrollDown() {
    queueMicrotask(() => {
      if (msgsRef) msgsRef.scrollTop = msgsRef.scrollHeight;
    });
  }

  // 설치된 ACP 에이전트 목록 로드. 반환값으로 preset 자동 구동 시 설치 여부 판정.
  async function loadAgents(): Promise<AgentInfo[]> {
    setAgentsErr(null);
    try {
      const r = await acpFetch<{ agents: AgentInfo[] }>("GET", "/agents");
      const list = r.agents ?? [];
      setAgents(list);
      return list;
    } catch (e) {
      setAgentsErr((e as Error)?.message ?? String(e));
      setAgents([]);
      return [];
    }
  }

  // preset(특정 에이전트로 진입)이면: 어댑터 목록을 받아 설치 여부 확인 후 자동 spawn.
  // 미설치면 spawnErr 로 명확히 안내(에이전트 선택 화면 fallback 에 표시됨).
  // preset 이 없으면(picker 경로) 단순히 목록만 로드.
  async function bootForPreset(p: AcpPreset) {
    const list = await loadAgents();
    const found = list.find((a) => a.name === p.adapter);
    if (found && !found.installed) {
      setSpawnErr(`이 에이전트의 ACP 어댑터(${p.adapter}) 미설치 — 어댑터를 설치한 뒤 다시 시도하세요.`);
      return;
    }
    // 목록에 없어도(probe 누락) 구동을 시도 — 실제 미설치면 spawn 단계에서 오류가 노출됨.
    await spawn(p.adapter, { cwd: p.cwd, execMode: p.execMode, label: p.label });
  }

  if (props.preset) {
    void bootForPreset(props.preset);
  } else {
    loadAgents();
  }

  // 답변 하단 ⧉복사 — 클립보드에 평문 복사. 실패(권한/비보안 컨텍스트)는 조용히 무시.
  function copyText(t: string) {
    if (!t) return;
    void navigator.clipboard?.writeText(t).catch(() => {});
  }

  // 연속된 과정 버블(tool/plan = 툴 호출·계획)을 "▸ 단계 N" 아코디언 그룹으로 묶는다(기본 접힘).
  // me/agent/note 는 그대로. 최종 답변은 평문, 과정은 접혀서 클릭 시 펼침.
  // ▸단계 아코디언 펼침 상태 — group id 별 Set. 시그널로 제어해 재렌더(SSE 스트림·conv_persisted
  // 재동기화 등)로 <details> DOM 이 재생성돼도 펼침이 유지된다(클릭하면 바로 닫히던 버그 fix).
  const [expandedSteps, setExpandedSteps] = createSignal<Set<number>>(new Set());
  type StepsGroup = { kind: "steps"; id: number; items: Bubble[] };
  const displayItems = createMemo<(Bubble | StepsGroup)[]>(() => {
    const out: (Bubble | StepsGroup)[] = [];
    let group: Bubble[] = [];
    const flush = () => {
      if (group.length) { out.push({ kind: "steps", id: group[0].id, items: group }); group = []; }
    };
    for (const b of bubbles()) {
      if (b.kind === "tool" || b.kind === "plan") group.push(b);
      else { flush(); out.push(b); }
    }
    flush();
    return out;
  });

  function pushBubble(b: Bubble) {
    setBubbles((prev) => [...prev, b]);
    scrollDown();
  }

  // ── 아티팩트 미리보기 패널 ──
  // 대화 중 등장한 파일·이미지 참조(첨부 `📎 name → /path`, 절대경로, 코드/텍스트/이미지 파일)를
  // 모아 우측 슬라이드 패널에 미리보기. 이미지=썸네일(클릭 확대), 그 외=다운로드 + 경로 표시.
  // 파일 바이트 서빙 라우트(daemon)가 없으므로 같은-머신 file:// 미리보기 + 경로 복사 위주.
  const [artOpen, setArtOpen] = createSignal(false);
  const [artZoom, setArtZoom] = createSignal<string | null>(null); // 확대 중인 이미지 src
  // 아티팩트별(=path 키) 인라인 뷰어/에디터 상태. 읽기·편집·저장은 fs_file_get/fs_file_put 사용.
  type ArtState = {
    open: boolean;          // 뷰어/에디터 펼침 여부
    edit: boolean;          // 편집 모드(textarea) 여부
    loading: boolean;       // fs_file_get 진행 중
    loaded: boolean;        // 한 번이라도 로드 성공
    content: string;        // 읽어온 원본 본문
    buffer: string;         // 편집 버퍼
    err: string | null;     // 읽기 에러 텍스트
    saving: boolean;        // fs_file_put 진행 중
    saveMsg: { ok: boolean; text: string } | null;
  };
  const emptyArtState = (): ArtState => ({ open: false, edit: false, loading: false, loaded: false, content: "", buffer: "", err: null, saving: false, saveMsg: null });
  const [artStates, setArtStates] = createSignal<Record<string, ArtState>>({});
  const artState = (path: string): ArtState => artStates()[path] ?? emptyArtState();
  function patchArtState(path: string, patch: Partial<ArtState>) {
    setArtStates((prev) => ({ ...prev, [path]: { ...(prev[path] ?? emptyArtState()), ...patch } }));
  }
  // 파일 본문 로드 (idempotent — 이미 로드됐으면 스킵). AgentsTab 편집 모달과 동일 호출 패턴.
  async function loadArtContent(path: string): Promise<string | null> {
    const st = artState(path);
    if (st.loaded) return st.content;
    patchArtState(path, { loading: true, err: null });
    try {
      const r = await invoke<{ content: string }>("fs_file_get", { path });
      const content = r.content ?? "";
      patchArtState(path, { loading: false, loaded: true, content, buffer: content });
      return content;
    } catch (e) {
      patchArtState(path, { loading: false, err: String((e as Error).message || e) });
      return null;
    }
  }
  // 읽기 토글 — 펼치면서(닫혀있었으면) 본문 로드.
  function toggleArtView(path: string) {
    const st = artState(path);
    if (st.open && !st.edit) { patchArtState(path, { open: false }); return; }
    patchArtState(path, { open: true, edit: false });
    if (!st.loaded) void loadArtContent(path);
  }
  // 편집 토글 — 펼치면서 본문 로드(버퍼 채움).
  function toggleArtEdit(path: string) {
    const st = artState(path);
    if (st.open && st.edit) { patchArtState(path, { open: false, edit: false }); return; }
    patchArtState(path, { open: true, edit: true, saveMsg: null });
    if (!st.loaded) void loadArtContent(path);
  }
  async function saveArt(path: string) {
    patchArtState(path, { saving: true, saveMsg: null });
    try {
      const r = await invoke<{ ok: boolean; path: string; bytes: number }>("fs_file_put", { path, content: artState(path).buffer });
      patchArtState(path, { saving: false, content: artState(path).buffer, saveMsg: { ok: true, text: `✅ 저장됨 · ${r.bytes ?? artState(path).buffer.length}B` } });
    } catch (e) {
      patchArtState(path, { saving: false, saveMsg: { ok: false, text: `❌ ${String((e as Error).message || e)}` } });
    }
  }
  // code/text/file 아티팩트 다운로드 — a.src 가 실 URL 이 아니면 fs_file_get 으로 본문 받아 Blob 다운로드.
  async function downloadArt(path: string, name: string) {
    const content = await loadArtContent(path);
    if (content == null) return; // 에러는 loadArtContent 가 artState.err 에 기록
    const blob = new Blob([content], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url; a.download = name;
    document.body.appendChild(a); a.click(); a.remove();
    URL.revokeObjectURL(url);
  }
  // a.src 가 브라우저가 직접 다운로드 가능한 URL 인지(http(s)/blob). file:// 는 fetch 다운로드 대상.
  const isUsableUrl = (src: string) => /^(https?:|blob:)/.test(src);
  type Artifact = { path: string; name: string; kind: "image" | "code" | "text" | "file"; src: string };
  const IMG_RE = /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif)$/i;
  const CODE_RE = /\.(ts|tsx|js|jsx|rs|py|go|java|c|cpp|h|hpp|json|toml|yaml|yml|sh|sql|html|css|md|txt|csv|xml)$/i;
  // 텍스트에서 파일 경로 후보 추출. `📎 name → /abs/path` 형태와 일반 절대/상대 경로 모두.
  function extractPaths(text: string): string[] {
    const out: string[] = [];
    // 첨부 형식: "📎 filename → /abs/path"
    const attRe = /📎[^→\n]*→\s*(\S+)/g;
    let m: RegExpExecArray | null;
    while ((m = attRe.exec(text)) !== null) out.push(m[1]);
    // 일반 경로(절대 /…, ~/…, 또는 디렉토리 포함 상대경로) + 알려진 확장자.
    const pathRe = /(?:^|[\s("'`])((?:\/|~\/|\.{1,2}\/)[^\s)"'`]+\.[A-Za-z0-9]{1,5})/g;
    while ((m = pathRe.exec(text)) !== null) out.push(m[1]);
    return out;
  }
  function bubbleText(b: Bubble): string {
    if (b.kind === "me" || b.kind === "note") return b.text;
    if (b.kind === "agent") return b.segs.map((s) => s.text).join("\n");
    if (b.kind === "tool") return b.title;
    return "";
  }
  const artifacts = createMemo<Artifact[]>(() => {
    const seen = new Set<string>();
    const out: Artifact[] = [];
    for (const b of bubbles()) {
      for (const raw of extractPaths(bubbleText(b))) {
        const path = raw.replace(/[.,;:]+$/, "");
        if (!path || seen.has(path)) continue;
        seen.add(path);
        const name = path.split("/").pop() || path;
        const kind: Artifact["kind"] = IMG_RE.test(path) ? "image" : CODE_RE.test(path) ? "code" : "file";
        // 같은-머신 미리보기: file:// 로 직접 참조(브라우저가 로컬 파일 접근 가능한 경우 썸네일).
        const src = /^https?:\/\//.test(path) ? path : `file://${path.startsWith("~") ? path : path}`;
        out.push({ path, name, kind, src });
      }
    }
    return out;
  });

  // session/update 한 건을 버블에 반영 (스트림 + prompt 응답 updates 공용).
  function applyUpdate(u: unknown) {
    if (!u || typeof u !== "object") return;
    const o = u as Record<string, unknown>;
    const tag = o.sessionUpdate as string | undefined;
    if (!tag) return;
    // 데몬이 턴 결과를 acp_messages 에 영속한 직후 보내는 마커. 권위 소스(DB)에서
    // 재동기화 → 다른 창 갔다 와도(loadHistory 1회성으로 놓쳤던) 완료 답변이 복원된다.
    if (tag === "conv_persisted") {
      setRecvActive(false); // 턴 완료(영속) → 입력중 해제
      void loadHistory(true);
      return;
    }
    // 진행 중 턴의 활동(메시지/사고/툴) 수신 = 입력중. 재연결한 SSE 로 와도 표시된다.
    if (tag === "agent_message_chunk" || tag === "agent_thought_chunk" || tag === "tool_call") setRecvActive(true);
    if (tag === "agent_message_chunk" || tag === "agent_thought_chunk") {
      const text = blocksToText([o.content]);
      if (!text) return;
      if (curAgentBubbleId == null) {
        const id = nextId++;
        curAgentBubbleId = id;
        pushBubble({ id, kind: "agent", segs: segmentText(text), time: nowClock() });
      } else {
        const id = curAgentBubbleId;
        setBubbles((prev) =>
          prev.map((b) => {
            if (b.id !== id || b.kind !== "agent") return b;
            const merged = b.segs
              .filter((s) => s.kind === "text")
              .map((s) => s.text)
              .join("");
            return { ...b, segs: segmentText(merged + text) };
          }),
        );
        scrollDown();
      }
    } else if (tag === "tool_call") {
      const callId = String(o.toolCallId ?? "");
      const id = nextId++;
      if (callId) toolBubbleByCall.set(callId, id);
      pushBubble({
        id,
        kind: "tool",
        toolId: callId,
        title: String(o.title ?? o.kind ?? "tool"),
        status: String(o.status ?? "pending"),
        time: nowClock(),
      });
      curAgentBubbleId = null; // tool 이후 새 에이전트 chunk 는 새 버블로.
    } else if (tag === "tool_call_update") {
      const callId = String(o.toolCallId ?? "");
      const bid = toolBubbleByCall.get(callId);
      if (bid != null && o.status) {
        setBubbles((prev) =>
          prev.map((b) => (b.id === bid && b.kind === "tool" ? { ...b, status: String(o.status) } : b)),
        );
      }
    } else if (tag === "plan") {
      const entries = Array.isArray(o.entries)
        ? (o.entries as Record<string, unknown>[]).map((e) => ({
            content: String(e.content ?? ""),
            status: String(e.status ?? ""),
          }))
        : [];
      pushBubble({ id: nextId++, kind: "plan", entries, time: nowClock() });
      curAgentBubbleId = null;
    } else if (tag === "usage_update") {
      // 어댑터 실제 토큰 usage → 컴포저 usage 표시(60k/200k (30%)) 실데이터.
      if (typeof o.used === "number") setUsedTok(o.used as number);
      if (typeof o.size === "number") setCtxSize(o.size as number);
    } else if (tag === "available_commands_update") {
      // 에이전트의 실제 슬래시 명령 → `/` 드롭다운에 노출(하드코딩 아님).
      const cmds = o.availableCommands;
      if (Array.isArray(cmds)) {
        setAvailCmds(
          cmds
            .map((c) => {
              const cc = c as Record<string, unknown>;
              return { name: String(cc.name ?? ""), description: cc.description ? String(cc.description) : undefined };
            })
            .filter((c) => c.name),
        );
      }
    } else if (tag === "user_message_chunk") {
      // 에이전트가 user 입력을 replay — 이미 .me 로 그렸으므로 무시.
    }
  }

  // ── 대화 영속화 — 에이전트 alias(preset.label) 기준 conv_key. 새로고침/재시작 후 복원. ──
  function convKey(): string {
    return props.preset?.label || props.preset?.adapter || "default";
  }
  async function recordMsg(role: "me" | "agent" | "note", text: string) {
    if (!text.trim()) return;
    try {
      await invoke("acp_conv_add", { key: convKey(), role, text });
    } catch {
      /* 영속화 실패는 대화 흐름을 막지 않음 */
    }
  }
  // resync=true: conv_persisted 알림에 의한 재동기화(턴 완료 후 DB 최신화). 이때는
  // "복원됨" note·pendingContext 주입을 건너뛴다(라이브 세션이라 맥락 재주입 불필요·중복 note 방지).
  async function loadHistory(resync = false): Promise<boolean> {
    try {
      const rows = await invoke<{ role: string; text: string; created_at?: string }[]>("acp_conv_list", { key: convKey() });
      if (!Array.isArray(rows) || rows.length === 0) return false;
      // created_at(RFC3339) → "M/D HH:MM". 복원된 메시지에 실제 보낸 시각 표시(언제 보냈는지 확인 가능).
      const fmtTs = (ca?: string): string => {
        if (!ca) return "";
        const d = new Date(ca);
        if (Number.isNaN(d.getTime())) return "";
        const now = new Date();
        const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
        return d.toDateString() === now.toDateString() ? hm : `${d.getMonth() + 1}/${d.getDate()} ${hm}`;
      };
      const restored: Bubble[] = rows.map((r) => {
        const t = fmtTs(r.created_at);
        if (r.role === "agent") return { id: nextId++, kind: "agent", segs: segmentText(r.text), time: t };
        if (r.role === "note") return { id: nextId++, kind: "note", text: r.text, time: t };
        // 과정 복원 — 데몬이 영속한 툴 호출(JSON {title,status})·계획(JSON entries) → ▸단계 아코디언.
        if (r.role === "tool") {
          let title = "tool";
          let status = "";
          try { const m = JSON.parse(r.text); title = m.title ?? title; status = m.status ?? status; } catch { /* 형식 어긋나면 기본값 */ }
          return { id: nextId++, kind: "tool", toolId: "", title, status, time: t };
        }
        if (r.role === "plan") {
          let entries: { content: string; status: string }[] = [];
          try { const p = JSON.parse(r.text); if (Array.isArray(p)) entries = p; } catch { /* */ }
          return { id: nextId++, kind: "plan", entries, time: t };
        }
        return { id: nextId++, kind: "me", text: r.text, time: t };
      });
      if (!resync) restored.push({ id: nextId++, kind: "note", text: "↑ 이전 대화 복원됨 — 이어서 대화하세요.", time: nowClock() });
      setBubbles(restored);
      // 맥락 재주입은 이제 데몬이 권위있게 담당한다(daemon_gui_acp prompt: 서브프로세스 새로 spawn 시
      // DB 기록을 첫 프롬프트에 prepend). UI 의존 pendingContext 는 비활성 — 이중 주입·UI 마운트
      // 타이밍에 의존하던 불안정 제거. (재시작·크래시·on_demand 재spawn 어떤 경우에도 데몬이 복원.)
      scrollDown();
      return true;
    } catch {
      return false;
    }
  }

  // 복원/재구동된 세션의 첫 프롬프트에 주입할 이전 대화 맥락. 최근 ~12k자만(토큰 보호).
  function buildContextPreamble(): string {
    const lines: string[] = [];
    for (const b of bubbles()) {
      if (b.kind === "me") lines.push(`사용자: ${b.text}`);
      else if (b.kind === "agent")
        lines.push(`너(에이전트): ${b.segs.filter((s) => s.kind === "text").map((s) => s.text).join("")}`);
    }
    if (lines.length === 0) return "";
    let body = lines.join("\n");
    if (body.length > 12000) body = "…(이전 일부 생략)\n" + body.slice(body.length - 12000);
    return `[이전 대화 맥락 — 데몬 재시작/새로고침 후 이어서 진행. 아래는 우리의 지난 대화다.]\n${body}\n[위 맥락을 기억하고 아래 현재 요청에 답하라]\n`;
  }

  // 옛 "이어가기" 클립보드 export/import(exportConversation/importConversationFromText/
  // promptImport) 제거됨 — 대화 핸드오프는 ⇢ 위임 모달(submitDelegate)로 대체.

  // 에이전트 선택 → 세션 생성 + SSE 구독.
  // cwd 생략 시 DEFAULT_ACP_CWD, execMode 생략 시 on_demand. label 은 헤더 표시명(생략 시 adapter).
  async function spawn(agent: string, opts?: SpawnArgs) {
    if (busy()) return;
    setBusy(true);
    setSpawnErr(null);
    lastSpawn = { agent, opts };
    try {
      const body: Record<string, unknown> = {
        agent,
        cwd: opts?.cwd ?? DEFAULT_ACP_CWD,
        // CreateSessionBody 는 rename 없음(snake_case 필드 그대로) — execution_mode/permission_mode 키로 전송.
        execution_mode: opts?.execMode || "on_demand",
        permission_mode: permMode(),
        model: model(),
        thinking: thinking(),
        machine: props.preset?.machine ?? null, // 원격이면 데몬이 SSH 로 그 머신에서 spawn.
        // 대화 신원(에이전트 alias) — 서버 세션 재사용(find-or-create) 키. 전환 후 복귀 시 같은 세션 재연결.
        // picker 진입(preset 없음)이면 null → 재사용 안 함(서로 다른 picker 세션 병합 방지).
        label: props.preset?.label ?? null,
      };
      const r = await acpFetch<{ sessionId: string; agent: string; spawned: boolean; reused?: boolean }>(
        "POST",
        "/sessions",
        body,
      );
      setSessionId(r.sessionId);
      setActiveAgent(props.preset?.displayName || opts?.label || agent);
      if (!opts?.keepHistory) {
        memInjected = false; // 새 세션 → 메모리 재주입 허용.
        setBubbles([]);
        setUsedTok(0);
        setCtxSize(0);
        // reused=true 면 서버에 이미 살아있던 세션에 재연결한 것 — 전환 전 작업이 계속 돌고 있음.
        pushBubble({
          id: nextId++,
          kind: "note",
          text: r.reused
            ? `🔗 기존 세션 재연결 — ${agent} (백그라운드 작업 계속 진행 중)`
            : `⚡ ACP 세션 시작 — ${agent}${r.spawned ? " (구동됨)" : " (첫 프롬프트 시 구동)"}`,
          time: nowClock(),
        });
      }
      curAgentBubbleId = null;
      toolBubbleByCall.clear();
      // SSE 구독 시작 (prompt turn 중 발생한 update 가 relay 됨).
      stopStream?.();
      setStreaming(true);
      stopStream = acpStream(
        r.sessionId,
        (payload) => applyUpdate(payload),
        (msg) => {
          setStreaming(false);
          pushBubble({ id: nextId++, kind: "note", text: `⚠ 스트림: ${msg}`, time: nowClock() });
        },
      );
      // 영속화된 이전 대화 복원 (있으면 시작 note 를 대체). keepHistory(칩 재구동) 시엔 유지.
      if (!opts?.keepHistory) await loadHistory();
      // 칩 재구동도 새 어댑터 프로세스 → 기존 대화가 있으면 맥락 주입(true resume).
      else if (bubbles().some((b) => b.kind === "me" || b.kind === "agent"))
        pendingContext = buildContextPreamble();
    } catch (e) {
      setSpawnErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  async function sendPrompt() {
    const id = sessionId();
    const text = draft().trim();
    if (!id || !text || busy()) return;
    setBusy(true);
    setSpawnErr(null);
    pushBubble({ id: nextId++, kind: "me", text, time: nowClock() });
    void recordMsg("me", text); // 사용자 메시지 영속화(실제 입력만).
    setDraft("");
    curAgentBubbleId = null;
    // 런타임 메모리 주입(하네스) — inject_memory 면 세션 첫 프롬프트에 OpenXgram L2 메모리를
    // 앞에 붙인다(토큰예산=memory_count). 차별화: 에이전트가 OpenXgram 기억을 자기 맥락으로 씀.
    let memPreamble = "";
    const cfg = rtCfg();
    // 큐레이션된 주입 규칙(enabled, 해당 에이전트 scope + 전역) — 세션 첫 프롬프트에 맨 앞 주입.
    // inject_memory 와 무관하게 적용(규칙은 항상 우선). mandatory_note 와 중복 없게 별도 블록.
    let rulesPreamble = "";
    if (!memInjected) {
      try {
        const scopeArg = props.preset?.label ? { scope: props.preset.label } : { scope: "*" };
        const ir = await invoke<{ injections: any[] }>("runtime_injections_list", scopeArg);
        const rules: any[] = (ir?.injections ?? []).filter((r: any) => r.enabled && (r.content ?? "").trim());
        if (rules.length) {
          const body = rules.map((r: any) => `- ${r.name ? `${r.name}: ` : ""}${(r.content ?? "").trim()}`).join("\n");
          rulesPreamble = `[OpenXgram 주입 규칙 — 반드시 준수]\n${body}\n\n`;
          pushBubble({ id: nextId++, kind: "note", text: `📌 런타임: 주입 규칙 ${rules.length}개 주입`, time: nowClock() });
        }
      } catch { /* 주입 규칙 실패는 무시 */ }
    }
    if (cfg?.inject_memory && !memInjected) {
      memInjected = true;
      try {
        // 핀(개별 선택)이 있으면 후보 풀을 넉넉히 가져와 핀만 주입, 없으면 종류 필터 후 최근 N개.
        const pins: string[] = cfg.memory_pins ?? [];
        const wikiPins: string[] = cfg.wiki_pins ?? [];
        const fetchN = pins.length ? 50 : (cfg.memory_count ?? 8);
        const rc = await invoke<any>("runtime_context", { count: String(fetchN) });
        const kinds: string[] = cfg.memory_kinds ?? ["fact", "decision", "rule", "reference"];
        const all: any[] = rc?.memories ?? [];
        const sel = pins.length
          ? all.filter((m: any) => pins.includes(m.id))
          : all.filter((m: any) => kinds.includes(m.kind)).slice(0, cfg.memory_count ?? 8);
        const mems = sel.map((m: any) => `[${m.kind}] ${m.content}`).join("\n");
        if (mems) memPreamble += `[OpenXgram 기억 — 이 에이전트가 참고할 사실/결정/규칙]\n${mems}\n\n`;
        // 위키 — 선택(핀)된 것만 주입.
        if (cfg.inject_wiki && wikiPins.length && rc?.wiki?.length) {
          const w = rc.wiki.filter((x: any) => wikiPins.includes(x.id));
          if (w.length) memPreamble += `[OpenXgram 위키] ${w.map((x: any) => x.title).join(", ")}\n\n`;
        }
        // 필수 규칙(게이트, 호환) — 전송 전 반드시 맨 앞에 주입.
        if (cfg.mandatory_note?.trim()) memPreamble = `[필수 규칙 — 반드시 준수]\n${cfg.mandatory_note.trim()}\n\n${memPreamble}`;
        if (memPreamble) pushBubble({ id: nextId++, kind: "note", text: `🧠 런타임: 기억 ${sel.length}개${cfg.mandatory_note?.trim() ? " + 필수규칙" : ""} 주입`, time: nowClock() });
      } catch { /* 주입 실패는 무시 */ }
    }
    // 주입 규칙(큐레이션)은 항상 최우선 — 메모리/필수규칙 앞에 prepend.
    if (rulesPreamble) memPreamble = rulesPreamble + memPreamble;
    if (!memInjected) memInjected = true;
    // 주입 총량 상한(토큰 절감).
    const cap = cfg?.max_inject_chars ?? 6000;
    if (cap > 0 && memPreamble.length > cap) memPreamble = memPreamble.slice(0, cap) + "\n…(주입 상한)\n\n";
    // true resume: 복원/재구동 후 첫 프롬프트엔 이전 맥락을 앞에 붙여 전송(에이전트가 이어감).
    // UI 버블·DB 기록엔 사용자 실제 입력(text)만, 전송 텍스트(sendText)에만 맥락 포함.
    const sendText = memPreamble + (pendingContext ? `${pendingContext}현재 요청: ${text}` : text);
    if (pendingContext) {
      pushBubble({ id: nextId++, kind: "note", text: "↻ 이전 맥락을 에이전트에 전달하여 이어감", time: nowClock() });
      pendingContext = null;
    }
    try {
      // SSE 가 동일 update 를 먼저 relay 할 수 있으므로, prompt 응답의 updates 는
      // 스트림이 죽었을 때의 폴백으로만 적용. stopReason 은 note 로 표시.
      const r = await acpFetch<{ stopReason: string; updates?: unknown[] }>(
        "POST",
        `/sessions/${encodeURIComponent(id)}/prompt`,
        { text: sendText },
      );
      if (!streaming() && Array.isArray(r.updates)) {
        for (const u of r.updates) applyUpdate(u);
      }
      if (r.stopReason && r.stopReason !== "end_turn") {
        pushBubble({ id: nextId++, kind: "note", text: `· turn 종료: ${r.stopReason}`, time: nowClock() });
      }
    } catch (e) {
      pushBubble({ id: nextId++, kind: "note", text: `⚠ 구동 실패: ${(e as Error)?.message ?? e}`, time: nowClock() });
    } finally {
      setBusy(false);
      setRecvActive(false); // 내 턴 종료(에러 포함) → 입력중 해제(conv_persisted 못 오는 에러 턴 대비).
    }
    // ⚠ agent 응답 영속화는 이제 데몬이 권위있게 담당한다(daemon_gui.rs acp_session_prompt).
    // 종전엔 UI 가 turn-end 에 recordMsg("agent", ...) 로 기록했으나, 사용자가 턴 중/후
    // 대화창을 나가면 이 코드가 실행되지 않아 기록이 누락 → 돌아오면 "idle" 로 보이는
    // 핵심 버그였다. 데몬이 prompt 턴 종료 시 acp_messages 에 INSERT 하므로 UI 이탈과
    // 무관하게 영속화된다. 이중 기록 방지를 위해 UI 측 agent recordMsg 는 제거.
    // loadHistory(DB) 가 진실 원천 — 라이브 버블은 SSE/updates 의 낙관적 표시일 뿐.
    //
    // 데몬은 prompt HTTP 응답을 반환하기 전에 기록을 마치므로, 아래 acp_conv_read 시점엔
    // 이미 agent row 가 존재(created_at ≤ 턴 완료시각) → last_read ≥ created_at 보장,
    // "안읽음 1" 위양성 배지 없음.
    // 대화 중이므로 방금 받은 응답은 읽음 처리(안읽음 배지 누적 방지).
    void invoke("acp_conv_read", { key: convKey() }).catch(() => {});
    // 대기열에 후속 메시지가 있으면 턴 종료 후 순서대로 자동 전송.
    const q = queue();
    if (q.length > 0) {
      setQueue(q.slice(1));
      setDraft(q[0]);
      void sendPrompt();
    }
  }

  async function cancelTurn() {
    const id = sessionId();
    if (!id) return;
    // soft cancel(best-effort) — 어댑터는 턴마다 새 session/new 라 session/cancel 이 진행 중 턴에
    // 안 닿을 수 있다(그래서 '취소가 안 되던' 것). 따라서 확실히 멈추려면 세션을 강제 종료한다.
    try {
      await acpFetch("POST", `/sessions/${encodeURIComponent(id)}/cancel`);
    } catch {
      /* best-effort */
    }
    pushBubble({ id: nextId++, kind: "note", text: "■ 중단 — 현재 턴을 멈추고 세션을 재구동합니다.", time: nowClock() });
    setBusy(false);
    setRecvActive(false);
    // 확실한 중단: 세션 DELETE 로 서브프로세스를 죽여 진행 중 턴을 강제 종료 → 즉시 재구동.
    // 맥락은 DB 기록에서 매 프롬프트 재주입되므로 손실 없음.
    await closeSession();
  }

  // 세션 닫기 → DELETE + 스트림 중단. roster 로 복귀하지 않고 에이전트 선택 화면으로.
  // 세션 닫기 = 재시작. 현 ACP 세션(subprocess)을 종료하고 곧바로 재구동 → 대화창 복귀.
  // (이전 버그: sessionId=null 로만 두면 preset 이 남아 "구동 중…" 에서 멈춤.)
  async function closeSession() {
    const id = sessionId();
    stopStream?.();
    stopStream = null;
    setStreaming(false);
    if (id) {
      try {
        await acpFetch("DELETE", `/sessions/${encodeURIComponent(id)}`);
      } catch {
        // best-effort — 닫힘 실패해도 재구동 시도.
      }
    }
    setSessionId(null);
    // preset 진입(roster/팝업)이면 재구동 → 대화창 복귀(이전 대화 복원). 멈춤 방지.
    if (props.preset) {
      await bootForPreset(props.preset);
    } else {
      setActiveAgent(null);
      setBubbles([]);
    }
  }

  onCleanup(() => {
    // 에이전트 전환(컴포넌트 unmount) 시 UI 스트림만 detach — 세션은 서버에서 계속 실행한다.
    // (이전엔 여기서 DELETE 로 세션을 죽여서, 일 시켜놓고 다른 대화로 이동하면 작업이 멈췄음.
    //  명시적 '세션 닫기'(closeSession) 만 DELETE. 돌아오면 create_session 이 같은 세션 재연결.)
    stopStream?.();
  });

  // 상세 패널 "세션 재시작" 트리거 — 값이 증가하면 닫고 재구동(대화창 복귀).
  createEffect<number>((prev) => {
    const v = props.restartTrigger?.() ?? 0;
    if (prev !== undefined && v !== prev && v > 0) void closeSession();
    return v;
  });

  // 우측 패널 "📥 가져오기" 트리거 — 값이 증가하면 promptImport() 실행(붙여넣기 → me 버블 + 영속).
  createEffect<number>((prev) => {
    const v = props.importTrigger?.() ?? 0;
    if (prev !== undefined && v !== prev && v > 0) void promptImport();
    return v;
  });

  // ── 위임 모달 (신원=에이전트 셀렉터 + endpoint 셀렉터) ──
  // 주소 = alias(신원) + endpoint(전달 방식). a2a_send 가 endpoint 에 따라 라우팅한다.
  // 엔드포인트는 a2a_agent_endpoints 로 대상 alias 마다 동적 조회 → 사용 가능한 것만 노출.
  // existing_acp/tmux 가 여러 인스턴스면 2단 셀렉터(타입 → 인스턴스).
  type AgentEndpoints = {
    new_acp?: boolean;
    existing_acp?: { id: string; label?: string }[];
    tmux?: { name: string; cwd?: string }[];
    worktree?: boolean;
    external?: boolean;
  };
  const [delOpen, setDelOpen] = createSignal(false);
  const [delTarget, setDelTarget] = createSignal("");
  const [delEndpoint, setDelEndpoint] = createSignal("new_acp"); // 엔드포인트 타입
  const [delInstance, setDelInstance] = createSignal(""); // 2단 셀렉터: existing_acp id 또는 tmux name
  const [delEndpoints, setDelEndpoints] = createSignal<AgentEndpoints | null>(null); // 동적 조회 결과
  const [delEpLoading, setDelEpLoading] = createSignal(false);
  const [delBusy, setDelBusy] = createSignal(false);
  const [delAgents] = createResource<{ alias: string; reachable?: boolean }[]>(async () => {
    try {
      const r = await invoke<{ agents: { alias: string; reachable?: boolean }[] }>("a2a_agents");
      return r?.agents ?? [];
    } catch {
      return [];
    }
  });
  // 명부(agents_list, GET /v1/gui/agents) — alias 별 로컬/친구 분류용(project_path·classification·machine).
  // a2a_agents 는 이 필드를 주지 않으므로 로컬/친구 판정에 이 소스를 join 한다.
  const [delRoster] = createResource<
    { alias: string; project_path?: string | null; classification?: string | null; machine?: string | null }[]
  >(async () => {
    try {
      const r = await invoke<
        { alias: string; project_path?: string | null; classification?: string | null; machine?: string | null }[]
      >("agents_list");
      return Array.isArray(r) ? r : [];
    } catch {
      return [];
    }
  });

  // ── 로컬/친구 분류 (TalkTab.isLocalAgent 와 동일 기준) ──
  // 1폴더=1에이전트=1 alias(신원). 표시는 alias 기준 — 폴더 그룹핑 없음.
  // 로컬 = 이 머신(ACP): project_path 가 /home/llm 하위 OR machine 비었거나 현재 머신.
  // 친구 = 다른 머신/외부: classification=friend OR project_path 가 다른 머신 경로 OR machine 원격값.
  const LOCAL_HOME_PREFIX = "/home/llm";
  const SELF_MACHINE_NAMES = ["server-seoul", "local", "서울", "seoul"];
  function isLocalMachineField(machine?: string | null): boolean {
    const m = (machine ?? "").trim().toLowerCase();
    if (!m) return true; // 비었으면 로컬(기존 로컬 에이전트는 machine 미설정).
    return SELF_MACHINE_NAMES.some((n) => m === n || m.includes(n) || n.includes(m));
  }
  function isLocalRosterAgent(r: { project_path?: string | null; classification?: string | null; machine?: string | null }): boolean {
    if ((r.classification ?? "") === "friend") return false; // 명시 친구.
    const p = (r.project_path ?? "").trim();
    if (p && !(p === LOCAL_HOME_PREFIX || p.startsWith(LOCAL_HOME_PREFIX + "/")) && p.startsWith("/home/")) return false;
    if (!isLocalMachineField(r.machine)) return false;
    return true;
  }
  type DelAgent = { alias: string; reachable?: boolean; machine?: string | null };
  // 대상 후보 = a2a_agents 의 alias(호출 가능한 신원). 로컬/친구는 roster 의 분류로 판정.
  // roster 에 없는 alias 는 machine 미상 → 로컬로 간주(기존 로컬 에이전트 기본).
  const delLocalFriend = createMemo<{ local: DelAgent[]; friends: DelAgent[] }>(() => {
    const rosterMap = new Map<string, { project_path?: string | null; classification?: string | null; machine?: string | null }>();
    for (const r of delRoster() ?? []) rosterMap.set(r.alias, r);
    const local: DelAgent[] = [];
    const friends: DelAgent[] = [];
    for (const a of delAgents() ?? []) {
      const r = rosterMap.get(a.alias);
      const isLocal = r ? isLocalRosterAgent(r) : true;
      const row: DelAgent = { alias: a.alias, reachable: a.reachable, machine: r?.machine ?? null };
      (isLocal ? local : friends).push(row);
    }
    local.sort((a, b) => a.alias.localeCompare(b.alias));
    friends.sort((a, b) => a.alias.localeCompare(b.alias));
    return { local, friends };
  });
  // 선택된 대상이 친구인지 — 친구면 endpoint 는 A2A(external) 단순 흐름.
  function isTargetFriend(alias: string): boolean {
    return (delLocalFriend().friends ?? []).some((f) => f.alias === alias);
  }
  // 대상 alias 의 도달 엔드포인트 조회. 백엔드 라우트가 아직 없으면(빌드 전) graceful fallback.
  async function loadEndpoints(alias: string) {
    if (!alias) { setDelEndpoints(null); return; }
    setDelEpLoading(true);
    try {
      const r = await invoke<AgentEndpoints>("a2a_agent_endpoints", { alias });
      // 정상 응답 — new_acp/external 기본 true 보장(라우트 동작 시 항상 가능).
      setDelEndpoints({
        new_acp: r?.new_acp !== false,
        existing_acp: Array.isArray(r?.existing_acp) ? r.existing_acp : [],
        tmux: Array.isArray(r?.tmux) ? r.tmux : [],
        worktree: !!r?.worktree,
        external: r?.external !== false,
      });
    } catch {
      // 라우트 미존재/실패 → 기본 2종만(절대 깨지지 않게).
      setDelEndpoints({ new_acp: true, existing_acp: [], tmux: [], worktree: false, external: true });
    } finally {
      setDelEpLoading(false);
    }
  }
  // 대상 에이전트 선택 → 엔드포인트 재조회 + 첫 사용가능 타입으로 리셋.
  // 친구(다른 머신·외부)면 endpoint 는 A2A(external) 단순 흐름 — 그 친구/머신으로 라우팅.
  // 로컬이면 endpoint 셀렉터(신규 ACP / 기존 ACP / TMUX / 워크트리) 동적 조회.
  function onDelTargetChange(alias: string) {
    setDelTarget(alias);
    setDelInstance("");
    if (isTargetFriend(alias)) {
      setDelEndpoint("external");
      // 친구는 A2A 단독 — 로컬 endpoint 조회 불필요. external 만 노출되도록 리셋.
      setDelEndpoints({ new_acp: false, existing_acp: [], tmux: [], worktree: false, external: true });
    } else {
      setDelEndpoint("new_acp");
      void loadEndpoints(alias);
    }
  }
  // 현재 선택된 엔드포인트 타입에 인스턴스 리스트가 있으면 반환(2단 셀렉터 노출 판정).
  function endpointInstances(): { value: string; label: string }[] {
    const eps = delEndpoints();
    if (!eps) return [];
    if (delEndpoint() === "existing_acp") {
      return (eps.existing_acp ?? []).map((s) => ({ value: s.id, label: s.label || s.id }));
    }
    if (delEndpoint() === "tmux") {
      return (eps.tmux ?? []).map((t) => ({ value: t.name, label: t.cwd ? `${t.name} (${t.cwd})` : t.name }));
    }
    return [];
  }
  function openDelegate() {
    if (!draft().trim()) {
      window.alert("위임할 내용을 입력창에 먼저 입력하세요.");
      return;
    }
    setDelTarget("");
    setDelEndpoint("new_acp");
    setDelInstance("");
    setDelEndpoints(null);
    setDelOpen(true);
  }
  async function submitDelegate() {
    const tgt = delTarget().trim();
    const text = draft().trim();
    if (!tgt || !text) return;
    const epType = delEndpoint();
    const inst = delInstance().trim();
    // 인스턴스 선택이 필요한 타입인데 미선택이면 막는다.
    if ((epType === "existing_acp" || epType === "tmux") && !inst) {
      window.alert("전달할 인스턴스를 선택하세요.");
      return;
    }
    // a2a_send 의 endpoint 파라미터: "tmux:<name>", "existing_acp:<id>", 또는 단순 타입.
    const endpoint =
      epType === "tmux" ? `tmux:${inst}` :
      epType === "existing_acp" ? `existing_acp:${inst}` :
      epType; // new_acp | external | worktree
    setDelBusy(true);
    const from = convKey();
    try {
      const r = await invoke<{ result?: { text?: string } }>("a2a_send", {
        target: tgt, task: text, from_agent: from, endpoint,
      });
      const ans = r?.result?.text?.trim() || "(응답 텍스트 없음)";
      pushBubble({ id: nextId++, kind: "note", text: `⇢ ${tgt} [${endpoint}] 위임: "${text}"`, time: nowClock() });
      pushBubble({ id: nextId++, kind: "note", text: `⇠ ${tgt} 응답:\n${ans}`, time: nowClock() });
      setDraft("");
      setDelOpen(false);
    } catch (e) {
      window.alert(`위임 실패: ${(e as Error)?.message ?? e}`);
    } finally {
      setDelBusy(false);
    }
  }

  // 전송 — 응답 중(busy)이면 대기열에 적재(턴 종료 시 자동 전송), 아니면 즉시 전송.
  // 📥 가져오기(복원) — 다른 LLM/에이전트의 작업·대화를 붙여넣어 현재 대화에 맥락으로 들여온다.
  // 'me' 로 기록 → 데몬이 다음 프롬프트에 히스토리로 주입(build_resume_preamble)하므로 에이전트가 이어감.
  async function promptImport() {
    const raw = window.prompt("다른 LLM/에이전트에서 가져올 작업·대화 내용을 붙여넣으세요:");
    if (!raw || !raw.trim()) return;
    const body = `[다른 LLM/에이전트에서 가져온 이전 작업 맥락 — 이어서 진행한다]\n${raw.trim()}`;
    pushBubble({ id: nextId++, kind: "me", text: body, time: nowClock() });
    await invoke("acp_conv_add", { key: convKey(), role: "me", text: body }).catch(() => {});
    pushBubble({ id: nextId++, kind: "note", text: "📥 가져옴 — 이어서 대화하면 에이전트가 이 맥락을 받습니다.", time: nowClock() });
    scrollDown();
  }

  function submit() {
    const text = draft().trim();
    if (!text) return;
    // /clear — ACP 메신저엔 CLI 하니스가 없어 자동 처리 안 됨(텍스트로 떨어져 에이전트가 설명만 함).
    // 우리가 직접: 영속 기록 삭제 + UI 비우기 + ACP subprocess 재시작(새 session/new = 컨텍스트 초기화).
    if (text === "/clear") { setDraft(""); void clearConversation(); return; }
    // /model <id> — 모델 변경(어댑터엔 안 닿는 CLI 빌트인이라 우리가 가로채 칩으로 처리 + 세션 재구동).
    if (text.startsWith("/model ")) {
      const mid = text.slice(7).trim();
      setDraft("");
      if (mid) selectChip("model", mid);
      return;
    }
    if (busy()) {
      setQueue([...queue(), text]);
      setDraft("");
      return;
    }
    void sendPrompt();
  }

  async function clearConversation() {
    await invoke("acp_conv_clear", { key: convKey() }).catch(() => {});
    setBubbles([]);
    setUsedTok(0);
    setCtxSize(0);
    pendingContext = null;
    pushBubble({ id: nextId++, kind: "note", text: "🧹 /clear — 대화·컨텍스트 초기화 (세션 재시작)", time: nowClock() });
    // ACP subprocess 재시작 = 새 session/new = 깨끗한 컨텍스트. DB 를 비웠으니 복원도 빈 상태.
    if (lastSpawn) {
      await spawn(lastSpawn.agent, { ...(lastSpawn.opts ?? {}), keepHistory: false });
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  createEffect(() => {
    bubbles();
    scrollDown();
  });

  return (
    <Show
      when={sessionId()}
      fallback={
        props.preset ? (
          // ── preset 진입(roster 선택) — picker 없이 구동/오류 상태만 표시 ──
          <div class="kk-talk-chat">
            <div class="chat-top">
              <span class="back" onClick={() => props.onClose()}>←</span>
              <div class="ava c-claude">⚡</div>
              <div class="nm">{props.preset.displayName || props.preset.label || props.preset.adapter}</div>
              <div class="meta-r">
                <span class="pill">⚡ ACP · {props.preset.adapter}</span>
              </div>
            </div>
            <div class="msgs">
              <Show when={agentsErr()}>
                <div class="kk-talk-err">⚠ 어댑터 목록 실패: {agentsErr()}</div>
              </Show>
              <Show when={spawnErr()}>
                <div class="kk-talk-err">⚠ {spawnErr()}</div>
              </Show>
              <Show when={!spawnErr() && !agentsErr()}>
                <div class="kk-talk-empty">⚡ ACP 세션 구동 중…</div>
              </Show>
            </div>
          </div>
        ) : (
        // ── 에이전트 선택 화면 (세션 미생성, picker 경로) ──
        <div class="kk-talk-chat">
          <div class="chat-top">
            <span class="back" onClick={() => props.onClose()}>←</span>
            <div class="ava c-claude">⚡</div>
            <div class="nm">ACP 에이전트</div>
            <div class="meta-r">
              {/* 옛 클립보드 📤이어가기/📥가져오기 제거 — 대화 핸드오프는 ⇢ 위임 모달로 대체됨. */}
              <span class="pill">로컬 subprocess</span>
            </div>
          </div>
          <div class="msgs">
            <div class="kk-acp-pick">
              <div class="kk-acp-pick-h">구동할 ACP 에이전트를 선택하세요</div>
              <Show when={agentsErr()}>
                <div class="kk-talk-empty">⚠ 에이전트 목록 실패: {agentsErr()}</div>
              </Show>
              <Show when={agents() == null && !agentsErr()}>
                <div class="kk-talk-empty">불러오는 중…</div>
              </Show>
              <Show when={agents() && (agents() as AgentInfo[]).length === 0 && !agentsErr()}>
                <div class="kk-talk-empty">등록된 ACP 어댑터가 없습니다.</div>
              </Show>
              <For each={agents() ?? []}>
                {(ag) => (
                  <div
                    class={`kk-acp-agent${ag.installed ? "" : " off"}`}
                    onClick={() => ag.installed && spawn(ag.name)}
                  >
                    <div class="av c-claude">⚡</div>
                    <div class="kk-acp-agent-meta">
                      <div class="kk-acp-agent-nm">{ag.name}</div>
                      <div class="kk-acp-agent-st">
                        {ag.installed ? "설치됨 · 클릭하여 세션 시작" : "ACP 에이전트 미설치"}
                      </div>
                    </div>
                    <Show when={!ag.installed}><span class="kk-acp-badge">미설치</span></Show>
                  </div>
                )}
              </For>
              <Show when={spawnErr()}>
                <div class="kk-talk-err">⚠ 세션 생성 실패: {spawnErr()}</div>
              </Show>
              <Show when={busy()}><div class="kk-talk-empty">세션 생성 중…</div></Show>
            </div>
          </div>
        </div>
        )
      }
    >
      {/* ── ACP 대화방 (세션 활성) — peer 대화와 동일 마크업 ── */}
      <div
        class={`kk-talk-chat${dragOver() ? " dragover" : ""}`}
        onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
        onDragLeave={() => setDragOver(false)}
        onDrop={onDrop}
      >
        <Show when={dragOver()}>
          <div class="kk-drop-hint">📎 파일을 여기에 놓아 첨부</div>
        </Show>
        <div class="chat-top">
          <span class="back" onClick={() => props.onClose()}>←</span>
          <div class="ava c-claude">⚡</div>
          <div class="nm">{props.preset?.displayName || activeAgent()}</div>
          <div class="meta-r">
            <Show when={props.headerExtra}>{props.headerExtra!() as never}</Show>
            <Show when={streaming()} fallback={<span class="pill off"><span class="pdot" />스트림 끊김</span>}>
              <span class="pill"><span class="pdot" />스트리밍</span>
            </Show>
            <span class="pill">⚡ ACP</span>
            {/* 옛 클립보드 📤이어가기/📥가져오기 버튼 제거 — 대화 핸드오프는 ⇢ 위임 모달로 대체됨. */}
            {/* 📎 아티팩트 패널 토글 — 대화 중 등장한 파일·이미지 미리보기. */}
            <Show when={artifacts().length > 0}>
              <span
                class="kk-acp-pop"
                title="대화 중 등장한 파일·이미지 미리보기"
                onClick={() => setArtOpen((v) => !v)}
              >📎 아티팩트 {artifacts().length}</span>
            </Show>
            <Show when={props.popoutAlias}>
              <span class="kk-acp-pop" title="새 창으로 열기" onClick={() => openPopout(props.popoutAlias!)}>⤢ 새 창</span>
            </Show>
            {/* 세션 닫기(=재시작)는 헤더에 노출하지 않음 — 상세 패널에서 제어(props.restartTrigger). */}
          </div>
        </div>

        <div class="msgs" ref={msgsRef} style="position:relative;">
          <For each={displayItems()}>
            {(b) =>
              b.kind === "steps" ? (
                <details
                  class="kk-acp-steps"
                  style="margin:3px 0;"
                  open={expandedSteps().has(b.id)}
                  onToggle={(e) => {
                    const isOpen = e.currentTarget.open;
                    setExpandedSteps((prev) => {
                      const n = new Set(prev);
                      if (isOpen) n.add(b.id);
                      else n.delete(b.id);
                      return n;
                    });
                  }}
                >
                  <summary style="cursor:pointer;color:#8b95a1;font-size:12px;user-select:none;">▸ 단계 (툴·계획 {b.items.length})</summary>
                  <div style="border-left:2px solid #2a2f3a;margin:3px 0 3px 5px;padding-left:8px;">
                    <For each={b.items}>
                      {(it) =>
                        it.kind === "tool" ? (
                          <div class={`toolcall${it.status === "failed" ? " fail" : ""}`} style="font-size:12px;margin:3px 0;">
                            <span class={it.status === "failed" ? "no" : "ok"}>{it.status === "failed" ? "✗" : "✓"}</span>{" "}
                            <span class="cmd">{it.title}</span> <span class="kk-acp-tstat">{it.status}</span>
                          </div>
                        ) : it.kind === "plan" ? (
                          <div class="kk-acp-plan">
                            <div class="kk-acp-plan-h">계획</div>
                            <For each={it.entries}>
                              {(e) => (
                                <div class={`kk-acp-plan-item st-${e.status}`}>
                                  <span class="kk-acp-plan-dot" /> {e.content}
                                </div>
                              )}
                            </For>
                          </div>
                        ) : null
                      }
                    </For>
                  </div>
                </details>
              ) : b.kind === "me" ? (
                <div class="me">
                  <div class="mr"><div class="tm">{b.time}</div></div>
                  <div class="b">{b.text}</div>
                </div>
              ) : b.kind === "note" ? (
                <div class="day">{b.text}</div>
              ) : b.kind === "tool" ? (
                <div class="agent">
                  <div class="body">
                    <div class={`toolcall${b.status === "failed" ? " fail" : ""}`}>
                      <span class={b.status === "failed" ? "no" : "ok"}>{b.status === "failed" ? "✗" : "✓"}</span>{" "}
                      <span class="cmd">{b.title}</span>
                      <span class="kk-acp-tstat">{b.status}</span>
                    </div>
                  </div>
                </div>
              ) : b.kind === "plan" ? (
                <div class="agent">
                  <div class="body">
                    <div class="kk-acp-plan">
                      <div class="kk-acp-plan-h">계획</div>
                      <For each={b.entries}>
                        {(e) => (
                          <div class={`kk-acp-plan-item st-${e.status}`}>
                            <span class="kk-acp-plan-dot" /> {e.content}
                          </div>
                        )}
                      </For>
                    </div>
                  </div>
                </div>
              ) : (
                <div class="agent kk-acp-answer">
                  <div class="head">
                    <div class="av c-claude">⚡</div>
                    <div class="nm">{props.preset?.displayName || activeAgent()}</div>
                  </div>
                  {/* 답변은 말풍선 X — 평문 전체폭(배경·테두리 없음). 사용자 메시지만 말풍선. */}
                  <div class="body" style="background:none;border:none;box-shadow:none;padding:2px 0;max-width:100%;">
                    <For each={b.segs}>
                      {(seg) =>
                        seg.kind === "code" ? <pre class="code">{seg.text}</pre> : <p>{seg.text}</p>
                      }
                    </For>
                  </div>
                  {/* 카카오톡식 하단 푸터 — 복사 버튼 + 받은 시각. */}
                  <div class="kk-acp-foot" style="display:flex;align-items:center;gap:6px;margin-top:3px;font-size:11px;color:#8b95a1;">
                    <button
                      class="kk-acp-copy"
                      title="이 답변 복사"
                      style="border:none;background:transparent;cursor:pointer;color:#8b95a1;font-size:12px;padding:0 2px;"
                      onClick={() => copyText(b.segs.filter((s) => s.kind === "text").map((s) => s.text).join(""))}
                    >⧉ 복사</button>
                    <span class="tm">{b.time}</span>
                  </div>
                </div>
              )
            }
          </For>
          <Show when={bubbles().length === 0}>
            <div class="kk-talk-empty">세션 준비됨. 아래에서 첫 프롬프트를 보내세요.</div>
          </Show>
          {/* 입력중 표시 — 내가 보낸 턴(busy) + 다른 창서 진행 중인 턴을 reconnect SSE 로 감지(recvActive).
              streaming 은 세션 살아있는 내내 true 라 안 씀(유휴에도 떠버림). */}
          <Show when={busy() || recvActive()}>
            <div class="agent kk-acp-typing">
              <div class="head"><span class="nm">⚡ 에이전트</span></div>
              <div class="body"><span class="kk-typing"><i /><i /><i /></span> 입력중…</div>
            </div>
          </Show>
          {/* ⇢ 위임 모달 — 신원(에이전트) + 엔드포인트(전달 방식) 셀렉터. */}
          <Show when={delOpen()}>
            <div
              style="position:fixed;inset:0;background:rgba(0,0,0,.4);z-index:1000;display:flex;align-items:center;justify-content:center;"
              onClick={() => setDelOpen(false)}
            >
              <div style="background:#fff;border-radius:12px;padding:18px;width:min(440px,92vw);color:#222;" onClick={(e) => e.stopPropagation()}>
                <div style="font-weight:700;margin-bottom:12px;">⇢ 에이전트에게 위임</div>
                <div style="font-size:12px;color:#8b95a1;margin-bottom:3px;">대상 에이전트 (대화명)</div>
                {/* 에이전트를 alias(대화명) 기준으로 직접 나열 — 폴더 그룹핑 없음.
                    🏠 로컬(이 머신·ACP) / 🖥 친구(다른 머신·외부) 2 구분 헤더로만 분리. */}
                <select value={delTarget()} onInput={(e) => onDelTargetChange(e.currentTarget.value)} style="width:100%;padding:7px;border:1px solid #d5dae2;border-radius:8px;margin-bottom:11px;">
                  <option value="">— 선택 —</option>
                  <Show when={delLocalFriend().local.length > 0}>
                    <optgroup label="🏠 로컬 에이전트 (이 머신)">
                      <For each={delLocalFriend().local}>
                        {(a) => <option value={a.alias}>{a.alias}{a.reachable === false ? " (미도달)" : ""}</option>}
                      </For>
                    </optgroup>
                  </Show>
                  <Show when={delLocalFriend().friends.length > 0}>
                    <optgroup label="🖥 친구 (다른 머신·외부)">
                      <For each={delLocalFriend().friends}>
                        {(a) => <option value={a.alias} title={a.machine ?? ""}>{a.alias}{a.machine ? ` · 🖥 ${a.machine}` : ""}{a.reachable === false ? " (미도달)" : ""}</option>}
                      </For>
                    </optgroup>
                  </Show>
                </select>
                {/* 친구(다른 머신·외부) — endpoint 는 A2A 고정. 풍부한 endpoint 셀렉터는 로컬 전용. */}
                <Show
                  when={!isTargetFriend(delTarget())}
                  fallback={
                    <Show when={delTarget()}>
                      <div style="font-size:12px;color:#8b95a1;margin-bottom:3px;">전달 방식</div>
                      <div style="font-size:13px;border:1px solid #d5dae2;border-radius:8px;padding:7px 9px;margin-bottom:11px;background:#f6f8fc;color:#3a4a6a;">
                        🖥 친구 — A2A 로 전달 (그 친구/머신으로 라우팅)
                      </div>
                    </Show>
                  }
                >
                <div style="font-size:12px;color:#8b95a1;margin-bottom:3px;">전달 방식 (엔드포인트)</div>
                <Show when={delTarget() && delEpLoading()}>
                  <div style="font-size:12px;color:#8b95a1;margin-bottom:11px;">엔드포인트 조회 중…</div>
                </Show>
                {/* 엔드포인트 셀렉터 — 로컬 대상의 도달 가능 항목만 동적 노출. */}
                <select
                  value={delEndpoint()}
                  disabled={!delTarget()}
                  onInput={(e) => { setDelEndpoint(e.currentTarget.value); setDelInstance(""); }}
                  style="width:100%;padding:7px;border:1px solid #d5dae2;border-radius:8px;margin-bottom:11px;"
                >
                  <Show when={delEndpoints()?.new_acp !== false}>
                    <option value="new_acp">신규 ACP 스레드</option>
                  </Show>
                  <Show when={(delEndpoints()?.existing_acp?.length ?? 0) > 0}>
                    <option value="existing_acp">기존 ACP 세션 ({delEndpoints()!.existing_acp!.length})</option>
                  </Show>
                  <Show when={(delEndpoints()?.tmux?.length ?? 0) > 0}>
                    <option value="tmux">TMUX 세션 ({delEndpoints()!.tmux!.length})</option>
                  </Show>
                  <Show when={delEndpoints()?.worktree}>
                    <option value="worktree">신규 워크트리</option>
                  </Show>
                  <Show when={delEndpoints()?.external !== false}>
                    <option value="external">외부 A2A</option>
                  </Show>
                </select>
                {/* 기존 ACP 세션이 0개면 그 사유를 hint 로 명시 — 옵션이 안 보이는 게 버그가 아니라
                    "그 대상 alias 로 라벨된 라이브 ACP 세션이 없어서"임을 사용자에게 알린다.
                    (백엔드 a2a_list_agent_endpoints 는 label==alias 인 살아있는 세션만 existing_acp 로 반환.) */}
                <Show when={delTarget() && !delEpLoading() && (delEndpoints()?.existing_acp?.length ?? 0) === 0}>
                  <div style="font-size:11px;color:#b08;margin:-6px 0 11px;">
                    ℹ 기존 ACP 세션 없음 — 이 에이전트로 라벨된 라이브 ACP 세션이 없습니다(신규 ACP/외부 A2A 로 보내세요).
                  </div>
                </Show>
                {/* TMUX 세션 0개 hint — 백엔드 a2a_list_agent_endpoints 는 그 에이전트의 project_path
                    (또는 그 하위 cwd)에서 도는 tmux 세션만 반환한다. 0개면 옵션이 안 보이는 게 버그가
                    아니라 "이 프로젝트 폴더 아래 tmux 세션이 없어서"임을 명시. */}
                <Show when={delTarget() && !delEpLoading() && delEndpoints()?.worktree && (delEndpoints()?.tmux?.length ?? 0) === 0}>
                  <div style="font-size:11px;color:#b08;margin:-6px 0 11px;">
                    ℹ 이 프로젝트 폴더에 tmux 세션 없음 — 대상의 project_path(또는 그 하위)에서 도는 tmux 세션이 없습니다.
                  </div>
                </Show>
                {/* 2단 셀렉터 — existing_acp/tmux 인스턴스가 여럿일 때 어느 것으로 보낼지. */}
                <Show when={endpointInstances().length > 0}>
                  <div style="font-size:12px;color:#8b95a1;margin-bottom:3px;">인스턴스 선택</div>
                  <select value={delInstance()} onInput={(e) => setDelInstance(e.currentTarget.value)} style="width:100%;padding:7px;border:1px solid #d5dae2;border-radius:8px;margin-bottom:11px;">
                    <option value="">— 선택 —</option>
                    <For each={endpointInstances()}>{(o) => <option value={o.value}>{o.label}</option>}</For>
                  </select>
                </Show>
                </Show>
                <div style="font-size:12px;color:#8b95a1;margin-bottom:3px;">내용</div>
                <div style="font-size:13px;border:1px solid #eee;border-radius:8px;padding:8px;max-height:130px;overflow:auto;margin-bottom:14px;white-space:pre-wrap;">{draft()}</div>
                <div style="display:flex;gap:8px;justify-content:flex-end;">
                  <button onClick={() => setDelOpen(false)} style="padding:6px 14px;border:1px solid #d5dae2;background:#fff;border-radius:8px;cursor:pointer;">취소</button>
                  <button disabled={!delTarget() || delBusy()} onClick={() => void submitDelegate()} style="padding:6px 14px;background:#238636;color:#fff;border:none;border-radius:8px;cursor:pointer;">{delBusy() ? "전송중…" : "보내기"}</button>
                </div>
              </div>
            </div>
          </Show>

          {/* ── 아티팩트 미리보기 패널 (우측 슬라이드) ──
              ⚠ position:fixed 필수 — 부모 .msgs 는 overflow-y:auto 스크롤 컨테이너라
              position:absolute;top:0 면 패널이 "스크롤된 콘텐츠 최상단"(뷰포트 밖)으로 가서
              📎 클릭해도 화면에 안 보였다(=무반응 버그). fixed 로 뷰포트에 고정한다. */}
          <Show when={artOpen()}>
            <div
              onClick={(e) => e.stopPropagation()}
              style="position:fixed;top:0;right:0;bottom:0;width:min(560px,94vw);min-width:280px;max-width:96vw;resize:horizontal;overflow:hidden;direction:rtl;background:#15171c;border-left:1px solid #2a2f3a;z-index:1050;display:flex;flex-direction:column;box-shadow:-4px 0 14px rgba(0,0,0,.35);"
            >
              <div style="direction:ltr;display:flex;align-items:center;justify-content:space-between;padding:10px 12px;border-bottom:1px solid #2a2f3a;">
                <span style="font-weight:700;color:#e6e9ee;font-size:13px;">📎 아티팩트 ({artifacts().length})</span>
                <span style="cursor:pointer;color:#9aa1ad;" onClick={() => setArtOpen(false)}>✕</span>
              </div>
              <div style="direction:ltr;flex:1;overflow:auto;padding:10px;display:flex;flex-direction:column;gap:10px;">
                <Show when={artifacts().length === 0}>
                  <div style="color:#8b95a1;font-size:12px;">대화에 파일·이미지가 없습니다.</div>
                </Show>
                <For each={artifacts()}>
                  {(a) => (
                    <div style="border:1px solid #2a2f3a;border-radius:8px;padding:8px;background:#1a1d24;">
                      <div style="display:flex;align-items:center;gap:6px;margin-bottom:6px;">
                        <span style="font-size:13px;">{a.kind === "image" ? "🖼" : a.kind === "code" ? "📄" : "📁"}</span>
                        <span style="flex:1;color:#e6e9ee;font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;" title={a.path}>{a.name}</span>
                      </div>
                      <Show when={a.kind === "image"}>
                        <img
                          src={a.src}
                          alt={a.name}
                          style="width:100%;max-height:140px;object-fit:contain;border-radius:6px;background:#0e0f13;cursor:zoom-in;"
                          onClick={() => setArtZoom(a.src)}
                          onError={(e) => { (e.currentTarget as HTMLImageElement).style.display = "none"; }}
                        />
                      </Show>
                      <div style="display:flex;flex-wrap:wrap;gap:10px;margin-top:6px;font-size:11px;align-items:center;">
                        {/* code/text/file: 인라인 읽기·편집 액션 (image 는 위 미리보기로 충분) */}
                        <Show when={a.kind !== "image"}>
                          <span style="cursor:pointer;color:#58a6ff;" onClick={() => toggleArtView(a.path)}>👁 {artState(a.path).open && !artState(a.path).edit ? "닫기" : "읽기"}</span>
                          <span style="cursor:pointer;color:#58a6ff;" onClick={() => toggleArtEdit(a.path)}>✎ {artState(a.path).open && artState(a.path).edit ? "닫기" : "편집"}</span>
                        </Show>
                        {/* 다운로드: image/실URL 은 native <a download>; 그 외는 fs_file_get → Blob 다운로드 */}
                        <Show
                          when={a.kind === "image" || isUsableUrl(a.src)}
                          fallback={<span style="cursor:pointer;color:#58a6ff;" onClick={() => void downloadArt(a.path, a.name)}>⬇ 다운로드</span>}
                        >
                          <a href={a.src} download={a.name} target="_blank" rel="noopener" style="color:#58a6ff;text-decoration:none;">⬇ 다운로드</a>
                        </Show>
                        <span style="cursor:pointer;color:#9aa1ad;" onClick={() => copyText(a.path)}>⧉ 경로 복사</span>
                      </div>
                      {/* 인라인 뷰어/에디터 — 와이드 라인은 가로 스크롤(줄바꿈 없음) */}
                      <Show when={artState(a.path).open}>
                        <div style="margin-top:8px;">
                          <Show when={artState(a.path).loading}>
                            <div style="color:#8b95a1;font-size:11px;">불러오는 중…</div>
                          </Show>
                          <Show when={artState(a.path).err}>
                            <div style="color:#f85149;font-size:11px;white-space:pre-wrap;word-break:break-word;">⚠ 읽기 실패: {artState(a.path).err}</div>
                          </Show>
                          <Show when={!artState(a.path).loading && !artState(a.path).err && !artState(a.path).edit}>
                            <pre style="overflow:auto;overflow-x:auto;white-space:pre;max-height:320px;font-size:11px;background:#0e0f13;color:#cdd6e0;padding:8px;border-radius:6px;margin:0;">{artState(a.path).content}</pre>
                          </Show>
                          <Show when={!artState(a.path).loading && !artState(a.path).err && artState(a.path).edit}>
                            <textarea
                              wrap="off"
                              style="white-space:pre;overflow:auto;overflow-x:auto;width:100%;min-height:200px;font-family:monospace;font-size:11px;background:#0e0f13;color:#cdd6e0;border:1px solid #2a2f3a;border-radius:6px;padding:8px;box-sizing:border-box;"
                              value={artState(a.path).buffer}
                              onInput={(e) => patchArtState(a.path, { buffer: (e.currentTarget as HTMLTextAreaElement).value })}
                            />
                            <div style="display:flex;gap:8px;align-items:center;margin-top:6px;">
                              <button
                                disabled={artState(a.path).saving}
                                onClick={() => void saveArt(a.path)}
                                style={`font-size:11px;padding:4px 12px;border-radius:6px;border:1px solid #2a2f3a;background:#238636;color:#fff;cursor:pointer;opacity:${artState(a.path).saving ? "0.6" : "1"};`}
                              >{artState(a.path).saving ? "저장 중…" : "저장"}</button>
                              <Show when={artState(a.path).saveMsg}>
                                <span style={`font-size:11px;color:${artState(a.path).saveMsg!.ok ? "#3fb950" : "#f85149"};`}>{artState(a.path).saveMsg!.text}</span>
                              </Show>
                            </div>
                          </Show>
                        </div>
                      </Show>
                      <div style="color:#5c6470;font-size:10px;margin-top:4px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;" title={a.path}>{a.path}</div>
                    </div>
                  )}
                </For>
              </div>
            </div>
          </Show>
          {/* 이미지 확대 오버레이 */}
          <Show when={artZoom()}>
            <div
              style="position:fixed;inset:0;background:rgba(0,0,0,.8);z-index:1100;display:flex;align-items:center;justify-content:center;cursor:zoom-out;"
              onClick={() => setArtZoom(null)}
            >
              <img src={artZoom()!} style="max-width:92vw;max-height:92vh;object-fit:contain;" onError={() => setArtZoom(null)} />
            </div>
          </Show>
        </div>

        {/* ── 컴포저 (TalkTab 정본 Claude Code 다크 재사용) ── */}
        <div class="composer-wrap">
          {/* status line — 폴더위치 · 사용모델 · 역할 · 공개여부 · 연결 워크플로우 · 토큰사용량. */}
          <div class="kk-statusline">
            <Show when={props.preset?.cwd}>
              <span class="kk-sl" title={props.preset!.cwd!}>📁 {props.preset!.cwd}</span>
            </Show>
            <span class="kk-sl">🤖 {modelName()}</span>
            <Show when={props.status?.().role}>
              <span class="kk-sl">🎭 {props.status!().role}</span>
            </Show>
            <span class="kk-sl">{props.status?.().isPublic ? "🌐 공개" : "🔒 비공개"}</span>
            <Show when={(props.status?.().workflows ?? 0) > 0}>
              <span class="kk-sl">🔀 워크플로우 {props.status!().workflows}</span>
            </Show>
            <span class="kk-sl kk-sl-usage">⚡ {usageLabel()}</span>
          </div>
          <Show when={queue().length > 0}>
            <div style="border:1px solid #2f6a3a; border-radius:8px; padding:6px 8px; margin-bottom:6px; background:#16241b;">
              <div style="color:#7fc99a; font-size:11.5px; margin-bottom:4px;">⏱ 대기열 ({queue().length}) — 현재 턴 끝나면 순서대로 전송</div>
              <For each={queue()}>
                {(q, i) => (
                  <div style="display:flex; align-items:center; gap:8px; padding:3px 0;">
                    <span style="flex:1; color:#cfe3d6; font-size:12px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;">{q}</span>
                    <span style="cursor:pointer; color:#9aa1ad;" title="대기열에서 제거" onClick={() => setQueue(queue().filter((_, j) => j !== i()))}>✕</span>
                  </div>
                )}
              </For>
            </div>
          </Show>
          <div class="composer">
            <textarea
              class="ph-input"
              rows="2"
              placeholder={busy() ? "후속 메시지 대기열에 추가… (현재 턴 끝나면 전송)" : "프롬프트 입력···  ⚡ ACP 에이전트로 전송"}
              value={draft()}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setDraft(v);
                // "/" 입력(공백 전 = 명령 타이핑 중)이면 슬래시 셀렉터 자동 표시 + 타이핑으로 필터.
                const m = v.match(/^\/(\S*)$/);
                if (m) {
                  setLeftMenu("slash");
                  setSlashFilter(m[1]);
                } else if (leftMenu() === "slash") {
                  setLeftMenu(null);
                  setSlashFilter("");
                }
              }}
              onKeyDown={onKey}
            />
            <div class="bar">
              <div class="bar-l">
                {/* @ 파일 참조 — cwd 파일 목록 → @경로 삽입 */}
                <span class="kk-chip-wrap">
                  <span class="ic-btn" title="파일 참조(@경로 삽입)" onClick={() => void toggleAt()}>@</span>
                  <Show when={leftMenu() === "at"}>
                    <div class="kk-chip-menu kk-at-menu">
                      <input
                        class="kk-at-filter"
                        placeholder="파일 검색…"
                        value={fileFilter()}
                        onInput={(e) => setFileFilter(e.currentTarget.value)}
                      />
                      <Show when={fileList() == null}>
                        <div class="kk-chip-opt">로딩…</div>
                      </Show>
                      <Show when={fileList() != null && fileList()!.length === 0}>
                        <div class="kk-chip-opt">파일 없음</div>
                      </Show>
                      <For
                        each={(fileList() ?? [])
                          .filter((f) => f.toLowerCase().includes(fileFilter().toLowerCase()))
                          .slice(0, 60)}
                      >
                        {(f) => (
                          <div class="kk-chip-opt" onClick={() => insertAt(f)}>{f}</div>
                        )}
                      </For>
                    </div>
                  </Show>
                </span>
                {/* / 슬래시 명령 — 에이전트의 실제 availableCommands(하드코딩 아님) */}
                <span class="kk-chip-wrap">
                  <span class="ic-btn" title="슬래시 명령" onClick={() => setLeftMenu(leftMenu() === "slash" ? null : "slash")}>/</span>
                  <Show when={leftMenu() === "slash"}>
                    <div class="kk-chip-menu kk-at-menu">
                      <input
                        class="kk-at-filter"
                        placeholder="명령 검색…"
                        value={slashFilter()}
                        onInput={(e) => setSlashFilter(e.currentTarget.value)}
                      />
                      <Show when={availCmds().length === 0}>
                        <div class="kk-chip-opt" style="opacity:.6">명령 로딩 중… (첫 응답 후 표시)</div>
                      </Show>
                      <For each={availCmds().filter((c) => c.name.toLowerCase().includes(slashFilter().toLowerCase())).slice(0, 80)}>
                        {(c) => (
                          <div class="kk-chip-opt" onClick={() => insertSlash(c.name)} title={c.description ?? ""}>
                            <b>/{c.name}</b>
                            <Show when={c.description}><span style="opacity:.55; margin-left:6px;">{c.description!.slice(0, 40)}</span></Show>
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                </span>
                {/* 📎 파일 첨부 */}
                <span class="ic-btn" onClick={attachFile} title="파일·문서 첨부">📎</span>
                {/* ⇢ 다른 에이전트에게 위임 — 셀렉터 모달(대상 에이전트 + 전달 방식). */}
                <span class="ic-btn" onClick={openDelegate} title="다른 에이전트에게 위임">⇢</span>
                {/* 📥 가져오기는 컴포저에서 제거 — 우측 정보 패널(⌗ 상태)의 "가져오기/보내기" 섹션으로 이동.
                    (자주 안 쓰는 기능 → 컴포저 단순화. 트리거는 importTrigger prop 로 외부에서.) */}
                <span class="divv" />
                {/* 권한 모드 — Bypass Permissions 면 에이전트가 도구를 실제 실행(default-deny 해제). */}
                <span class="kk-chip-wrap">
                  <span class="mode perm" title="권한 모드" onClick={() => setOpenMenu(openMenu() === "perm" ? null : "perm")}>
                    {labelOf(PERM_OPTS, permMode())} <span class="car">⌃</span>
                  </span>
                  <Show when={openMenu() === "perm"}>
                    <div class="kk-chip-menu">
                      <For each={PERM_OPTS}>
                        {(o) => (
                          <div class={`kk-chip-opt${permMode() === o.v ? " on" : ""}`} onClick={() => selectChip("perm", o.v)}>{o.label}</div>
                        )}
                      </For>
                    </div>
                  </Show>
                </span>
                {/* 모델 — ANTHROPIC_MODEL env 로 에이전트 프로세스에 전달. */}
                <span class="kk-chip-wrap">
                  <span class="mode model" title="모델" onClick={() => setOpenMenu(openMenu() === "model" ? null : "model")}>
                    {labelOf(MODEL_OPTS, model())} <span class="car">⌃</span>
                  </span>
                  <Show when={openMenu() === "model"}>
                    <div class="kk-chip-menu kk-at-menu">
                      <input
                        class="kk-at-filter"
                        placeholder="모델 검색 (claude, gpt, codex…)"
                        value={modelFilter()}
                        onInput={(e) => setModelFilter(e.currentTarget.value)}
                      />
                      {/* Default(어댑터 기본) — 맨 위. */}
                      <div class={`kk-chip-opt${model() === "default" ? " on" : ""}`} onClick={() => selectChip("model", "default")}>
                        Default (recommended)
                      </div>
                      <Show when={orModels.loading}>
                        <div class="kk-chip-opt" style="opacity:.6">모델 불러오는 중…</div>
                      </Show>
                      {/* 모델 목록 — Claude(Anthropic, fable-5 등) → OpenAI(gpt) → 알파벳 순. */}
                      <For
                        each={(orModels() ?? [])
                          .filter((m) => {
                            const f = modelFilter().toLowerCase();
                            return !f || m.id.toLowerCase().includes(f) || m.name.toLowerCase().includes(f) || m.provider.toLowerCase().includes(f);
                          })
                          .sort((a, b) => {
                            const rank = (p: string) => { const x = p.toLowerCase(); return (x.includes("anthropic") || x.includes("claude")) ? 0 : (x.includes("openai") || x.includes("gpt")) ? 1 : 2; };
                            const d = rank(a.provider) - rank(b.provider);
                            return d !== 0 ? d : `${a.provider}${a.id}`.localeCompare(`${b.provider}${b.id}`);
                          })
                          .slice(0, 60)}
                      >
                        {(m) => (
                          <div class={`kk-chip-opt${model() === m.id ? " on" : ""}`} onClick={() => selectChip("model", m.id)} title={m.name}>
                            <b>{m.id}</b> <span style="opacity:.5; font-size:10.5px;">{m.provider}</span>
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                </span>
                {/* thinking — MAX_THINKING_TOKENS env. */}
                <span class="kk-chip-wrap">
                  <span class="mode think" title="thinking 강도" onClick={() => setOpenMenu(openMenu() === "think" ? null : "think")}>
                    {labelOf(THINK_OPTS, thinking())} <span class="car">⌃</span>
                  </span>
                  <Show when={openMenu() === "think"}>
                    <div class="kk-chip-menu">
                      <For each={THINK_OPTS}>
                        {(o) => (
                          <div class={`kk-chip-opt${thinking() === o.v ? " on" : ""}`} onClick={() => selectChip("think", o.v)}>{o.label}</div>
                        )}
                      </For>
                    </div>
                  </Show>
                </span>
              </div>
              <div class="spacer" />
              <div class="bar-r">
                <Show when={busy()}>
                  <span class="kk-acp-cancel" onClick={() => void cancelTurn()}>■ 취소</span>
                </Show>
                {/* 목업 정본: 토큰사용/컨텍스트윈도우 (%) · 비용. 모델명·버전은 모델 칩에 표시. */}
                {/* 토큰 사용량은 status line(composer 상단)으로 이동 — 중복 제거. */}
                <span
                  class={`send${draft().trim() ? "" : " dis"}`}
                  title={busy() ? "대기열에 추가 (턴 종료 시 전송)" : "전송"}
                  onClick={() => submit()}
                >
                  {busy() ? (draft().trim() ? "➕" : "…") : "➤"}
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
}
