import { createSignal, createResource, createMemo, onCleanup, For, Show } from "solid-js";
import { invoke } from "../api/client";
import { ProviderLogo, providerKey } from "./ProviderLogo";
import { AddAgentModal } from "./AddAgentModal";
import { computeUnregisteredSessions, type SessionsDto, type DetectedSession } from "./agentSessions";
import "./agents-extra.css";

// Phase 2 (B+C+D) — 에이전트 탭. 정본 디자인: _mockups/kakao-mockup.html.
// 좌: 명부(분류 그룹화) + 8단계 추가 폼. 우: 프로필(정보·실행모드·동적 설정탐지·설정).
// 백엔드: /v1/gui/agents(LEFT JOIN agent_profiles), /agent/{alias}/profile, /agent/{alias}/config-chain.
//
// 정본 목업의 외부채널(channelOvl)·tmux 터미널(termOvl)·지침/MCP 편집(edOvl) 3개 오버레이를
// 프로필 진입점(qbtn·cfgrow)에서 열도록 이식. 실데이터: bindings_status(채널) /
// session_screen(tmux 라이브 화면) / fs_file_get·fs_file_put(편집 모달 본문 — 읽기·쓰기).
// 프로필에 project_path 파일트리(fs_tree), 추가 모달에 폴더 선택 트리(fs_tree)도 연결.

interface AgentRow {
  alias: string;
  display_name?: string | null;
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
  source?: string | null;       // 'user' | 'built_in'
  activated?: boolean | null;   // built_in 동봉 에이전트의 활성화 여부
}

// 표시 이름 — TalkTab 과 동일 규칙(대화명 있으면 그것, 없으면 alias).
const agentName = (a: { display_name?: string | null; alias: string }) =>
  (a.display_name && a.display_name.trim()) || a.alias;

interface Profile {
  alias: string;
  exists: boolean;
  display_name?: string | null;
  ai_type: string;
  classification: string;
  execution_mode: string;
  machine?: string | null;
  worktree?: string | null;
  is_public: boolean;
  role?: string | null;
  group?: string | null;
  folder?: string | null;
  description?: string | null;
}

interface ChainNode {
  path: string;
  scope: string;
  exists: boolean;
  bytes?: number;
  imports?: ChainNode[];
  raw?: string;
}

interface ConfigChain {
  ok: boolean;
  alias: string;
  ai_type: string;
  project_path?: string;
  instruction_chain: ChainNode[];
  mcp_servers: string[];
  mcp_source?: string | null;
  settings_files: { path: string; scope: string; exists: boolean }[];
  hooks: { event: string; matcher: string; scope: string }[];
  env_keys: string[];
  skills: string[];
  error?: string;
}

// 파일 트리 노드 — fs_tree 라우트 반환 형태.
interface FsNode {
  name: string;
  path: string;
  is_dir: boolean;
  children?: FsNode[];
}

// 외부 채널 바인딩 — bindings_status 라우트 (Messenger.tsx 와 동일). agent_id 로 키.
interface BindingStatus {
  agent_id: string;
  platform: string;
  channel_ref: string;
  bot_alias?: string | null;
  bot_label?: string | null;
  matched_session_count: number;
  latest_preview?: string | null;
  match_status: string;
}

// tmux 라이브 화면 — session_screen 라우트 (Messenger.tsx 와 동일).
interface SessionScreenDto {
  identifier: string;
  kind: string;
  display: string;
  content: string;
  lines: number;
  source_note: string;
  fetched_at: string;
}

// 편집 모달이 여는 설정 파일 항목 (config-chain 에서 도출). raw 없으면 읽기전용 빈 본문.
interface EditorFile {
  title: string;
  path: string;
  scope: string;
  body: string;
  hasRaw: boolean;
}

// 채널 플랫폼 → 아바타 아이콘·색 클래스 (목업 .ch-dc/.ch-tg/.ch-sl).
function channelVisual(platform: string): { icon: string; cls: string } {
  const p = (platform || "").toLowerCase();
  if (p.includes("discord")) return { icon: "💬", cls: "ax-ch-dc" };
  if (p.includes("telegram")) return { icon: "✈️", cls: "ax-ch-tg" };
  if (p.includes("slack")) return { icon: "💼", cls: "ax-ch-sl" };
  return { icon: "🔗", cls: "ax-ch-xx" };
}

const AI_COLOR: Record<string, string> = {
  claude: "c-claude", codex: "c-codex", gemini: "c-gemini", ollama: "c-ollama", hermes: "c-hermes",
};
const CLASS_GROUPS = [
  { key: "primary", icon: "👑", label: "프라이머리" },
  { key: "project", icon: "📁", label: "프로젝트 에이전트" },
  { key: "special", icon: "⚙️", label: "특수 기능 에이전트" },
];
const CLASS_LABEL: Record<string, string> = {
  primary: "👑 프라이머리", project: "📁 프로젝트 에이전트", special: "⚙️ 특수 기능 에이전트",
};
const EXEC_MODES = [
  { key: "always", label: "⚡ 상시 켜둠" },
  { key: "on_demand", label: "🔌 필요할 때만" },
  { key: "heartbeat", label: "😴 깨움 (하트비트)" },
];

function avatarColor(ai?: string | null): string {
  return (ai && AI_COLOR[ai.toLowerCase()]) || "c-group";
}

