import { createSignal, createResource, createMemo, For, Show } from "solid-js";
import { invoke } from "../api/client";

// Phase 2 (B+C+D) — 에이전트 탭. 정본 디자인: _mockups/kakao-mockup.html.
// 좌: 명부(분류 그룹화) + 8단계 추가 폼. 우: 프로필(정보·실행모드·동적 설정탐지·설정).
// 백엔드: /v1/gui/agents(LEFT JOIN agent_profiles), /agent/{alias}/profile, /agent/{alias}/config-chain.

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
  return (ai && AI_COLOR[ai]) || "c-group";
}

export function AgentsTab(props: { onGotoChat?: (alias: string) => void }) {
  const [agents, { refetch: refetchAgents }] = createResource<AgentRow[]>(() => invoke("agents_list"));
  const [selected, setSelected] = createSignal<string | null>(null);
  const [showAdd, setShowAdd] = createSignal(false);

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
            {(p) => <ProfileView p={p()} chain={chain()} chainLoading={chain.loading} onExec={setExecMode} onGotoChat={props.onGotoChat} />}
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
    </div>
  );
}

function ProfileView(props: {
  p: Profile;
  chain: ConfigChain | undefined;
  chainLoading: boolean;
  onExec: (mode: string) => void;
  onGotoChat?: (alias: string) => void;
}) {
  const p = () => props.p;
  const locked = () => p().classification === "primary" || p().classification === "special";
  const lockedMode = () => (p().classification === "primary" ? "always" : "heartbeat");

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
          <For each={props.chain!.instruction_chain}>{(node) => <ChainRow node={node} depth={0} />}</For>
          <Show when={props.chain!.mcp_servers.length > 0}>
            <div class="cfgrow">
              <span class="cfi">🔌</span>
              <div><div class="cfp">{props.chain!.mcp_source || ".mcp.json"}</div>
                <div class="cfc">MCP 서버: {props.chain!.mcp_servers.join(" · ")}</div></div>
              <span class="cfx">MCP</span>
            </div>
          </Show>
          <For each={props.chain!.settings_files.filter((s) => s.exists)}>
            {(s) => (
              <div class="cfgrow">
                <span class="cfi">🛡</span>
                <div><div class="cfp">{s.path}</div>
                  <div class="cfc">권한 · 훅 {props.chain!.hooks.filter((h) => h.scope === s.scope).map((h) => h.event).join("·") || "없음"}</div></div>
                <span class="cfx">{s.scope}</span>
              </div>
            )}
          </For>
          <Show when={props.chain!.env_keys.length > 0}>
            <div class="cfgrow">
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

      <div class="apvsec">에이전트 설정 <span class="auto">(Phase 5·6에서 배선)</span></div>
      <div class="apvbtns">
        <button class="qbtn" disabled>👛 예산 한도 · 외부 채널 연동 · 공개/수익 (Phase 6)</button>
      </div>

      <div class="apvsec">빠른 작업</div>
      <div class="apvbtns">
        <button class="qbtn" onClick={() => props.onGotoChat?.(props.p.alias)}>💬 이 에이전트와 대화하기</button>
      </div>
    </div>
  );
}

function ChainRow(props: { node: ChainNode; depth: number }) {
  const n = () => props.node;
  const icon = () => (n().scope === "global" ? "🌐" : n().scope === "import" ? "↳" : n().scope === "agent" ? "🧩" : "📄");
  const label = () =>
    n().scope === "global" ? "전역 지침" : n().scope === "agent" ? "에이전트 지침(AGENT.md)" :
    n().scope === "import" ? "지침이 불러오는 import" : "프로젝트 지침";
  return (
    <Show when={n().exists || props.depth === 0}>
      <div class={`cfgrow${n().exists ? "" : " miss"}${props.depth > 0 ? " imp" : ""}`}>
        <span class="cfi">{icon()}</span>
        <div>
          <div class="cfp">{n().raw ? n().raw : n().path}</div>
          <div class="cfc">{label()}{n().exists ? "" : " · 없음"}{n().bytes ? ` · ${n().bytes}B` : ""}</div>
        </div>
        <span class="cfx">{n().scope}</span>
      </div>
      <Show when={n().imports}>
        <For each={n().imports}>{(child) => <ChainRow node={child} depth={props.depth + 1} />}</For>
      </Show>
    </Show>
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
