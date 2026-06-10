import { createSignal, createResource, createMemo, onCleanup, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./agents-extra.css";

// Phase 2 (B+C+D) — 에이전트 탭. 정본 디자인: _mockups/kakao-mockup.html.
// 좌: 명부(분류 그룹화) + 8단계 추가 폼. 우: 프로필(정보·실행모드·동적 설정탐지·설정).
// 백엔드: /v1/gui/agents(LEFT JOIN agent_profiles), /agent/{alias}/profile, /agent/{alias}/config-chain.
//
// 정본 목업의 외부채널(channelOvl)·tmux 터미널(termOvl)·지침/MCP 편집(edOvl) 3개 오버레이를
// 프로필 진입점(qbtn·cfgrow)에서 열도록 이식. 실데이터: bindings_status(채널) /
// session_screen(tmux 라이브 화면) / config-chain.raw(편집 모달 본문, 쓰기 라우트 없음 → 읽기전용).

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
}

interface Profile {
  alias: string;
  exists: boolean;
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

  const grouped = createMemo(() => {
    const list = agents() ?? [];
    const by: Record<string, AgentRow[]> = { primary: [], project: [], special: [] };
    for (const a of list) {
      const cls = a.classification && by[a.classification] ? a.classification : "project";
      by[cls].push(a);
    }
    return by;
  });

  async function setExecMode(mode: string) {
    const a = selected();
    if (!a) return;
    await invoke("agent_profile_set", { alias: a, execution_mode: mode });
    await refetchProfile();
    await refetchAgents();
  }