export function AgentsTab(props: { onGotoChat?: (alias: string) => void; onGotoMarket?: () => void }) {
  const [agents, { refetch: refetchAgents }] = createResource<AgentRow[]>(() => invoke("agents_list"));
  const [selected, setSelected] = createSignal<string | null>(null);
  const [showAdd, setShowAdd] = createSignal(false);
  // 미등록 tmux "추가" 진입 시 그 세션 cwd 를 폴더로 prefill(=project_path). null 이면 일반 추가.
  const [addPrefillFolder, setAddPrefillFolder] = createSignal<string | null>(null);
  // sessions 라우트(이 머신 tmux+워크트리) — "추가되지 않은 에이전트" 도출 소스. TalkTab 과 동일 contract.
  const [sessions] = createResource<SessionsDto>(() => invoke("sessions"));
  // 정본 목업 보조 오버레이 — 각각 프로필 진입점에서 열림.
  const [showChannel, setShowChannel] = createSignal(false);
  const [showTerm, setShowTerm] = createSignal(false);
  const [editorFile, setEditorFile] = createSignal<EditorFile | null>(null);

  const [profile, { refetch: refetchProfile }] = createResource(
    () => selected() ?? undefined,
    (alias) => invoke<Profile>("agent_profile_get", { alias }),
  );
  const [chain] = createResource(
    () => {
      const a = selected();
      if (!a) return null;
      const p = profile();
      return { alias: a, ai_type: p?.ai_type ?? "claude" };
    },
    (src) => invoke<ConfigChain>("agent_config_chain", { alias: src.alias, ai_type: src.ai_type }),
  );

  // 🔍 에이전트 검색 — alias(id 포함, 예 aoe_X_d63f41f)·역할(tmux)·경로(turbo)·ai_type·머신·분류 등 횡단 부분일치.
  const [q, setQ] = createSignal("");
  const matchAgent = (a: AgentRow, ql: string) =>
    !ql ||
    [a.alias, a.display_name, a.role, a.description, a.ai_type, a.machine, a.project_path, a.group_name, a.classification, a.execution_mode]
      .some((f) => (f ?? "").toLowerCase().includes(ql));

  const grouped = createMemo(() => {
    const ql = q().trim().toLowerCase();
    const list = (agents() ?? []).filter((a) => matchAgent(a, ql));
    const by: Record<string, AgentRow[]> = { primary: [], project: [], special: [] };
    for (const a of list) {
      const cls = a.classification && by[a.classification] ? a.classification : "project";
      by[cls].push(a);
    }
    return by;
  });

  // ➕ "추가되지 않은 에이전트" — 감지된 tmux 중 어느 에이전트 폴더와도 안 맞는 것(미등록).
  //   TalkTab 과 동일 로직(./agentSessions 공유). 클릭 → AddAgentModal 을 cwd prefill 로 열어 등록.
  const unregistered = createMemo<DetectedSession[]>(() =>
    computeUnregisteredSessions(
      sessions()?.sessions ?? [],
      (agents() ?? []).map((a) => a.project_path ?? "").filter(Boolean),
    ),
  );

  // 폴더 끝 세그먼트만 짧게(전체 경로는 title). TalkTab baseName 동일.
  const baseName = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

  // 미등록 세션 "추가" — 그 세션 cwd 를 prefill 로 추가 모달 열기.
  function addFromSession(s: DetectedSession) {
    setAddPrefillFolder(s.cwd ? s.cwd.trim() : null);
    setShowAdd(true);
  }

  async function setExecMode(mode: string) {
    const a = selected();
    if (!a) return;
    await invoke("agent_profile_set", { alias: a, execution_mode: mode });
    await refetchProfile();
    await refetchAgents();
  }

  async function setDisplayName(name: string) {
    const a = selected();
    if (!a) return;
    await invoke("agent_profile_set", { alias: a, display_name: name });
    await refetchProfile();
    await refetchAgents();
  }

  async function setClassification(cls: string) {
    const a = selected();
    if (!a) return;
    await invoke("agent_profile_set", { alias: a, classification: cls });
    await refetchProfile();
    await refetchAgents();
  }

  // built-in 동봉 에이전트(xgram-ops 등) 활성/비활성 토글 → 활성화해야 명부 노출·peer 통신.
  async function activate(alias: string, on = true) {
    await invoke("agent_activate", { alias, activate: on });
    await refetchAgents();
  }

  return (
    <div class="kk-agents">
      <div class="kk-roster">
        <div class="rtop">
          <h2>에이전트</h2>
          <button class="kk-add" onClick={() => { setAddPrefillFolder(null); setShowAdd(true); }}>➕ <span class="lbl">에이전트 추가</span></button>
        </div>
        {/* 🔍 검색 — alias·id·역할·경로·ai_type·머신 등 횡단 부분일치(예: d63f41f, turbo, aoe, tmux) */}
        <input
          type="text"
          value={q()}
          onInput={(e) => setQ(e.currentTarget.value)}
          placeholder="🔍 검색 (이름·id·역할·tmux·경로 …)"
          style="display:block;width:calc(100% - 32px);box-sizing:border-box;margin:6px 16px 8px;padding:9px 12px;border:1px solid #e3e5e9;border-radius:10px;background:#f1f2f4;color:#333;font-size:13px;outline:none;"
        />
        <Show when={!agents.loading} fallback={<div class="empty">불러오는 중…</div>}>
          <For each={CLASS_GROUPS}>
            {(g) => (
              <Show when={(grouped()[g.key] ?? []).length > 0}>
                <div class="kk-gt">
                  {g.icon} {g.label} <span class="sub">({grouped()[g.key].length})</span>
                </div>
                <For each={grouped()[g.key]}>
                  {(a) => (
                    <div
                      class={`kk-row${selected() === a.alias ? " active" : ""}${g.key === "primary" ? " primary" : ""}`}
                      onClick={() => setSelected(a.alias)}
                    >
                      <div class={`kk-ava ${avatarColor(a.ai_type)}`}>
                        {a.alias.slice(0, 1).toUpperCase()}
                        <span class={`dot${a.messenger_enabled ? " on" : ""}`} />
                      </div>
                      <div class="kk-meta">
                        <div class="kk-nm">
                          {agentName(a)}
                          <ProviderLogo provider={providerKey(a)} />
                          <Show when={a.is_public}><span class="tag">공개</span></Show>
                          {/* built-in 동봉 에이전트: 미활성이면 활성화 버튼, 활성이면 배지. */}
                          <Show when={a.source === "built_in" && !a.activated}>
                            <button
                              class="tag"
                              style="cursor:pointer; background:#2f6a3a; color:#fff; border:none; font-weight:700;"
                              title="이 동봉 에이전트를 활성화"
                              onClick={(e) => { e.stopPropagation(); activate(a.alias); }}
                            >활성화</button>
                          </Show>
                          <Show when={a.source === "built_in" && a.activated}>
                            <span class="tag" style="background:#2f6a3a33; color:#7fc99a;">활성</span>
                          </Show>
                        </div>
                        {/* 에이전트명(ID) — TalkTab 카드와 동일 표시. */}
                        <div class="kk-card-sub">
                          <span class="kk-card-alias" title="에이전트명(ID)">@{a.alias}</span>
                        </div>
                        <div class="kk-st">{a.role || a.description || "—"}</div>
                      </div>
                    </div>
                  )}
                </For>
              </Show>
            )}
          </For>
          {/* ➕ 추가되지 않은 에이전트 — 감지된 tmux 중 미등록. 클릭 → cwd prefill 로 추가(등록). */}
          <Show when={unregistered().length > 0}>
            <div class="kk-gt">
              ➕ 추가되지 않은 에이전트 <span class="sub">({unregistered().length})</span>
            </div>
            <For each={unregistered()}>
              {(s) => (
                <div class="kk-row" title="클릭 → 대화명(alias) 부여해 등록" onClick={() => addFromSession(s)}>
                  <div class="kk-ava c-group">＋<span class="dot" /></div>
                  <div class="kk-meta">
                    <div class="kk-nm">
                      <span title={s.cwd ?? ""}>{s.cwd ? baseName(s.cwd) : "폴더 미상"}</span>
                      <span class="tag">tmux</span>
                    </div>
                    <div class="kk-card-sub">
                      <span class="kk-card-alias" title="tmux 세션 식별자">{s.identifier}</span>
                    </div>
                    <div class="kk-st" style="display:flex; align-items:center; gap:6px;">
                      <span>{s.display || s.identifier}</span>
                      <button
                        class="tag"
                        style="cursor:pointer; background:#2f6a3a; color:#fff; border:none; font-weight:700;"
                        title="이 세션을 에이전트로 추가"
                        onClick={(e) => { e.stopPropagation(); addFromSession(s); }}
                      >＋ 추가</button>
                    </div>
                  </div>
                </div>
              )}
            </For>
          </Show>
          <Show when={(agents() ?? []).length === 0}>
            <div class="empty">등록된 에이전트가 없습니다.<br />우측 상단 <b>➕ 에이전트 추가</b>로 등록하세요.</div>
          </Show>
        </Show>
      </div>

      <div class="kk-prof">
        <Show when={selected()} fallback={<div class="empty">좌측에서 에이전트를 선택하세요.</div>}>
          <Show when={!profile.loading && profile()} fallback={<div class="empty">프로필 불러오는 중…</div>}>
            {(p) => (
              <ProfileView
                p={p()}
                chain={chain()}
                chainLoading={chain.loading}
                onExec={setExecMode}
                onRename={setDisplayName}
                onSetClass={setClassification}
                onGotoChat={props.onGotoChat}
                onGotoMarket={props.onGotoMarket}
                onOpenChannel={() => setShowChannel(true)}
                onOpenTerm={() => setShowTerm(true)}
                onOpenEditor={(f) => setEditorFile(f)}
              />
            )}
          </Show>
        </Show>
      </div>

      <Show when={showAdd()}>
        <AddAgentModal
          prefillFolder={addPrefillFolder()}
          onClose={() => { setShowAdd(false); setAddPrefillFolder(null); }}
          onCreated={async (alias) => {
            setShowAdd(false);
            setAddPrefillFolder(null);
            await refetchAgents();
            setSelected(alias);
          }}
        />
      </Show>

      <Show when={showChannel() && selected()}>
        <ChannelOverlay alias={selected()!} onClose={() => setShowChannel(false)} />
      </Show>
      <Show when={showTerm() && selected()}>
        <TerminalOverlay alias={selected()!} onClose={() => setShowTerm(false)} />
      </Show>
      <Show when={editorFile()}>
        <EditorOverlay file={editorFile()!} onClose={() => setEditorFile(null)} />
      </Show>
    </div>
  );
}

