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
  const MODEL_OPTS = [{ v: "default", label: "Default (recommended)" }];
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
          { identifier: convKey(), filename: f.name, content_b64: b64 },
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
    setDraft(d && !d.endsWith(" ") ? `${d} ${cmd}` : `${d}${cmd}`);
    setLeftMenu(null);
    setSlashFilter("");
  }
  const [busy, setBusy] = createSignal(false); // 세션 생성/프롬프트 진행 중
  const [streaming, setStreaming] = createSignal(false);
  // 응답 중(busy)에 입력한 후속 메시지 대기열 — 현재 턴 종료 시 순서대로 자동 전송.
  const [queue, setQueue] = createSignal<string[]>([]);
  // 런타임(하네스) 설정 — 메모리 주입 등. 백엔드 identity_settings.
  const [rtCfg] = createResource(() => invoke<any>("runtime_config_get").then((r) => r?.config).catch(() => null));
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

  function pushBubble(b: Bubble) {
    setBubbles((prev) => [...prev, b]);
    scrollDown();
  }

  // session/update 한 건을 버블에 반영 (스트림 + prompt 응답 updates 공용).
  function applyUpdate(u: unknown) {
    if (!u || typeof u !== "object") return;
    const o = u as Record<string, unknown>;
    const tag = o.sessionUpdate as string | undefined;
    if (!tag) return;
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
  async function loadHistory(): Promise<boolean> {
    try {
      const rows = await invoke<{ role: string; text: string }[]>("acp_conv_list", { key: convKey() });
      if (!Array.isArray(rows) || rows.length === 0) return false;
      const restored: Bubble[] = rows.map((r) => {
        if (r.role === "agent") return { id: nextId++, kind: "agent", segs: segmentText(r.text), time: "" };
        if (r.role === "note") return { id: nextId++, kind: "note", text: r.text, time: "" };
        return { id: nextId++, kind: "me", text: r.text, time: "" };
      });
      restored.push({ id: nextId++, kind: "note", text: "↑ 이전 대화 복원됨 — 이어서 대화하세요.", time: nowClock() });
      setBubbles(restored);
      // true resume: 새 어댑터 프로세스에 이전 맥락을 첫 프롬프트로 주입 → 에이전트가 실제로 이어감.
      pendingContext = buildContextPreamble();
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
      };
      const r = await acpFetch<{ sessionId: string; agent: string; spawned: boolean }>(
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
        pushBubble({
          id: nextId++,
          kind: "note",
          text: `⚡ ACP 세션 시작 — ${agent}${r.spawned ? " (구동됨)" : " (첫 프롬프트 시 구동)"}`,
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
    const uid = nextId; // 이번 turn 의 user 버블 id — 이후 생성된 agent 버블 식별용.
    pushBubble({ id: nextId++, kind: "me", text, time: nowClock() });
    void recordMsg("me", text); // 사용자 메시지 영속화(실제 입력만).
    setDraft("");
    curAgentBubbleId = null;
    // 런타임 메모리 주입(하네스) — inject_memory 면 세션 첫 프롬프트에 OpenXgram L2 메모리를
    // 앞에 붙인다(토큰예산=memory_count). 차별화: 에이전트가 OpenXgram 기억을 자기 맥락으로 씀.
    let memPreamble = "";
    const cfg = rtCfg();
    if (cfg?.inject_memory && !memInjected) {
      memInjected = true;
      try {
        const rc = await invoke<any>("runtime_context", { count: String(cfg.memory_count ?? 8) });
        const mems = (rc?.memories ?? []).map((m: any) => `[${m.kind}] ${m.content}`).join("\n");
        if (mems) memPreamble += `[OpenXgram 기억 — 이 에이전트가 참고할 사실/결정/규칙]\n${mems}\n\n`;
        if (cfg.inject_wiki && rc?.wiki?.length) memPreamble += `[OpenXgram 위키] ${rc.wiki.map((w: any) => w.title).join(", ")}\n\n`;
        if (memPreamble) pushBubble({ id: nextId++, kind: "note", text: `🧠 런타임: 메모리 ${rc?.memory_count ?? 0}개 주입`, time: nowClock() });
      } catch { /* 주입 실패는 무시 */ }
    }
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
    }
    // turn 종료 후 에이전트 응답 텍스트 영속화(이번 turn 에 생성된 agent 버블 합산).
    const aText = bubbles()
      .filter((b) => b.kind === "agent" && b.id > uid)
      .map((b) => (b.kind === "agent" ? b.segs.filter((s) => s.kind === "text").map((s) => s.text).join("") : ""))
      .join("\n")
      .trim();
    if (aText) void recordMsg("agent", aText);
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
    try {
      await acpFetch("POST", `/sessions/${encodeURIComponent(id)}/cancel`);
      pushBubble({ id: nextId++, kind: "note", text: "· 취소 요청 전송", time: nowClock() });
    } catch (e) {
      pushBubble({ id: nextId++, kind: "note", text: `⚠ 취소 실패: ${(e as Error)?.message ?? e}`, time: nowClock() });
    }
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
    stopStream?.();
    const id = sessionId();
    if (id) void acpFetch("DELETE", `/sessions/${encodeURIComponent(id)}`).catch(() => {});
  });

  // 상세 패널 "세션 재시작" 트리거 — 값이 증가하면 닫고 재구동(대화창 복귀).
  createEffect<number>((prev) => {
    const v = props.restartTrigger?.() ?? 0;
    if (prev !== undefined && v !== prev && v > 0) void closeSession();
    return v;
  });

  // 전송 — 응답 중(busy)이면 대기열에 적재(턴 종료 시 자동 전송), 아니면 즉시 전송.
  function submit() {
    const text = draft().trim();
    if (!text) return;
    // /clear — ACP 메신저엔 CLI 하니스가 없어 자동 처리 안 됨(텍스트로 떨어져 에이전트가 설명만 함).
    // 우리가 직접: 영속 기록 삭제 + UI 비우기 + ACP subprocess 재시작(새 session/new = 컨텍스트 초기화).
    if (text === "/clear") { setDraft(""); void clearConversation(); return; }
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
            <Show when={props.popoutAlias}>
              <span class="kk-acp-pop" title="새 창으로 열기" onClick={() => openPopout(props.popoutAlias!)}>⤢ 새 창</span>
            </Show>
            {/* 세션 닫기(=재시작)는 헤더에 노출하지 않음 — 상세 패널에서 제어(props.restartTrigger). */}
          </div>
        </div>

        <div class="msgs" ref={msgsRef}>
          <For each={bubbles()}>
            {(b) =>
              b.kind === "me" ? (
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
                <div class="agent">
                  <div class="head">
                    <div class="av c-claude">⚡</div>
                    <div class="nm">{props.preset?.displayName || activeAgent()}</div>
                    <div class="tm">{b.time}</div>
                  </div>
                  <div class="body">
                    <For each={b.segs}>
                      {(seg) =>
                        seg.kind === "code" ? <pre class="code">{seg.text}</pre> : <p>{seg.text}</p>
                      }
                    </For>
                  </div>
                </div>
              )
            }
          </For>
          <Show when={bubbles().length === 0}>
            <div class="kk-talk-empty">세션 준비됨. 아래에서 첫 프롬프트를 보내세요.</div>
          </Show>
          {/* 응답 대기 표시 — 메시지 보낸 뒤 에이전트 응답이 오기 전까지 '응답 중' 인디케이터. */}
          <Show when={busy() && bubbles().length > 0}>
            <div class="agent kk-acp-typing">
              <div class="head"><span class="nm">⚡ 에이전트</span></div>
              <div class="body"><span class="kk-typing"><i /><i /><i /></span> 응답 중…</div>
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
              onInput={(e) => setDraft(e.currentTarget.value)}
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
                      {/* Default(어댑터 기본) */}
                      <div class={`kk-chip-opt${model() === "default" ? " on" : ""}`} onClick={() => selectChip("model", "default")}>
                        Default (recommended)
                      </div>
                      <Show when={orModels.loading}>
                        <div class="kk-chip-opt" style="opacity:.6">모델 불러오는 중…</div>
                      </Show>
                      <For
                        each={(orModels() ?? [])
                          .filter((m) => {
                            const f = modelFilter().toLowerCase();
                            return !f || m.id.toLowerCase().includes(f) || m.name.toLowerCase().includes(f) || m.provider.toLowerCase().includes(f);
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
