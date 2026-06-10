import { createSignal, createResource, createMemo, createEffect, For, Show } from "solid-js";
import { invoke } from "../api/client";

// 대화 탭 — 카카오톡 정본 목업(_mockups/kakao-mockup.html) 충실 이식.
// 좌: 분류 그룹화 명부(👑 프라이머리 / 📌 상단 고정 / 📁 프로젝트 / ⚙️ 특수) + llm-type 아바타색
//     + 마지막 메시지 미리보기/시각(messages_recent 파생).
// 우: .chat-top(온라인/머신 pill) + .msgs(.me 그레이 말풍선 / .agent 전체폭) + Claude-Code 다크 컴포저.
// 데이터: agents_list(분류·ai_type·그룹) · peers_list(online) · messages_recent(미리보기)
//         · peer_conversation(대화) · peer_send(전송). 동적 only — 가짜 데이터 없음.

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

// Messenger.tsx isSelfSender 와 동일 규칙: self/me/user → 내 발신(오른쪽).
function isSelfSender(sender: string): boolean {
  const s = (sender || "").toLowerCase();
  if (s === "me" || s === "user") return true;
  if (s.startsWith("self:") || s.startsWith("me:")) return true;
  return false;
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

function fmtDay(iso: string): string {
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    return `${d.getFullYear()}년 ${d.getMonth() + 1}월 ${d.getDate()}일`;
  } catch {
    return "";
  }
}

// 본문이 펜스드 코드 블록을 포함하면 .code 로, peer_send/tool 한 줄이면 .toolcall 로 best-effort 분해.
type Seg =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "tool"; text: string };