function ProfileView(props: {
  p: Profile;
  chain: ConfigChain | undefined;
  chainLoading: boolean;
  onExec: (mode: string) => void;
  onRename: (name: string) => void;
  onSetClass: (cls: string) => void;
  onGotoChat?: (alias: string) => void;
  onGotoMarket?: () => void;
  onOpenChannel: () => void;
  onOpenTerm: () => void;
  onOpenEditor: (f: EditorFile) => void;
}) {
  const p = () => props.p;
  // 대화명(표시 이름) 인라인 편집 — null=비편집.
  const [dnEdit, setDnEdit] = createSignal<string | null>(null);
  const locked = () => p().classification === "primary" || p().classification === "special";
  const lockedMode = () => (p().classification === "primary" ? "always" : "heartbeat");
  const [walletNote, setWalletNote] = createSignal(false);

  // 탐지된 지침 노드 → 편집 모달용 파일 (raw 있으면 본문, 없으면 읽기전용 빈 본문).
  const nodeToEditor = (n: ChainNode, title: string): EditorFile => ({
    title,
    path: n.path,
    scope: n.scope,
    body: n.raw ?? "",
    hasRaw: !!n.raw,
  });

  return (
    <div>
      <div class="apvsec">정보</div>
      <div class="apvgrid">
        {/* 대화명(표시 이름) — 수정 가능. 비우면 에이전트명(alias) 으로 표시. */}
        <div class="apvcard">
          <div class="k">대화명 <span style="opacity:.6">(표시 이름)</span></div>
          <Show
            when={dnEdit() !== null}
            fallback={
              <div class="v" style="display:flex; align-items:center; gap:6px;">
                <span>{p().display_name || p().alias}</span>
                <span style="cursor:pointer; opacity:.6;" title="대화명 수정" onClick={() => setDnEdit(p().display_name || "")}>✏</span>
              </div>
            }
          >
            <div style="display:flex; gap:5px; align-items:center;">
              <input
                style="flex:1; min-width:0; background:#0f1216; border:1px solid #2b303a; border-radius:6px; color:#cfd5de; font-size:13px; padding:5px 8px; outline:none;"
                value={dnEdit() ?? ""}
                placeholder={p().alias}
                onInput={(e) => setDnEdit(e.currentTarget.value)}
                onKeyDown={(e) => { if (e.key === "Enter") { props.onRename((dnEdit() ?? "").trim()); setDnEdit(null); } if (e.key === "Escape") setDnEdit(null); }}
              />
              <button style="background:#2f6a3a; border:none; border-radius:6px; color:#fff; font-size:11.5px; padding:5px 9px; cursor:pointer;" onClick={() => { props.onRename((dnEdit() ?? "").trim()); setDnEdit(null); }}>저장</button>
              <button style="background:#222732; border:1px solid #2b303a; border-radius:6px; color:#cfd5de; font-size:11.5px; padding:5px 8px; cursor:pointer;" onClick={() => setDnEdit(null)}>✕</button>
            </div>
          </Show>
        </div>
        <div class="apvcard"><div class="k">에이전트명 (ID)</div><div class="v" style="font-family:ui-monospace,Menlo,monospace; font-size:12px;">{p().alias}</div></div>
        <div class="apvcard"><div class="k">AI 종류</div><div class="v" style="display:flex;align-items:center;gap:6px;"><ProviderLogo provider={providerKey(p())} /></div></div>
        {/* 분류 — 클릭해서 변경. primary 로 지정하면 기존 프라이머리는 자동 강등(단일 프라이머리). */}
        <div class="apvcard">
          <div class="k">분류 <span style="opacity:.6">(클릭 변경)</span></div>
          <div class="v" style="display:flex; gap:5px; flex-wrap:wrap; margin-top:2px;">
            {(["primary", "project", "special"] as const).map((c) => (
              <span
                onClick={() => { if (p().classification !== c) props.onSetClass(c); }}
                style={`cursor:pointer; font-size:11.5px; padding:4px 8px; border-radius:7px; border:1px solid ${p().classification === c ? "#fee500" : "#2b303a"}; background:${p().classification === c ? "#fee50022" : "transparent"}; color:${p().classification === c ? "#e6c200" : "#9aa1ad"}; font-weight:${p().classification === c ? 700 : 400};`}
              >
                {CLASS_LABEL[c] || c}
              </span>
            ))}
          </div>
        </div>
        <div class="apvcard"><div class="k">머신</div><div class="v">{p().machine || "—"}</div></div>
        <div class="apvcard"><div class="k">폴더</div><div class="v">{p().folder || "—"}</div></div>
        <div class="apvcard"><div class="k">역할</div><div class="v">{p().role || "—"}</div></div>
        <div class="apvcard"><div class="k">그룹</div><div class="v">{p().group || "—"}</div></div>
        <div class="apvcard"><div class="k">공개</div><div class="v">{p().is_public ? "🌐 공개" : "비공개"}</div></div>
        <div class="apvcard"><div class="k">워크트리</div><div class="v">{p().worktree || "—"}</div></div>
      </div>

      <div class="apvsec">프로젝트 폴더 <span class="auto">(파일 트리)</span></div>
      <Show
        when={p().folder}
        fallback={<div class="apvhint">이 에이전트에 설정된 프로젝트 폴더가 없습니다. (추가/수정 시 폴더 지정)</div>}
      >
        <div class="apvhint">
          <code>{p().folder}</code> — 폴더는 펼쳐서 탐색, 지침/설정 파일은 클릭해 편집할 수 있습니다.
        </div>
        <FileTree
          rootPath={p().folder!}
          depth={2}
          onPick={(node) => {
            // 편집 화이트리스트 대상 파일이면 편집 모달로 연다 (CLAUDE/AGENTS/GEMINI/settings*/.mcp.json/*.md).
            if (node.is_dir) return;
            const nm = node.name.toLowerCase();
            const editable =
              nm.endsWith(".md") || nm === ".mcp.json" ||
              (nm.startsWith("settings") && nm.endsWith(".json"));
            if (!editable) return;
            props.onOpenEditor({
              title: node.name,
              path: node.path,
              scope: "project",
              body: "",
              hasRaw: false,
            });
          }}
        />
      </Show>

      <div class="apvsec">실행 모드</div>
      <div class="kk-seg">
        <For each={EXEC_MODES}>
          {(m) => {
            const active = () => (locked() ? lockedMode() === m.key : p().execution_mode === m.key);
            return (
              <div
                class={`s${active() ? " on" : ""}${locked() ? " lock" : ""}`}
                onClick={() => { if (!locked()) props.onExec(m.key); }}
              >
                {m.label}
              </div>
            );
          }}
        </For>
      </div>
      <div class="apvhint">
        <b>프라이머리</b>는 상시 고정, <b>특수</b>는 깨움(하트비트) 고정. <b>프로젝트</b>만 선택 가능합니다.
      </div>

      <div class="apvsec">영향 주는 지침·설정 <span class="auto">(자동 탐지)</span></div>
      <div class="apvhint">
        이 에이전트는 <b>{p().ai_type}</b> — 아래 파일들이 <b>실제 적용 중</b>(데몬이 런타임 탐지).
        codex면 <code>AGENTS.md</code>, gemini면 <code>GEMINI.md</code> 체인으로 자동 변경됩니다.
      </div>
      <Show when={!props.chainLoading} fallback={<div class="apvhint">탐지 중…</div>}>
        <Show when={props.chain?.ok} fallback={<div class="apvhint">⚠ 탐지 실패: {props.chain?.error || "cwd 해석 불가 (tmux 세션/폴더 필요)"}</div>}>
          <For each={props.chain!.instruction_chain}>
            {(node) => <ChainRow node={node} depth={0} onOpenEditor={props.onOpenEditor} toEditor={nodeToEditor} />}
          </For>
          <Show when={props.chain!.mcp_servers.length > 0}>
            <div
              class="cfgrow"
              onClick={() => props.onOpenEditor({
                title: "MCP 서버",
                path: props.chain!.mcp_source || ".mcp.json",
                scope: "MCP",
                body: `# MCP 서버 (탐지)\n${props.chain!.mcp_servers.join("\n")}\n`,
                hasRaw: false,
              })}
            >
              <span class="cfi">🔌</span>
              <div><div class="cfp">{props.chain!.mcp_source || ".mcp.json"}</div>
                <div class="cfc">MCP 서버: {props.chain!.mcp_servers.join(" · ")}</div></div>
              <span class="cfx">MCP</span>
            </div>
          </Show>
          <For each={props.chain!.settings_files.filter((s) => s.exists)}>
            {(s) => (
              <div
                class="cfgrow"
                onClick={() => props.onOpenEditor({
                  title: "settings.json (권한·훅)",
                  path: s.path,
                  scope: s.scope,
                  body: `# ${s.path}\n# 훅: ${props.chain!.hooks.filter((h) => h.scope === s.scope).map((h) => h.event).join("·") || "없음"}\n`,
                  hasRaw: false,
                })}
              >
                <span class="cfi">🛡</span>
                <div><div class="cfp">{s.path}</div>
                  <div class="cfc">권한 · 훅 {props.chain!.hooks.filter((h) => h.scope === s.scope).map((h) => h.event).join("·") || "없음"}</div></div>
                <span class="cfx">{s.scope}</span>
              </div>
            )}
          </For>
          <Show when={props.chain!.env_keys.length > 0}>
            <div
              class="cfgrow"
              onClick={() => props.onOpenEditor({
                title: "환경변수 (값 마스킹)",
                path: "env",
                scope: "env",
                body: `# 환경변수 (값은 마스킹)\n${props.chain!.env_keys.map((k) => `${k}=••••••••`).join("\n")}\n`,
                hasRaw: false,
              })}
            >
              <span class="cfi">🔑</span>
              <div><div class="cfp">env ({props.chain!.env_keys.length})</div>
                <div class="cfc">{props.chain!.env_keys.join(" · ")} <i>(값 마스킹)</i></div></div>
              <span class="cfx">env</span>
            </div>
          </Show>
          <Show when={props.chain!.skills.length > 0}>
            <div class="cfgrow">
              <span class="cfi">✨</span>
              <div><div class="cfp">skills ({props.chain!.skills.length})</div>
                <div class="cfc">{props.chain!.skills.join(" · ")}</div></div>
              <span class="cfx">skill</span>
            </div>
          </Show>
        </Show>
      </Show>

      <div class="apvsec">에이전트 설정</div>
      <div class="apvbtns">
        <button class="qbtn" onClick={props.onOpenChannel}>🔗 외부 채널 연동 · Discord/Telegram/Slack</button>
        <Show
          when={props.onGotoMarket}
          fallback={<button class="qbtn" onClick={() => setWalletNote((v) => !v)}>👛 예산 한도 · 공개/수익 (마켓 탭)</button>}
        >
          <button class="qbtn" onClick={() => props.onGotoMarket!()}>👛 예산 한도 · 공개/수익 (마켓 탭)</button>
        </Show>
        <Show when={walletNote()}>
          <div class="apvhint">예산·수익 화면은 하단 <b>🌐 마켓</b> 탭에 있습니다. (지갑 배지 · 공개 에이전트 수익)</div>
        </Show>
      </div>

      <div class="apvsec">빠른 작업</div>
      <div class="apvbtns">
        <button class="qbtn" onClick={props.onOpenTerm}>⌗ tmux 터미널 열기</button>
        <button class="qbtn" onClick={() => props.onGotoChat?.(props.p.alias)}>💬 이 에이전트와 대화하기</button>
      </div>
    </div>
  );
}

