import { createSignal, createResource, createMemo, createEffect, For, Show } from "solid-js";
import { invoke } from "../api/client";
import { AcpConversation, aiTypeToAdapter, type AcpPreset } from "./AcpConversation";

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
}

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

export function TalkTab(props: { onJumpToSettings?: () => void }) {
  const [agents] = createResource<AgentRow[]>(() => invoke("agents_list"));
  const [peers] = createResource<PeerDto[]>(() => invoke("peers_list"), { initialValue: [] });
  const [recent] = createResource<MessageDto[]>(() => invoke("messages_recent", { limit: 100 }), { initialValue: [] });
  // 정보 패널 소스 — sessions(이 머신 tmux+워크트리) · workflows(orchestrator 매칭). 동적 only.
  const [sessions] = createResource<SessionsDto | null>(() => invoke("sessions"), { initialValue: null });
  const [workflows] = createResource<WorkflowDto[]>(() => invoke("workflows_list"), { initialValue: [] });

  const [selected, setSelected] = createSignal<string | null>(null);
  const [mobileChat, setMobileChat] = createSignal(false);
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
    for (const k of Object.keys(by)) {
      by[k].sort((x, y) => onl(y) - onl(x) || ts(y) - ts(x));
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
      label: a.alias,
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
          <button class="add-btn" onClick={() => props.onJumpToSettings?.()}>＋ 에이전트 추가</button>
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
                                  {a.alias}
                                  <Show when={tagLabel(a)}><span class="tag">{tagLabel(a)}</span></Show>
                                  <Show when={a.is_public}><span class="tag">공개</span></Show>
                                </div>
                                <div class="st">{preview(a)}</div>
                              </div>
                              <div class="rcol">
                                <div class="time">{previewTime(a)}</div>
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
              onClose={() => setMobileChat(false)}
            />
          )}
        </Show>
      </Show>
      <Show when={!acpMode() && !acpPreset()}>
        <div class="kk-talk-chat">
          <div class="kk-talk-blank">좌측에서 대화할 에이전트를 선택하세요.</div>
        </div>
      </Show>

      {/* ── 정보 패널 토글 — 에이전트 선택 시(ACP 대화 중) 우상단 pill. peer chat-top 제거로 별도 트리거 필요. ── */}
      <Show when={selAgent() && !acpMode()}>
        <span class="kk-info-toggle pill clk" onClick={() => setInfoOpen((v) => !v)}>
          ⌗ 상태 {selSessions().length + selWorktrees().length > 0 ? `(${selSessions().length + selWorktrees().length})` : ""}
        </span>
      </Show>

      {/* ── 정보 사이드 패널 (정본 #info) — 선택 에이전트의 폴더·tmux·워크트리·워크플로우 ── */}
      <Show when={selAgent()}>
        {(a) => (
          <div class={`info${infoOpen() ? " show" : ""}`}>
            <div class="info-head">
              <span class="t">{a().alias} · 상태</span>
              <span class="x" onClick={() => setInfoOpen(false)}>✕</span>
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
                    <div class="sess">
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
    </div>
  );
}
