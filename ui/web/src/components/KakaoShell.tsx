import { createSignal, createResource, createMemo, createEffect, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./mockup.css";
import { AcpConversation, aiTypeToAdapter, type AcpPreset } from "./AcpConversation";
import { A2AMiniPanel } from "./A2AMiniPanel";
import { RoomModal } from "./RoomModal";
import { AddAgentModal } from "./AddAgentModal";
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
interface PeerDto { alias: string; last_seen?: string; machine?: string }
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

  const [selected, setSelected] = createSignal<string | null>(null);
  const [search, setSearch] = createSignal("");
  // 곁뷰: a2a(협업) / tmux(작업환경) — 상호배타. 목업 .side 두 패널.
  const [sideA2A, setSideA2A] = createSignal(false);
  const [sideTmux, setSideTmux] = createSignal(false);
  const [roomCfgOpen, setRoomCfgOpen] = createSignal(false);
  const [addOpen, setAddOpen] = createSignal(false);

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

  const rooms = createMemo<AgentRow[]>(() => {
    const q = search().trim().toLowerCase();
    const list = (agents() ?? []).filter((a) => {
      if (!q) return true;
      return [a.alias, a.display_name, a.role, a.machine, a.ai_type].some((f) => (f ?? "").toLowerCase().includes(q));
    });
    // 프라이머리 먼저, 그다음 마지막 메시지 최신순.
    const ts = (a: AgentRow) => { const m = lastMsgByAlias().get(a.alias.toLowerCase()); return m ? Date.parse(m.timestamp) : 0; };
    return [...list].sort((x, y) => {
      if (isPrimary(x) !== isPrimary(y)) return isPrimary(x) ? -1 : 1;
      return ts(y) - ts(x);
    });
  });

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

  // ── 작업환경(tmux) 곁뷰 데이터 소스 — sessions 라우트(이 머신 tmux+워크트리). 동적 only.
  //   TalkTab 정보 패널과 동일 contract: 선택 에이전트의 cwd(project_path) 매칭 + alias 보조.
  const [sessions] = createResource<SessionsDto | null>(() => invoke("sessions"), { initialValue: null });

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
    w.focus();
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
        <h2>채팅</h2>
        <input class="search" type="text" value={search()} onInput={(e) => setSearch(e.currentTarget.value)} placeholder="🔍  에이전트·대화방 검색" />
        <div class="rooms">
          <Show when={!agents.loading} fallback={<div style="padding:16px;color:var(--muted);font-size:13px">불러오는 중…</div>}>
            <For each={rooms()}>
              {(a) => {
                const primary = isPrimary(a);
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
              }}
            </For>
            <div class="room" onClick={() => setAddOpen(true)} title="에이전트 추가">
              <div class="av grp">＋</div>
              <div class="meta"><div class="nm">에이전트 추가</div><div class="ms">머신 · 외부 A2A · 새 ACP</div></div>
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
                    {/* 세션 목록 — 클릭 시 라이브 화면 새 창. */}
                    <div class="kk-we-sec">
                      <div class="kk-we-sech">실행 중 tmux · {selSessions().length} <span class="kk-we-sub">(클릭 → 라이브 새 창)</span></div>
                      <For each={selSessions()}>
                        {(s) => (
                          <div class="kk-we-sess" title="클릭 → 라이브 화면 새 창" onClick={() => openTmuxPopout(s.identifier, s.display || s.identifier)}>
                            <span class="dot" />
                            <span class="nm">{s.display || s.identifier}</span>
                            <span class="sx">{s.kind}{(s.cwd ? ` · ${baseName(s.cwd)}` : "")}</span>
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

                    {/* 라이브 pane — capture-pane(다크 .term). 실제 터미널 내용이 있을 때만 다크. */}
                    <div class="kk-we-sec kk-we-sec-term">
                      <div class="kk-we-sech">
                        라이브 화면 <span class="kk-we-sub">{captureTarget() ? captureTarget()!.display || captureTarget()!.identifier : ""}</span>
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
          <button class="add" onClick={() => setAddOpen(true)}>＋ 친구(에이전트) 추가</button>
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

      {/* 에이전트 추가 모달 */}
      <Show when={addOpen()}>
        <AddAgentModal
          onClose={() => setAddOpen(false)}
          onCreated={(alias) => { setAddOpen(false); void refetchAgents(); setSelected(alias); setTab("chat"); }}
        />
      </Show>

      {/* 옛 하단 고정 캡션바(.note) 제거 — 대화 모델 캡션은 .chat-top 헤더 제목 아래 .chat-cap 로 이전. */}
    </div>
  );
}