function ChainRow(props: {
  node: ChainNode;
  depth: number;
  onOpenEditor: (f: EditorFile) => void;
  toEditor: (n: ChainNode, title: string) => EditorFile;
}) {
  const n = () => props.node;
  const icon = () => (n().scope === "global" ? "🌐" : n().scope === "import" ? "↳" : n().scope === "agent" ? "🧩" : "📄");
  const label = () =>
    n().scope === "global" ? "전역 지침" : n().scope === "agent" ? "에이전트 지침(AGENT.md)" :
    n().scope === "import" ? "지침이 불러오는 import" : "프로젝트 지침";
  return (
    <Show when={n().exists || props.depth === 0}>
      <div
        class={`cfgrow${n().exists ? "" : " miss"}${props.depth > 0 ? " imp" : ""}`}
        onClick={() => { if (n().exists) props.onOpenEditor(props.toEditor(n(), label())); }}
      >
        <span class="cfi">{icon()}</span>
        <div>
          <div class="cfp">{n().raw ? n().raw : n().path}</div>
          <div class="cfc">{label()}{n().exists ? "" : " · 없음"}{n().bytes ? ` · ${n().bytes}B` : ""}</div>
        </div>
        <span class="cfx">{n().scope}</span>
      </div>
      <Show when={n().imports}>
        <For each={n().imports}>
          {(child) => <ChainRow node={child} depth={props.depth + 1} onOpenEditor={props.onOpenEditor} toEditor={props.toEditor} />}
        </For>
      </Show>
    </Show>
  );
}

