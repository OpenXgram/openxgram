import { createSignal, createResource, createMemo, createEffect, For, Show } from "solid-js";
import { invoke } from "../api/client";

// 대화 탭 — 카카오톡 네이티브 재디자인. 정본: _mockups/kakao-mockup.html (.chat-top·말풍선·다크 컴포저).
// Messenger.tsx 의 백엔드 contract 재사용(수정 X): peers_list / peer_conversation / peer_send.
// 좌: 명부(kk-row 재사용) · 우: chat-top + 말풍선 영역 + Claude-Code 스타일 다크 컴포저(토큰 미터).

interface PeerDto {
  alias: string;
  address: string;
  public_key_hex: string;
  machine?: string;
  last_seen?: string;
  description?: string | null;
  capabilities?: string[];
  role?: string | null;
  project_folder?: string | null;
  llm_type?: string | null;
  llm_version?: string | null;
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

// Messenger.tsx isSelfSender 와 동일 규칙: self/me/user → 내 발신(오른쪽).
function isSelfSender(sender: string): boolean {
  const s = (sender || "").toLowerCase();
  if (s === "me" || s === "user") return true;
  if (s.startsWith("self:") || s.startsWith("me:")) return true;
  return false;
}

// Messenger.tsx connTier 와 동일: 1h 이내 online, 24h 이내 idle, 그 외 offline.
function onlineState(lastSeen?: string): "online" | "idle" | "offline" {
  if (!lastSeen) return "offline";
  const t = Date.parse(lastSeen);
  if (Number.isNaN(t)) return "offline";
  const age = Date.now() - t;
  if (age < 60 * 60 * 1000) return "online";
  if (age < 24 * 60 * 60 * 1000) return "idle";
  return "offline";
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

function fmtDay(iso: string): string {
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    return `${d.getFullYear()}년 ${d.getMonth() + 1}월 ${d.getDate()}일`;
  } catch {
    return "";
  }
}

export function TalkTab(props: { onJumpToSettings?: () => void }) {
  const [peers, { refetch: refetchPeers }] = createResource<PeerDto[]>(() => invoke("peers_list"));
  const [selected, setSelected] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [sendErr, setSendErr] = createSignal<string | null>(null);
  const [mobileChat, setMobileChat] = createSignal(false);

  const [convo, { refetch: refetchConvo }] = createResource(
    () => selected() ?? undefined,
    (alias) => invoke<MessageDto[]>("peer_conversation", { alias, limit: 500 }),
  );

  // 명부 정렬: online 먼저, 그다음 last_seen 최신순.
  const sortedPeers = createMemo(() => {
    const list = [...(peers() ?? [])];
    const rank = (p: PeerDto) => {
      const s = onlineState(p.last_seen ?? undefined);
      return s === "online" ? 0 : s === "idle" ? 1 : 2;
    };
    return list.sort((a, b) => {
      const r = rank(a) - rank(b);
      if (r !== 0) return r;
      const ta = a.last_seen ? Date.parse(a.last_seen) : 0;
      const tb = b.last_seen ? Date.parse(b.last_seen) : 0;
      return (tb || 0) - (ta || 0);
    });
  });

  const selPeer = createMemo(() => sortedPeers().find((p) => p.alias === selected()) ?? null);

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

  // 토큰 미터: 백엔드 per-conversation 예산값 미노출 → 정본 목업과 동일한 placeholder 표기.
  const usageLabel = () => "토큰 미터 · 데몬 예산 배선 대기 (Phase 6)";

  return (
    <div class={`kk-talk${mobileChat() ? " mchat" : ""}`}>
      {/* 좌측 명부 */}
      <div class="kk-talk-roster">
        <div class="rtop">
          <h2>대화</h2>
        </div>
        <Show when={!peers.loading} fallback={<div class="empty">불러오는 중…</div>}>
          <Show when={!peers.error} fallback={<div class="empty">명부를 불러오지 못했습니다.<br />데몬 연결을 확인하세요.</div>}>
            <Show
              when={sortedPeers().length > 0}
              fallback={<div class="empty">대화 가능한 에이전트가 없습니다.<br /><b>에이전트</b> 탭에서 peer 를 등록하세요.</div>}
            >
              <For each={sortedPeers()}>
                {(p) => {
                  const st = () => onlineState(p.last_seen ?? undefined);
                  return (
                    <div
                      class={`kk-row${selected() === p.alias ? " active" : ""}${p.role === "primary" ? " primary" : ""}`}
                      onClick={() => pick(p.alias)}
                    >
                      <div class={`kk-ava ${avatarColor(p.llm_type)}`}>
                        {p.alias.slice(0, 1).toUpperCase()}
                        <span class={`dot${st() === "online" ? " on" : ""}`} />
                      </div>
                      <div class="kk-meta">
                        <div class="kk-nm">
                          {p.alias}
                          <Show when={p.llm_type}><span class="tag">{p.llm_type}</span></Show>
                        </div>
                        <div class="kk-st">{p.role || p.description || p.machine || "—"}</div>
                      </div>
                    </div>
                  );
                }}
              </For>
            </Show>
          </Show>
        </Show>
      </div>

      {/* 우측 대화방 */}
      <div class="kk-talk-chat">
        <Show
          when={selPeer()}
          fallback={<div class="kk-talk-blank">좌측에서 대화할 에이전트를 선택하세요.</div>}
        >
          {(p) => (
            <>
              <div class="kk-talk-top">
                <span class="kk-talk-back" onClick={() => setMobileChat(false)}>←</span>
                <div class={`kk-ava ${avatarColor(p().llm_type)}`}>{p().alias.slice(0, 1).toUpperCase()}</div>
                <div class="nm">{p().alias}</div>
                <div class="meta-r">
                  <Show when={onlineState(p().last_seen ?? undefined) === "online"} fallback={
                    <span class="kk-pill off"><span class="pdot" />{onlineState(p().last_seen ?? undefined) === "idle" ? "유휴" : "오프라인"}</span>
                  }>
                    <span class="kk-pill"><span class="pdot" />온라인</span>
                  </Show>
                  <Show when={p().machine}><span class="kk-pill">🖥 {p().machine}</span></Show>
                </div>
              </div>

              <div class="kk-talk-msgs" ref={msgsRef}>
                <Show when={!convo.loading} fallback={<div class="kk-talk-empty">대화 불러오는 중…</div>}>
                  <Show
                    when={(convo() ?? []).length > 0}
                    fallback={<div class="kk-talk-empty">아직 주고받은 메시지가 없습니다.<br />아래에서 첫 메시지를 보내보세요.</div>}
                  >
                    <For each={rows()}>
                      {(r) =>
                        r.kind === "day" ? (
                          <div class="kk-talk-day">{r.label}</div>
                        ) : isSelfSender(r.m.sender) ? (
                          <div class="kk-talk-me">
                            <div class="mr"><div class="tm">{fmtClock(r.m.timestamp)}</div></div>
                            <div class="b">{r.m.body}</div>
                          </div>
                        ) : (
                          <div class="kk-talk-agent">
                            <div class="head">
                              <div class={`av ${avatarColor(p().llm_type)}`}>{(r.m.sender || p().alias).slice(0, 1).toUpperCase()}</div>
                              <div class="nm">{p().alias}</div>
                              <div class="tm">{fmtClock(r.m.timestamp)}</div>
                            </div>
                            <div class="body">{r.m.body}</div>
                          </div>
                        )
                      }
                    </For>
                  </Show>
                </Show>
              </div>

              {/* Claude Code 스타일 다크 컴포저 + 토큰 미터 */}
              <div class="kk-talk-composer-wrap">
                <div class="kk-talk-composer">
                  <textarea
                    class="kk-talk-input"
                    rows="2"
                    placeholder="메시지 입력…  Shift+Enter 줄바꿈 · Enter 전송"
                    value={draft()}
                    disabled={sending()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={onKey}
                  />
                  <div class="bar">
                    <div class="bar-l">
                      <span class="ic-btn" title="파일/명령 (데몬 배선 대기)">@</span>
                      <span class="ic-btn" title="명령 (데몬 배선 대기)">/</span>
                      <span class="divv" />
                      <span class="mode" onClick={() => props.onJumpToSettings?.()}>설정 ⌃</span>
                    </div>
                    <div class="bar-r">
                      <span class="usage">{usageLabel()}</span>
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
