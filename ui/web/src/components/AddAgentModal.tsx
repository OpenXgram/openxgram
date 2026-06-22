import { createSignal, createResource, Show, For } from "solid-js";
import { invoke, acpFetch } from "@/api/client";

// fs/tree 노드 — 디렉토리 트리 선택용.
type FsNode = { name: string; path: string; is_dir?: boolean; children?: FsNode[] };
// fs/tree roots 모드 응답 (path 빈값/__roots__) — OS 별 최상위 루트.
// windows: 드라이브 + \\wsl$\ 공유, unix: $HOME + /.
type FsRoots = { os: "windows" | "unix"; is_roots: true; roots: FsNode[] };

// 재귀 트리 노드 — 디렉토리만 표시. 이름 클릭=선택, ▸/▾=펼침.
function TreeNode(p: {
  node: FsNode;
  depth: number;
  expanded: () => Set<string>;
  onToggle: (path: string) => void;
  onSelect: (path: string) => void;
  selected: () => string;
}) {
  const kids = () => (p.node.children ?? []).filter((c) => c.is_dir !== false);
  const isOpen = () => p.expanded().has(p.node.path);
  return (
    <div class="ft-node">
      <div
        class={`ft-row${p.selected() === p.node.path ? " sel" : ""}`}
        style={{ "padding-left": `${p.depth * 14}px` }}
      >
        <span class="ft-tw" onClick={() => p.onToggle(p.node.path)}>
          {kids().length ? (isOpen() ? "▾" : "▸") : "·"}
        </span>
        <span class="ft-name" onClick={() => p.onSelect(p.node.path)} title={p.node.path}>
          📁 {p.node.name}
        </span>
      </div>
      <Show when={isOpen()}>
        <For each={kids()}>
          {(c) => (
            <TreeNode
              node={c}
              depth={p.depth + 1}
              expanded={p.expanded}
              onToggle={p.onToggle}
              onSelect={p.onSelect}
              selected={p.selected}
            />
          )}
        </For>
      </Show>
    </div>
  );
}

// 에이전트 추가 모달 — 목업 정본(머신·폴더·AI종류·이름·역할·분류·그룹·실행모드·워크트리·공개).
// POST agents_register → agent_capabilities + agent_profiles 둘 다 기록(이게 있어야 로스터에 노출).
// 만들면 바로 대화방. onCreated(alias) 로 부모가 새로고침 + 선택.

// AI 종류 = 데몬이 검증하는 ai_type enum 과 정확히 일치해야 한다.
//   register/profile 저장 시 validate_profile_enums 가 claude|codex|gemini 만 허용하고
//   (daemon_gui.rs validate_profile_enums / mcp_serve.rs), 세션 생성 시 이 값이
//   레지스트리 어댑터로 매핑된다(daemon_gui.rs: claude→claude-agent-acp,
//   codex→codex-acp, gemini→gemini). 과거 'ollama'/'hermes' 는 어느 레이어에도
//   존재하지 않아 등록이 항상 실패했다 → 제거(드리프트 방지).
const AI_TYPES: { v: string; label: string }[] = [
  { v: "claude", label: "claude" },
  { v: "codex", label: "codex" },
  { v: "gemini", label: "gemini" },
  { v: "opencode", label: "opencode" },
];
const PERM_OPTS = [
  { v: "acceptEdits", label: "기본 (acceptEdits)" },
  { v: "bypassPermissions", label: "모든 권한 (Bypass)" },
  { v: "plan", label: "계획만 (plan)" },
];
const CLASS_OPTS = [
  { v: "project", label: "📁 프로젝트 에이전트" },
  { v: "special", label: "⚙️ 특수 기능 에이전트" },
  { v: "primary", label: "⭐ 통합관리 (프라이머리)" },
];