  return (
    <div class="kk-agents">
      <div class="kk-roster">
        <div class="rtop">
          <h2>에이전트</h2>
          <button class="kk-add" onClick={() => setShowAdd(true)}>+ 추가</button>
        </div>
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
                          {a.alias}
                          <Show when={a.ai_type}><span class="tag">{a.ai_type}</span></Show>
                          <Show when={a.is_public}><span class="tag">공개</span></Show>
                        </div>
                        <div class="kk-st">{a.role || a.description || "—"}</div>
                      </div>
                    </div>
                  )}
                </For>
              </Show>
            )}
          </For>
          <Show when={(agents() ?? []).length === 0}>
            <div class="empty">등록된 에이전트가 없습니다.<br />우측 상단 <b>+ 추가</b>로 등록하세요.</div>
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
          onClose={() => setShowAdd(false)}
          onCreated={async (alias) => {
            setShowAdd(false);
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
  onGotoChat?: (alias: string) => void;
  onGotoMarket?: () => void;
  onOpenChannel: () => void;
  onOpenTerm: () => void;
  onOpenEditor: (f: EditorFile) => void;
}) {
  const p = () => props.p;
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
        <div class="apvcard"><div class="k">AI 종류</div><div class="v">{p().ai_type}</div></div>
        <div class="apvcard"><div class="k">분류</div><div class="v">{CLASS_LABEL[p().classification] || p().classification}</div></div>
        <div class="apvcard"><div class="k">머신</div><div class="v">{p().machine || "—"}</div></div>
        <div class="apvcard"><div class="k">폴더</div><div class="v">{p().folder || "—"}</div></div>
        <div class="apvcard"><div class="k">역할</div><div class="v">{p().role || "—"}</div></div>
        <div class="apvcard"><div class="k">그룹</div><div class="v">{p().group || "—"}</div></div>
        <div class="apvcard"><div class="k">공개</div><div class="v">{p().is_public ? "🌐 공개" : "비공개"}</div></div>
        <div class="apvcard"><div class="k">워크트리</div><div class="v">{p().worktree || "—"}</div></div>
      </div>

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

// ── 지침/MCP 편집 모달 (목업 edOvl) — config-chain.raw 본문 표시. 쓰기 라우트 없음 → 읽기전용.
function EditorOverlay(props: { file: EditorFile; onClose: () => void }) {
  return (
    <div class="ax-ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="ax-editor">
        <div class="ax-eh">
          <span class="ax-ei">{props.file.title}</span>
          <span class="ax-ep">{props.file.path}</span>
          <span class="ax-ex" onClick={props.onClose}>✕</span>
        </div>
        <textarea spellcheck={false} readOnly value={props.file.hasRaw ? props.file.body : `${props.file.body}\n\n(이 파일 본문은 config-chain 에 raw 로 노출되지 않습니다. 경로·요약만 표시.)`} />
        <div class="ax-ef">
          <span class="ax-rohint">🔒 읽기전용 — 편집 백엔드(쓰기 라우트) 미연결</span>
          <button class="c" onClick={props.onClose}>닫기</button>
          <button class="s" disabled>저장 (미연결)</button>
        </div>
      </div>
    </div>
  );
}

// 8단계 추가 폼 — 머신·폴더·AI종류·이름·역할·분류·그룹·워크트리/공개.
function AddAgentModal(props: { onClose: () => void; onCreated: (alias: string) => void }) {
  const [step, setStep] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal("");
  const [d, setD] = createSignal({
    machine: "", folder: "", ai_type: "claude", name: "", role: "",
    classification: "project", group: "", worktree: "", is_public: false,
  });
  const set = (k: string, v: unknown) => setD({ ...d(), [k]: v });

  const STEPS = [
    { key: "machine", title: "머신", sub: "이 에이전트가 도는 머신 (예: 서울, zalman). 추가 폼에서만 보입니다.", field: "text", ph: "서울" },
    { key: "folder", title: "작업 폴더", sub: "프로젝트 경로 (동적 설정 탐지의 기준).", field: "text", ph: "~/projects/starian-set" },
    { key: "ai_type", title: "AI 종류", sub: "지침 체인 분기 (claude=CLAUDE.md / codex=AGENTS.md / gemini=GEMINI.md).", field: "ai" },
    { key: "name", title: "이름 (alias)", sub: "에이전트 고유 이름. 명부·라우팅 키.", field: "text", ph: "akashic" },
    { key: "role", title: "역할", sub: "한 줄 역할 설명.", field: "text", ph: "작업 정리 · SNS" },
    { key: "classification", title: "분류", sub: "명부 그룹 + 실행모드 기본값.", field: "class" },
    { key: "group", title: "그룹", sub: "협업 단위 (peer_send fan-out 대상). 선택.", field: "text", ph: "배포팀" },
    { key: "final", title: "워크트리 · 공개", sub: "git worktree(선택) + 마켓 공개 여부.", field: "final" },
  ];
  const cur = () => STEPS[step()];
  const last = () => step() === STEPS.length - 1;

  async function submit() {
    const v = d();
    if (!v.name.trim()) { setErr("이름(alias)은 필수입니다."); setStep(3); return; }
    setBusy(true); setErr("");
    const execution_mode = v.classification === "primary" ? "always" : v.classification === "special" ? "heartbeat" : "on_demand";
    try {
      await invoke("agent_profile_set", {
        alias: v.name.trim(), ai_type: v.ai_type, classification: v.classification,
        execution_mode, machine: v.machine || null, worktree: v.worktree || null,
        is_public: v.is_public, role: v.role || null, group: v.group || null, folder: v.folder || null,
      });
      props.onCreated(v.name.trim());
    } catch (e) {
      setErr(`등록 실패: ${(e as Error).message}`);
      setBusy(false);
    }
  }

  return (
    <div class="kk-ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="kk-modal">
        <h3>에이전트 추가 <span style="font-weight:600;color:#9aa1ad;font-size:13px;">({step() + 1}/{STEPS.length})</span></h3>
        <p class="ssub">{cur().title} — {cur().sub}</p>

        <Show when={cur().field === "text"}>
          <label>{cur().title}</label>
          <input
            value={(d() as Record<string, string>)[cur().key]}
            placeholder={(cur() as { ph?: string }).ph}
            onInput={(e) => set(cur().key, e.currentTarget.value)}
          />
        </Show>
        <Show when={cur().field === "ai"}>
          <label>AI 종류</label>
          <select value={d().ai_type} onChange={(e) => set("ai_type", e.currentTarget.value)}>
            <option value="claude">claude (CLAUDE.md)</option>
            <option value="codex">codex (AGENTS.md)</option>
            <option value="gemini">gemini (GEMINI.md)</option>
          </select>
        </Show>
        <Show when={cur().field === "class"}>
          <label>분류</label>
          <div class="kk-seg">
            <For each={CLASS_GROUPS}>
              {(g) => (
                <div class={`s${d().classification === g.key ? " on" : ""}`} onClick={() => set("classification", g.key)}>
                  {g.icon} {g.label}
                </div>
              )}
            </For>
          </div>
        </Show>
        <Show when={cur().field === "final"}>
          <label>워크트리 경로 (선택)</label>
          <input value={d().worktree} placeholder="wt/rc-288" onInput={(e) => set("worktree", e.currentTarget.value)} />
          <label class="chk" onClick={() => set("is_public", !d().is_public)}>
            <input type="checkbox" checked={d().is_public} onChange={(e) => set("is_public", e.currentTarget.checked)} />
            🌐 OpenAgentX 마켓에 공개
          </label>
        </Show>

        <Show when={err()}><div class="err">{err()}</div></Show>

        <div class="mrow">
          <Show when={step() > 0}>
            <button onClick={() => setStep(step() - 1)} disabled={busy()}>← 이전</button>
          </Show>
          <Show when={!last()}>
            <button class="go" onClick={() => setStep(step() + 1)}>다음 →</button>
          </Show>
          <Show when={last()}>
            <button class="go" onClick={submit} disabled={busy()}>{busy() ? "등록 중…" : "✓ 추가"}</button>
          </Show>
        </div>
      </div>
    </div>
  );
}