// ── 외부 채널 연동 (목업 channelOvl) — bindings_status 실데이터, 이 에이전트(agent_id) 필터.
function ChannelOverlay(props: { alias: string; onClose: () => void }) {
  const [data] = createResource(async () => {
    const resp = await invoke<{ bindings: BindingStatus[] }>("bindings_status");
    return (resp.bindings || []).filter((b) => b.agent_id === props.alias);
  });
  const statusLabel: Record<string, string> = {
    up_to_date: "● 연결됨", pending_echo: "● 연결됨", first_setup: "● 설정 중",
    no_assistant_messages: "● 대기", no_match: "○ 미매칭",
  };
  const isOn = (s: string) => s !== "no_match";
  return (
    <div class="ax-ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="ax-board">
        <div class="ax-bh">
          <h2>🔗 {props.alias} · 외부 채널 연동</h2>
          <span class="ax-sub">이 에이전트로 들어오는 채널</span>
          <span class="ax-bx" onClick={props.onClose}>✕</span>
        </div>
        <div class="ax-bb">
          <div class="ax-note">
            ⚙️ <b>OpenXgram 서버(데몬)가 직접 처리합니다.</b> 에이전트가 MCP로 보내는 방식이 아니라,
            데몬이 채널을 직접 물고 수신·파일저장·라우팅·회신합니다 — <b>이 에이전트가 꺼져 있어도 동작</b>합니다.
          </div>
          <div class="ax-wsec">이 에이전트에 연결된 채널</div>
          <Show when={!data.loading} fallback={<div class="ax-empty">불러오는 중…</div>}>
            <Show
              when={(data() ?? []).length > 0}
              fallback={<div class="ax-empty">연결된 외부 채널이 없습니다.<br />채널은 데몬 설정(봇 토큰·웹훅)에서 바인딩됩니다.</div>}
            >
              <For each={data()!}>
                {(b) => {
                  const v = channelVisual(b.platform);
                  return (
                    <div class="ax-chcard">
                      <div class="ax-chtop">
                        <div class={`ax-chav ${v.cls}`}>{v.icon}</div>
                        <div class="ax-chn">{b.platform} · {b.bot_label || b.bot_alias || b.channel_ref.slice(0, 16)}</div>
                        <span class={`ax-chs${isOn(b.match_status) ? "" : " off"}`}>{statusLabel[b.match_status] || b.match_status}</span>
                      </div>
                      <div class="ax-chbind">
                        바인딩 <span class="ax-ar">→</span> <span class="ax-bt">📁 {props.alias}</span>
                        <Show when={b.matched_session_count > 0}><span class="ax-sub">· 세션 {b.matched_session_count}</span></Show>
                      </div>
                      <div class="ax-chdir">
                        ↕ 지시 받기 + 결과 보내기 (양방향)
                        <Show when={b.latest_preview}> · 최근: {b.latest_preview!.slice(0, 60)}</Show>
                      </div>
                    </div>
                  );
                }}
              </For>
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
}

// ── tmux 라이브 터미널 미리보기 (목업 termOvl) — session_screen 실데이터, 3초 폴링.
function TerminalOverlay(props: { alias: string; onClose: () => void }) {
  const [content, setContent] = createSignal("");
  const [note, setNote] = createSignal("");
  const [fetchedAt, setFetchedAt] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);
  let dead = false;
  let timer: number | undefined;

  // ANSI escape 제거 (Messenger.tsx 동일 패턴).
  const stripAnsi = (s: string) =>
    s
      // eslint-disable-next-line no-control-regex
      .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "")
      // eslint-disable-next-line no-control-regex
      .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "")
      // eslint-disable-next-line no-control-regex
      .replace(/\x1b[@-Z\\-_]/g, "");

  async function refresh() {
    try {
      const dto = await invoke<SessionScreenDto>("session_screen", { identifier: props.alias });
      setContent(stripAnsi(dto.content || ""));
      setNote(dto.source_note || "");
      setFetchedAt(dto.fetched_at || "");
      setError(null);
      setLoading(false);
    } catch (e) {
      setLoading(false);
      const msg = String(e);
      setError(msg);
      if (/not.?found|404|no such|exist/i.test(msg)) {
        dead = true;
        if (timer) { clearInterval(timer); timer = undefined; }
      }
    }
  }
  refresh();
  timer = window.setInterval(() => { if (!dead) refresh(); }, 3000);
  onCleanup(() => { if (timer) clearInterval(timer); });

  return (
    <div class="ax-ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="ax-term">
        <div class="ax-term-h">
          <span class="ax-tl r" /><span class="ax-tl y" /><span class="ax-tl g" />
          <span class="ax-tt">tmux: <b>{props.alias}</b> · 라이브</span>
          <span class="ax-tx" onClick={props.onClose}>✕</span>
        </div>
        <pre>
          <Show when={loading()} fallback={
            <Show
              when={!error()}
              fallback={<span style="color:#f85149;">세션 없음 — {error()}</span>}
            >
              <Show when={content()} fallback={<span style="color:#8b949e;">(빈 화면)</span>}>{content()}</Show>
            </Show>
          }>
            <span style="color:#8b949e;">불러오는 중…</span>
          </Show>
        </pre>
        <div class="ax-tmeta">
          <Show when={note()}>{note()} · </Show>
          <Show when={fetchedAt()} fallback={"라이브 폴링 3초"}>{fetchedAt()} · 라이브 폴링 3초</Show>
        </div>
      </div>
    </div>
  );
}