export function AddAgentModal(props: { onClose: () => void; onCreated: (alias: string) => void; prefillFolder?: string | null }) {
  // rc.322 — 로컬 에이전트 추가는 '현재 머신'에만 의미 있다(잘만 GUI 에서 서울
  //   에이전트를 만들 수 없음). 그래서 cross-machine 선택 dropdown 을 제거하고
  //   현재 머신을 sessions 라우트(machine.alias / hostname — TalkTab·Messenger 가
  //   self 머신을 얻는 동일 출처)에서 읽어 read-only 로 표시한다.
  const [selfMachine] = createResource<string>(
    async () => {
      try {
        const r = await invoke<{ machine?: { alias?: string; hostname?: string } }>("sessions");
        return r?.machine?.alias?.trim() || r?.machine?.hostname?.trim() || "(이 머신)";
      } catch {
        return "(이 머신)";
      }
    },
    { initialValue: "(이 머신)" },
  );
  // 머신 = 항상 현재 머신. 표시는 read-only, agents_register 에는 이 값을 전달.
  const machine = () => selfMachine();
  // 미등록 tmux "추가" 진입 시 그 세션 cwd 를 폴더로 prefill(=project_path). 없으면 기본.
  const [folder, setFolder] = createSignal((props.prefillFolder && props.prefillFolder.trim()) || "/home/llm/projects/starian-set");
  const [aiType, setAiType] = createSignal("claude");
  const [alias, setAlias] = createSignal("");
  const [role, setRole] = createSignal("");
  const [classification, setClassification] = createSignal("project");
  const [group, setGroup] = createSignal("");
  const [execMode, setExecMode] = createSignal("on_demand"); // always | on_demand | heartbeat
  const [worktree, setWorktree] = createSignal(false);
  const [isPublic, setIsPublic] = createSignal(false);
  const [permMode, setPermMode] = createSignal("acceptEdits"); // acceptEdits | bypassPermissions | plan
  // ACP 어댑터 설치 상태 — ai_type 옵션에 ✓ 설치됨 / 미설치 표시.
  const [acpInstalled] = createResource<Record<string, boolean>>(
    async () => {
      try {
        const r = await acpFetch<{ agents?: { name: string; installed: boolean }[] }>("GET", "/agents");
        const map: Record<string, boolean> = {};
        for (const a of r?.agents || []) map[a.name] = a.installed;
        // adapter 이름 → ai_type 매핑
        return { claude: map["claude-agent-acp"] ?? false, codex: map["codex-acp"] ?? false, gemini: map["gemini"] ?? false, opencode: map["opencode"] ?? false };
      } catch { return {}; }
    },
    { initialValue: {} },
  );
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  // 폴더 트리 선택기 — 해당 머신(데몬)의 디렉토리 트리에서 프로젝트 폴더 선택.
  // 시작 루트는 데몬 OS 에 맞게 roots 모드(path 빈값)로 동적 해석 — Linux 경로
  //   하드코딩 금지(Windows 데몬에서 /home/llm 은 존재하지 않아 picker 가 빈다).
  const [treeOpen, setTreeOpen] = createSignal(false);
  const [treeRoot, setTreeRoot] = createSignal("");
  const [tree, setTree] = createSignal<FsNode | null>(null);
  const [treeErr, setTreeErr] = createSignal<string | null>(null);
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());
  // roots 모드로 데몬 OS 의 최상위 루트들을 받아 첫 루트를 시작점으로 반환.
  //   여러 루트(Windows 드라이브 등)면 첫 루트로 진입하고, 사용자는 상단 입력바로
  //   다른 루트(예: D:\ 또는 \\wsl$\Ubuntu)로 이동 가능.
  async function resolveStartRoot(): Promise<string | null> {
    // 로컬 데몬 fs 브라우징 — machine 은 비워 보낸다(self-hostname 을 보내면 백엔드가
    //   원격으로 취급해 roots 모드를 건너뛰고 "path 필요" 400 이 난다). 폴더 선택은 항상 로컬.
    const r = await invoke<FsRoots | FsNode>("fs_tree", { path: "", depth: 1, machine: "" });
    if (r && (r as FsRoots).is_roots) {
      const roots = (r as FsRoots).roots ?? [];
      return roots.length ? roots[0].path : null;
    }
    // 원격 머신 등 roots 미지원 응답이면 그 노드 path 를 시작점으로.
    return (r as FsNode)?.path ?? null;
  }
  async function loadTree(root: string) {
    setTreeErr(null);
    try {
      // root 가 비면 데몬 OS 의 시작 루트를 동적 해석(하드코딩 제거).
      const start = root && root.trim() ? root : await resolveStartRoot();
      if (!start) {
        setTreeErr("데몬에서 시작 폴더(루트)를 찾지 못했습니다.");
        setTree(null);
        return;
      }
      // 선택 머신 전달 — 원격 머신이면 데몬이 SSH 로 그 머신 디렉토리를 조회.
      const t = await invoke<FsNode>("fs_tree", { path: start, depth: 4, machine: "" });
      setTree(t);
      setTreeRoot(t.path); // 원격 HOME fallback 등으로 root 가 바뀌면 입력에도 반영.
      setExpanded(new Set([t.path])); // 루트 펼침.
    } catch (e) {
      setTreeErr((e as Error)?.message ?? String(e));
      setTree(null);
    }
  }
  async function openTree() {
    setTreeOpen(true);
    if (!tree()) await loadTree(treeRoot());
  }
  function toggleNode(path: string) {
    const s = new Set(expanded());
    if (s.has(path)) s.delete(path);
    else s.add(path);
    setExpanded(s);
  }
  function pickFolder(path: string) {
    setFolder(path);
    setTreeOpen(false);
  }

  async function create() {
    const name = alias().trim();
    if (!name) {
      setErr("이름(alias)을 입력하세요.");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await invoke("agents_register", {
        alias: name,
        role: role().trim() || null,
        description: role().trim() || null,
        project_path: folder().trim() || null,
        group_name: group().trim() || null,
        messenger_enabled: true, // 실제 에이전트 = 키페어 + peer 등록.
        ai_type: aiType(),
        classification: classification(),
        execution_mode: execMode(),
        machine: machine(),
        worktree: worktree() ? folder().trim() || null : null,
        is_public: isPublic(),
        perm_mode: permMode(),
      });
      props.onCreated(name);
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="modal">
        <h2>에이전트 추가</h2>
        <p class="sub">만들면 바로 대화방이 생깁니다.</p>

        <div class="mrow">
          <div class="fld">
            <label>1 · 머신</label>
            {/* rc.322 — 로컬 추가는 현재 머신 전용. 선택 불가(read-only). */}
            <input class="ctl" value={machine()} readonly disabled title={`${machine()} — 로컬 에이전트는 현재 머신에만 추가됩니다`} />
          </div>
          <div class="fld">
            <label>3 · AI 종류</label>
            <select class="ctl" value={aiType()} onChange={(e) => setAiType(e.currentTarget.value)}>
              {AI_TYPES.map((t) => <option value={t.v}>{t.label}{acpInstalled()[t.v] === false ? " · 미설치" : " ✓"}</option>)}
            </select>
            <Show when={acpInstalled()[aiType()] === false}>
              <div style="font-size:11px; color:#d29922; margin-top:3px;">⚠ 이 어댑터는 미설치 — 선택해도 ACP 구동이 안 됩니다 (설치 필요).</div>
            </Show>
          </div>
        </div>

        <div class="fld">
          <label>2 · 프로젝트 폴더</label>
          <div style="display:flex; gap:7px;">
            <input class="ctl" style="flex:1;" value={folder()} onInput={(e) => setFolder(e.currentTarget.value)} placeholder="/home/llm/projects/..." />
            <button class="btn-q" type="button" onClick={() => void openTree()}>📁 찾기</button>
          </div>
          <Show when={treeOpen()}>
            <div class="ft-panel">
              <div class="ft-rootbar">
                <input
                  class="ctl"
                  style="flex:1; font-size:12px;"
                  value={treeRoot()}
                  onInput={(e) => setTreeRoot(e.currentTarget.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") void loadTree(treeRoot()); }}
                />
                <button class="btn-q" type="button" onClick={() => void loadTree(treeRoot())}>이동</button>
                <button class="btn-q" type="button" onClick={() => setTreeOpen(false)}>✕</button>
              </div>
              <Show when={treeErr()}>
                <div style="color:#ff6b6b; font-size:11.5px; padding:6px;">⚠ {treeErr()}</div>
              </Show>
              <div class="ft-scroll">
                <Show when={tree()} fallback={<div style="padding:8px; color:#9aa1ad; font-size:12px;">불러오는 중…</div>}>
                  <TreeNode
                    node={tree()!}
                    depth={0}
                    expanded={expanded}
                    onToggle={toggleNode}
                    onSelect={pickFolder}
                    selected={folder}
                  />
                </Show>
              </div>
              <div class="ft-hint">폴더 이름을 클릭하면 선택됩니다. ▸ 로 하위 폴더 펼치기.</div>
            </div>
          </Show>
        </div>

        <div class="mrow">
          <div class="fld">
            <label>4 · 이름</label>
            <input class="ctl" value={alias()} onInput={(e) => setAlias(e.currentTarget.value)} placeholder="akashic" />
          </div>
          <div class="fld">
            <label>6 · 분류</label>
            <select class="ctl" value={classification()} onChange={(e) => setClassification(e.currentTarget.value)}>
              {CLASS_OPTS.map((c) => <option value={c.v}>{c.label}</option>)}
            </select>
          </div>
        </div>

        <div class="mrow">
          <div class="fld">
            <label>5 · 역할 <span class="opt">(선택)</span></label>
            <input class="ctl" value={role()} onInput={(e) => setRole(e.currentTarget.value)} placeholder="작업 정리 · SNS 포스팅" />
          </div>
          <div class="fld">
            <label>그룹 <span class="opt">(선택)</span></label>
            <input class="ctl" value={group()} onInput={(e) => setGroup(e.currentTarget.value)} placeholder="배포팀" />
          </div>
        </div>

        <div class="fld">
          <label>실행 모드</label>
          <div class="seg">
            <div class={`s${execMode() === "always" ? " on" : ""}`} onClick={() => setExecMode("always")}>⚡ 상시 켜둠</div>
            <div class={`s${execMode() === "on_demand" ? " on" : ""}`} onClick={() => setExecMode("on_demand")}>🕓 필요할 때</div>
            <div class={`s${execMode() === "heartbeat" ? " on" : ""}`} onClick={() => setExecMode("heartbeat")}>😴 하트비트</div>
          </div>
        </div>

        <div class="mrow">
          <div class="fld">
            <label>7 · 워크트리</label>
            <div class="seg">
              <div class={`s${worktree() ? " on" : ""}`} onClick={() => setWorktree(true)}>사용</div>
              <div class={`s${!worktree() ? " on" : ""}`} onClick={() => setWorktree(false)}>안 함</div>
            </div>
          </div>
          <div class="fld">
            <label>8 · 공개 (OpenAgentX)</label>
            <div class="seg">
              <div class={`s${!isPublic() ? " on" : ""}`} onClick={() => setIsPublic(false)}>비공개</div>
              <div class={`s${isPublic() ? " on" : ""}`} onClick={() => setIsPublic(true)}>공개 →</div>
            </div>
          </div>
        </div>

        <div class="fld">
          <label>9 · 권한 (기본 perm_mode)</label>
          <select class="ctl" value={permMode()} onChange={(e) => setPermMode(e.currentTarget.value)}>
            {PERM_OPTS.map((p) => <option value={p.v}>{p.label}</option>)}
          </select>
          <div style="font-size:11px; color:var(--kk-sub,#8a929b); margin-top:3px;">Bypass=모든 권한(도구 무확인) · acceptEdits=편집 자동수락(기본) · plan=계획만(실행 안 함)</div>
        </div>

        <Show when={err()}>
          <div style="color:#ff6b6b;font-size:12px;margin:6px 0;">⚠ {err()}</div>
        </Show>

        <div class="modal-foot">
          <button class="btn-q" onClick={() => props.onClose()} disabled={busy()}>취소</button>
          <button class="btn-go" onClick={() => void create()} disabled={busy()}>{busy() ? "만드는 중…" : "만들기"}</button>
        </div>
        <div class="hint">머신·tailscale·토큰은 이 안에서만 — 만든 뒤엔 에이전트 목록에 '이름'만 보입니다.</div>
      </div>
    </div>
  );
}