function segmentBody(body: string): Seg[] {
  const out: Seg[] = [];
  const fence = /```[\w-]*\n?([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = fence.exec(body)) !== null) {
    if (m.index > last) pushText(out, body.slice(last, m.index));
    out.push({ kind: "code", text: m[1].replace(/\n$/, "") });
    last = fence.lastIndex;
  }
  if (last < body.length) pushText(out, body.slice(last));
  if (out.length === 0) pushText(out, body);
  return out;
}

// 코드 블록 밖 텍스트: tool 라인(✓/✗ 으로 시작하거나 peer_send/Bash 같은 호출 한 줄)은 .toolcall 로.
function pushText(out: Seg[], chunk: string) {
  const trimmed = chunk.replace(/^\n+|\n+$/g, "");
  if (!trimmed) return;
  const lines = trimmed.split("\n");
  let buf: string[] = [];
  const flush = () => {
    if (buf.length) { out.push({ kind: "text", text: buf.join("\n") }); buf = []; }
  };
  for (const line of lines) {
    if (/^\s*(?:[✓✗⌗]|peer_send\b|Bash\b|Tool\b)/.test(line) && line.length < 120) {
      flush();
      out.push({ kind: "tool", text: line.trim() });
    } else {
      buf.push(line);
    }
  }
  flush();
}

export function TalkTab(props: { onJumpToSettings?: () => void }) {
  const [agents] = createResource<AgentRow[]>(() => invoke("agents_list"));
  const [peers] = createResource<PeerDto[]>(() => invoke("peers_list"), { initialValue: [] });
  const [recent] = createResource<MessageDto[]>(() => invoke("messages_recent", { limit: 100 }), { initialValue: [] });

  const [selected, setSelected] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [sendErr, setSendErr] = createSignal<string | null>(null);
  const [mobileChat, setMobileChat] = createSignal(false);

  const [convo, { refetch: refetchConvo }] = createResource(
    () => selected() ?? undefined,
    (alias) => invoke<MessageDto[]>("peer_conversation", { alias, limit: 500 }),
  );

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
  const selPeer = createMemo(() => peerMap().get((selected() ?? "").toLowerCase()) ?? null);

  // 메시지 + 날짜 구분선 삽입.
  const rows = createMemo(() => {
    const msgs = convo() ?? [];
    const out: ({ kind: "day"; label: string } | { kind: "msg"; m: MessageDto })[] = [];
    let lastDay = "";
    for (const m of msgs) {
      const day = fmtDay(m.timestamp);
      if (day && day !== lastDay) {
        out.push({ kind: "day", label: day });
        lastDay = day;
      }
      out.push({ kind: "msg", m });
    }
    return out;
  });

  let msgsRef: HTMLDivElement | undefined;
  createEffect(() => {
    rows();
    queueMicrotask(() => {
      if (msgsRef) msgsRef.scrollTop = msgsRef.scrollHeight;
    });
  });

  function pick(alias: string) {
    setSelected(alias);
    setSendErr(null);
    setMobileChat(true);
  }

  async function send() {
    const alias = selected();
    const body = draft().trim();
    if (!alias || !body || sending()) return;
    setSending(true);
    setSendErr(null);
    try {
      await invoke("peer_send", { alias, body });
      setDraft("");
      await refetchConvo();
    } catch (e) {
      setSendErr(typeof e === "string" ? e : (e as Error)?.message ?? String(e));
    } finally {
      setSending(false);
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
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

      {/* ── 우측 대화방 (정본: chat-top + msgs + composer) ── */}
      <div class="kk-talk-chat">
        <Show
          when={selAgent()}
          fallback={<div class="kk-talk-blank">좌측에서 대화할 에이전트를 선택하세요.</div>}
        >
          {(a) => (
            <>
              <div class="chat-top">
                <span class="back" onClick={() => setMobileChat(false)}>←</span>
                <div class={`ava ${a().classification === "primary" ? "c-primary" : avatarColor(a().ai_type)}`}>
                  {a().classification === "primary" ? "👑" : a().alias.slice(0, 1).toUpperCase()}
                </div>
                <div class="nm">{a().alias}</div>
                <div class="meta-r">
                  <Show
                    when={isOnline(selPeer()?.last_seen)}
                    fallback={<span class="pill off"><span class="pdot" />오프라인</span>}
                  >
                    <span class="pill"><span class="pdot" />온라인</span>
                  </Show>
                  <Show when={a().machine || selPeer()?.machine}>
                    <span class="pill">🖥 {a().machine || selPeer()?.machine}</span>
                  </Show>
                  <Show when={a().group_name}>
                    <span class="pill">👥 {a().group_name}</span>
                  </Show>
                </div>
              </div>

              <div class="msgs" ref={msgsRef}>
                <Show when={!convo.loading} fallback={<div class="kk-talk-empty">대화 불러오는 중…</div>}>
                  <Show when={!convo.error} fallback={<div class="kk-talk-empty">대화를 불러오지 못했습니다.</div>}>
                    <Show
                      when={(convo() ?? []).length > 0}
                      fallback={<div class="kk-talk-empty">아직 주고받은 메시지가 없습니다.<br />아래에서 첫 메시지를 보내보세요.</div>}
                    >
                      <For each={rows()}>
                        {(r) =>
                          r.kind === "day" ? (
                            <div class="day">{r.label}</div>
                          ) : isSelfSender(r.m.sender) ? (
                            <div class="me">
                              <div class="mr"><div class="tm">{fmtClock(r.m.timestamp)}</div></div>
                              <div class="b">{r.m.body}</div>
                            </div>
                          ) : (
                            <div class="agent">
                              <div class="head">
                                <div class={`av ${avatarColor(a().ai_type)}`}>
                                  {(r.m.sender || a().alias).replace(/^peer:/, "").slice(0, 1).toUpperCase()}
                                </div>
                                <div class="nm">{a().alias}</div>
                                <div class="tm">{fmtClock(r.m.timestamp)}</div>
                              </div>
                              <div class="body">
                                <For each={segmentBody(r.m.body)}>
                                  {(seg) =>
                                    seg.kind === "code" ? (
                                      <pre class="code">{seg.text}</pre>
                                    ) : seg.kind === "tool" ? (
                                      <div class="toolcall"><span class="ok">✓</span> <span class="cmd">{seg.text.replace(/^[✓✗⌗]\s*/, "")}</span></div>
                                    ) : (
                                      <p>{seg.text}</p>
                                    )
                                  }
                                </For>
                              </div>
                            </div>
                          )
                        }
                      </For>
                    </Show>
                  </Show>
                </Show>
              </div>

              {/* ── 컴포저 (정본: Claude Code 다크 + 칩 + 토큰미터) ── */}
              <div class="composer-wrap">
                <div class="composer">
                  <textarea
                    class="ph-input"
                    rows="2"
                    placeholder="메시지 입력···  Type @ for files, / for commands"
                    value={draft()}
                    disabled={sending()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={onKey}
                  />
                  <div class="bar">
                    <div class="bar-l">
                      <span class="ic-btn">@</span>
                      <span class="ic-btn">/</span>
                      <span class="ic-btn">📎</span>
                      <span class="divv" />
                      <span class="mode perm">🛡 Bypass Permissions <span class="car">⌃</span></span>
                      <span class="mode model">Default (recommended) <span class="car">⌃</span></span>
                      <span class="mode think">High <span class="car">⌃</span></span>
                    </div>
                    <div class="spacer" />
                    <div class="bar-r">
                      <span class="usage">0 / 1.00M (0%)</span>
                      <span
                        class={`send${draft().trim() && !sending() ? "" : " dis"}`}
                        onClick={() => void send()}
                      >
                        {sending() ? "…" : "➤"}
                      </span>
                    </div>
                  </div>
                  <Show when={sendErr()}><div class="kk-talk-err">⚠ 전송 실패: {sendErr()}</div></Show>
                </div>
              </div>
            </>
          )}
        </Show>
      </div>
    </div>
  );
}