// ── 지침/MCP 편집 모달 (목업 edOvl).
// item 2: 탐지된 지침/설정 파일을 fs_file_get 으로 로드 → textarea 편집 → fs_file_put 으로 저장.
// 화이트리스트(CLAUDE.md/AGENTS.md/GEMINI.md/settings*.json/.mcp.json/*.md) 밖이면 403 → 명시적 안내.
// path 없거나 env/skill 처럼 가짜 경로면 편집 불가(읽기전용 요약)로 분기.
function EditorOverlay(props: { file: EditorFile; onClose: () => void }) {
  // env(값 마스킹)·MCP 요약·skills 등은 실제 디스크 파일이 아니므로 편집 비대상.
  const editable = () => {
    const p = props.file.path || "";
    if (!p || p === "env") return false;
    if (props.file.scope === "env" || props.file.scope === "skill") return false;
    return true;
  };

  const [body, setBody] = createSignal("");
  const [loading, setLoading] = createSignal(true);
  const [loadErr, setLoadErr] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  // 저장 결과: null=없음 / {ok:true, bytes} / {ok:false, msg, forbidden}
  const [saveMsg, setSaveMsg] = createSignal<{ ok: boolean; text: string } | null>(null);
  const [dirty, setDirty] = createSignal(false);

  async function load() {
    if (!editable()) {
      setBody(props.file.body);
      setLoading(false);
      return;
    }
    setLoading(true);
    setLoadErr(null);
    try {
      const r = await invoke<{ content: string }>("fs_file_get", { path: props.file.path });
      setBody(r.content ?? "");
      setDirty(false);
    } catch (e) {
      // 파일이 아직 없거나 읽기 실패 — config-chain raw 본문을 fallback 으로 채움(있으면).
      setLoadErr(String((e as Error).message || e));
      setBody(props.file.hasRaw ? props.file.body : "");
    } finally {
      setLoading(false);
    }
  }
  load();

  async function save() {
    setSaving(true);
    setSaveMsg(null);
    try {
      const r = await invoke<{ ok: boolean; path: string; bytes: number }>("fs_file_put", {
        path: props.file.path,
        content: body(),
      });
      setDirty(false);
      setSaveMsg({ ok: true, text: `저장됨 · ${r.bytes ?? body().length}B · ${r.path || props.file.path}` });
    } catch (e) {
      const msg = String((e as Error).message || e);
      // 화이트리스트 밖이면 데몬이 403 반환 → invoke 가 "HTTP 403: ..." throw.
      if (/\b403\b|forbidden|whitelist|화이트리스트/i.test(msg)) {
        setSaveMsg({
          ok: false,
          text: "❌ 저장 거부됨 (403) — 이 경로는 편집 허용 목록 밖입니다. " +
            "허용: CLAUDE.md · AGENTS.md · GEMINI.md · settings*.json · .mcp.json · *.md (프로젝트 폴더 또는 ~/.claude 내부).",
        });
      } else {
        setSaveMsg({ ok: false, text: `❌ 저장 실패 — ${msg}` });
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <div class="ax-ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="ax-editor">
        <div class="ax-eh">
          <span class="ax-ei">{props.file.title}</span>
          <span class="ax-ep">{props.file.path}</span>
          <span class="ax-ex" onClick={props.onClose}>✕</span>
        </div>
        <Show
          when={editable()}
          fallback={
            <textarea
              spellcheck={false}
              readOnly
              value={`${props.file.body}\n\n(이 항목은 실제 편집 가능한 파일이 아닙니다 — 경로·요약만 표시.)`}
            />
          }
        >
          <Show when={!loading()} fallback={<textarea spellcheck={false} readOnly value="불러오는 중…" />}>
            <textarea
              spellcheck={false}
              value={body()}
              onInput={(e) => { setBody(e.currentTarget.value); setDirty(true); setSaveMsg(null); }}
            />
          </Show>
        </Show>
        <Show when={loadErr()}>
          <div class="ax-esave err">⚠ 읽기 실패: {loadErr()} (저장 시 새로 생성될 수 있습니다)</div>
        </Show>
        <Show when={saveMsg()}>
          <div class={`ax-esave${saveMsg()!.ok ? " ok" : " err"}`}>{saveMsg()!.text}</div>
        </Show>
        <div class="ax-ef">
          <Show
            when={editable()}
            fallback={<span class="ax-rohint">🔒 읽기전용 — 디스크 파일 아님(요약 표시)</span>}
          >
            <span class="ax-rohint">{dirty() ? "● 저장되지 않은 변경" : "✓ 저장됨/변경 없음"}</span>
          </Show>
          <button class="c" onClick={props.onClose}>닫기</button>
          <button class="s" disabled={!editable() || saving() || loading()} onClick={save}>
            {saving() ? "저장 중…" : "저장"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── 파일 트리 (item 7·8 공용) — fs_tree 의 children 을 재귀 렌더.
// 폴더는 펼침/접힘, onPick(노드) 콜백으로 폴더 선택(추가 모달) 또는 파일 클릭(프로필) 처리.
function FsTreeNode(props: {
  node: FsNode;
  depth: number;
  onPick?: (n: FsNode) => void;
  picked?: () => string | null;
}) {
  const n = () => props.node;
  const hasChildren = () => !!(n().children && n().children!.length > 0);
  const [open, setOpen] = createSignal(props.depth < 1);
  const isPicked = () => props.picked && props.picked() === n().path;

  return (
    <div class="ax-ftnode">
      <div
        class={`ax-ftrow${n().is_dir ? " dir" : " file"}${isPicked() ? " picked" : ""}`}
        style={{ "padding-left": `${6 + props.depth * 16}px` }}
        onClick={() => {
          if (n().is_dir) {
            if (hasChildren()) setOpen((v) => !v);
            props.onPick?.(n());
          } else {
            props.onPick?.(n());
          }
        }}
      >
        <span class="ax-ftic">
          {n().is_dir ? (hasChildren() ? (open() ? "▾" : "▸") : "▪") : "·"}
        </span>
        <span class="ax-ftem">{n().is_dir ? "📁" : "📄"}</span>
        <span class="ax-ftnm">{n().name}</span>
      </div>
      <Show when={n().is_dir && open() && hasChildren()}>
        <For each={n().children}>
          {(c) => <FsTreeNode node={c} depth={props.depth + 1} onPick={props.onPick} picked={props.picked} />}
        </For>
      </Show>
    </div>
  );
}

// 루트 경로 → fs_tree 호출 + 렌더 래퍼. 에러/로딩 명시 표시.
function FileTree(props: {
  rootPath: string;
  depth?: number;
  onPick?: (n: FsNode) => void;
  picked?: () => string | null;
}) {
  const [tree] = createResource(
    () => props.rootPath,
    (path) => invoke<FsNode>("fs_tree", { path, depth: props.depth ?? 2 }),
  );
  return (
    <Show when={!tree.loading} fallback={<div class="ax-ftmsg">파일 트리 불러오는 중…</div>}>
      <Show
        when={!tree.error && tree()}
        fallback={<div class="ax-ftmsg err">⚠ 트리 로드 실패: {String((tree.error as Error)?.message || tree.error || "경로 없음")}</div>}
      >
        <div class="ax-fttree">
          <FsTreeNode node={tree()!} depth={0} onPick={props.onPick} picked={props.picked} />
        </div>
      </Show>
    </Show>
  );
}
