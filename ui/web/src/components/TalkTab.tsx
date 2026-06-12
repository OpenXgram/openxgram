import { createSignal, createResource, createMemo, createEffect, For, Show } from "solid-js";
import { invoke } from "../api/client";
import { AcpConversation, aiTypeToAdapter, type AcpPreset } from "./AcpConversation";
import { AddAgentModal } from "./AddAgentModal";

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

// sessions 라우트(SessionsDto) — Messenger.tsx 와 동일 contract. 이 에이전트의 tmux 세션·워크트리 소스.
interface DetectedSession {
  kind: "tmux" | "claude_project" | "xgram_session";
  identifier: string;
  display: string;
  status: "active" | "attached" | "detached" | "stale";
  windows: number | null;
  attached: boolean | null;
  created_at: string | null;
  last_active_at: string | null;
  agent_id: string | null;
  // rc.228 — 세션에 nested 된 git worktree (path/branch). 패널 워크트리 섹션 소스.
  worktrees?: { path: string; branch?: string | null }[];
}
interface SessionsDto {
  machine: { hostname: string; alias: string; tailscale_ip: string | null };
  sessions: DetectedSession[];
}

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
  const [agents, { refetch: refetchAgents }] = createResource<AgentRow[]>(() => invoke("agents_list"));
  const [addOpen, setAddOpen] = createSignal(false);
  // 상세 패널 "세션 재시작" 트리거 — 증가시키면 AcpConversation 이 세션을 닫고 재구동.
  const [restartTick, setRestartTick] = createSignal(0);
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

  // 분류 그룹화 — AgentsTab 와 동일 분류 키. pinned 는 별도 소스가 없어 비활성(빈 그룹 자동 숨김).
  const grouped = createMemo(() => {
    const by: Record<string, AgentRow[]> = { primary: [], pinned: [], project: [], special: [] };
    for (const a of agents() ?? []) {
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

  const selAgent = createMemo(() => (agents() ?? []).find((a) => a.alias === selected()) ?? null);

  // 선택 에이전트 → ACP preset(어댑터/cwd/실행모드/라벨). 우측 대화방을 ACP 세션으로 구동.
  //   adapter   = ai_type 매핑(claude→claude-agent-acp, codex→codex-acp, gemini→gemini, 그 외 기본)
  //   cwd       = project_path (없으면 daemon 기본)
  //   execMode  = execution_mode (없으면 on_demand)
  const acpPreset = createMemo<AcpPreset | null>(() => {
    const a = selAgent();
    if (!a) return null;
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

  // 선택 에이전트의 tmux 세션 — sessions 라우트에서 agent_id 또는 display/identifier 가 alias 와 매칭되는 것.
  //   tmux kind 만(목업 "실행 중 tmux"). 매칭 데이터 없으면 빈 배열 → 패널은 빈 상태 힌트 렌더.
  const selSessions = createMemo<DetectedSession[]>(() => {
    const alias = (selected() ?? "").toLowerCase();
    if (!alias) return [];
    const all = sessions()?.sessions ?? [];
    return all.filter((s) => {
      if (s.kind !== "tmux") return false;
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
    // 대화 열람 = 읽음 처리(백엔드 기록만). refetchAgents 호출 금지 — 로스터 전체 재렌더 +
    // 안읽음 클리어 재정렬로 선택 에이전트가 스크롤에서 사라지던 버그. 배지는 다음 자연 갱신에 반영.
    void invoke("acp_conv_read", { key: alias }).catch(() => {});
  }

  // ⚡ 에이전트 미리 정하지 않고 ACP 어댑터 picker 진입(기존 경로 유지).
  function openAcp() {
    setAcpMode(true);
    setMobileChat(true);
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
                              <div class="meta">
                                <div class="nm">
                                  {agentName(a)}
                                  <Show when={tagLabel(a)}><span class="tag">{tagLabel(a)}</span></Show>
                                  <Show when={a.is_public}><span class="tag">공개</span></Show>
                                </div>
                                {/* 에이전트명(ID) — AI종류는 .nm 태그, 역할은 미리보기에 이미 표시되므로 중복 제거. */}
                                <div class="kk-card-sub">
                                  <span class="kk-card-alias" title="에이전트명(ID)">@{a.alias}</span>
                                </div>
                                {/* 최근/읽지 않은 메시지 미리보기 (없으면 역할·설명) */}
                                <div class="st">{preview(a)}</div>
                              </div>
                              <div class="rcol">
                                <div class="time">{previewTime(a)}</div>
                                {/* 안읽음 카운트 배지 — 에이전트가 보낸 미확인 메시지 수. */}
                                <Show when={(a.unread ?? 0) > 0}>
                                  <span class="kk-unread">{(a.unread ?? 0) > 99 ? "99+" : a.unread}</span>
                                </Show>
                              </div>
                            </div>
                          );
                        }}
                      </For>
                    </Show>
                  )}
                </For>
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
                  ⌗ 상태{selSessions().length + selWorktrees().length > 0 ? ` ${selSessions().length + selWorktrees().length}` : ""}
                </span>
              )}
            />
          )}
        </Show>
      </Show>
      <Show when={!acpMode() && !acpPreset()}>
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
          onClose={() => setAddOpen(false)}
          onCreated={(alias) => {
            setAddOpen(false);
            void refetchAgents();
            setSelected(alias);
          }}
        />
      </Show>
    </div>
  );
}
