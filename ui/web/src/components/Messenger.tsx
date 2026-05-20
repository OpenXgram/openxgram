import { createMemo, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { invoke } from "@/api/client";
import { useI18n } from "../i18n";
import { AgentSidePanel } from "./AgentSidePanel";
import { SessionScreen } from "./SessionScreen";
import { RoutingRulesModal } from "./RoutingRulesModal";

// v1.3 Tier 1 — 좌측 머신×세션 트리 (UI-MESSENGER-SPEC §3.2, S4).
//   - peer 목록 = 본인의 다른 머신/세션 — machine 별 그룹화
//   - ▼/▶ collapse (S4) — 30+ 세션 한 화면 관리
//   - 정렬 (이름·활동) + 필터 (전체·연결만·미연결만)
//   - 4-tuple 부분표시: alias · machine · fingerprint (ULID 도입은 Tier 2 별 단계)
//   - 채널(Discord/Telegram) 친구는 별 "채널" 그룹
//   중앙: L0 messages — 친구 sender 필터, 3초 폴링, peer 송신 활성 (Step 0 완료)

interface MessageDto {
  id: string;
  session_id: string;
  sender: string;
  body: string;
  timestamp: string;
  conversation_id: string;
}

async function fetchMessages(): Promise<MessageDto[]> {
  try {
    return await invoke<MessageDto[]>("messages_recent", { limit: 100 });
  } catch {
    return [];
  }
}

function fmtTime(iso: string): string {
  // ISO 8601 → 'MM-dd HH:mm' (KST). 실패 시 원문.
  try {
    const d = new Date(iso);
    return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  } catch {
    return iso;
  }
}

interface PeerDto {
  alias: string;
  address: string;
  public_key_hex: string;
  machine?: string;
  last_seen?: string;
}

interface NotifyStatusDto {
  telegram_configured: boolean;
  discord_configured: boolean;
  discord_webhook_configured: boolean;
}

type FriendKind = "peer" | "discord" | "telegram" | "tmux" | "claude_project" | "xgram_session";

interface Friend {
  kind: FriendKind;
  id: string;            // peer.alias 또는 "discord" / "telegram" / "tmux:<name>" 등
  display: string;       // 화면에 보일 이름
  subtitle: string;      // 화면 보조 (주소·last_seen·"connected" 등)
  meta?: PeerDto;        // peer일 경우 원본 데이터
  sessionMeta?: DetectedSession; // session 일 경우
}

// v1.3 §3.2 — /v1/gui/sessions 응답 (M-1 통합 detector).
interface MachineInfoDto {
  hostname: string;
  alias: string;
  tailscale_ip: string | null;
}
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
}
interface SessionsDto {
  machine: MachineInfoDto;
  sessions: DetectedSession[];
}
async function fetchSessions(): Promise<SessionsDto | null> {
  try {
    return await invoke<SessionsDto>("sessions");
  } catch {
    return null;
  }
}

async function fetchPeers(): Promise<PeerDto[]> {
  try {
    return await invoke<PeerDto[]>("peers_list");
  } catch {
    return [];
  }
}

async function fetchNotifyStatus(): Promise<NotifyStatusDto> {
  try {
    return await invoke<NotifyStatusDto>("notify_status");
  } catch {
    return {
      telegram_configured: false,
      discord_configured: false,
      discord_webhook_configured: false,
    };
  }
}

function fingerprint(pubkeyHex: string): string {
  const trimmed = pubkeyHex.replace(/^0x/, "");
  if (trimmed.length < 16) return trimmed;
  return `${trimmed.slice(0, 8)}…${trimmed.slice(-8)}`;
}

type SortMode = "name" | "activity";
type ConnFilter = "all" | "connected" | "offline";
type LeftMode = "agent" | "thread";

interface MachineGroup {
  machine: string;
  friends: Friend[];
  connected: number;
}

// 스레드 = 같은 conversation_id 의 메시지 묶음 (Tier 2 — client-side grouping).
// daemon side ThreadStore 는 별 단계 — 지금은 messages_recent 의 conversation_id 활용.
interface ThreadSummary {
  conversation_id: string;
  participants: string[];           // unique senders
  message_count: number;
  last_at: string;                  // ISO
  last_body: string;                // 미리보기
}

const UNKNOWN_MACHINE = "(unknown)";
const UNKNOWN_CONV = "_no_conversation_";

export function Messenger(props: { onJumpToSettings?: () => void } = {}) {
  const { t } = useI18n();
  const [peers] = createResource(fetchPeers);
  const [notifyStatus] = createResource(fetchNotifyStatus);
  // v1.3 §3.2 — 이 머신의 tmux + Claude Code projects + xgram sessions 통합.
  const [sessions] = createResource(fetchSessions);
  const [selected, setSelected] = createSignal<string | null>(null);     // friend id (에이전트 모드)
  const [selectedThread, setSelectedThread] = createSignal<string | null>(null); // conversation_id
  const [leftMode, setLeftMode] = createSignal<LeftMode>("agent");        // L1
  // L5 — Hand-off 모달
  const [handoffSource, setHandoffSource] = createSignal<MessageDto | null>(null);
  const [showRouting, setShowRouting] = createSignal(false); // V11
  const [messages, { refetch: refetchMessages }] = createResource(fetchMessages);

  // 좌측 컨트롤
  const [sortMode, setSortMode] = createSignal<SortMode>("activity");
  const [connFilter, setConnFilter] = createSignal<ConnFilter>("all");
  const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({});

  // 3초 간격 메시지 폴링 — 활동 흐름 모니터링.
  const pollTimer = setInterval(() => {
    void refetchMessages();
  }, 3000);
  onCleanup(() => clearInterval(pollTimer));

  function toggleCollapse(machine: string) {
    setCollapsed((prev) => ({ ...prev, [machine]: !prev[machine] }));
  }

  function isConnected(p: PeerDto): boolean {
    // last_seen 이 1시간 이내면 연결, 아니면 offline 으로 간주.
    if (!p.last_seen) return false;
    const ts = Date.parse(p.last_seen);
    if (Number.isNaN(ts)) return false;
    return Date.now() - ts < 60 * 60 * 1000;
  }

  // peer 만 머신 그룹화 + 채널은 별 "채널" 가짜 머신.
  const groups = createMemo<MachineGroup[]>(() => {
    const byMachine = new Map<string, Friend[]>();

    for (const p of peers() ?? []) {
      if (connFilter() === "connected" && !isConnected(p)) continue;
      if (connFilter() === "offline" && isConnected(p)) continue;
      const m = (p.machine?.trim() || UNKNOWN_MACHINE);
      const friend: Friend = {
        kind: "peer",
        id: `peer:${p.alias}`,
        display: p.alias,
        // L2 4-tuple: alias · machine · address(short) · fingerprint
        subtitle:
          `${(p.address || "").slice(0, 10)} · ${fingerprint(p.public_key_hex)}` +
          (p.last_seen ? ` · ${p.last_seen}` : ""),
        meta: p,
      };
      if (!byMachine.has(m)) byMachine.set(m, []);
      byMachine.get(m)!.push(friend);
    }

    // 이 머신의 tmux + Claude Code projects + xgram sessions (M-1).
    // sessions.machine.alias 를 머신 그룹으로 사용 — peers 의 machine 과 중복 시 같은 그룹에 추가.
    const sess = sessions();
    if (sess) {
      const m = sess.machine.alias || sess.machine.hostname || UNKNOWN_MACHINE;
      const arr = byMachine.get(m) ?? [];
      for (const s of sess.sessions) {
        // 연결 필터: attached/active = 연결, 그 외 = offline (간단 매핑)
        const conn = s.status === "attached" || s.status === "active";
        if (connFilter() === "connected" && !conn) continue;
        if (connFilter() === "offline" && conn) continue;
        const icon =
          s.kind === "tmux" ? "📟" : s.kind === "claude_project" ? "🤖" : "🪪";
        arr.push({
          kind: s.kind as FriendKind,
          id: `session:${s.identifier}`,
          display: `${icon} ${s.display}`,
          subtitle:
            s.kind === "tmux"
              ? `tmux · ${s.windows ?? 0} win · ${s.attached ? "attached" : "detached"}`
              : s.kind === "claude_project"
              ? `Claude Code · ${s.last_active_at ? new Date(s.last_active_at).toLocaleString() : "—"}`
              : "xgram session",
          sessionMeta: s,
        });
      }
      byMachine.set(m, arr);
    }

    // 정렬
    const sorter = (a: Friend, b: Friend) => {
      if (sortMode() === "name") return a.display.localeCompare(b.display);
      // activity: meta.last_seen DESC (없으면 뒤로)
      const ta = a.meta?.last_seen ? Date.parse(a.meta.last_seen) : 0;
      const tb = b.meta?.last_seen ? Date.parse(b.meta.last_seen) : 0;
      return tb - ta;
    };
    for (const arr of byMachine.values()) arr.sort(sorter);

    const out: MachineGroup[] = [];
    // 머신 정렬: 이름순. UNKNOWN_MACHINE 은 마지막.
    const machines = Array.from(byMachine.keys()).sort((a, b) => {
      if (a === UNKNOWN_MACHINE) return 1;
      if (b === UNKNOWN_MACHINE) return -1;
      return a.localeCompare(b);
    });
    for (const m of machines) {
      const friends = byMachine.get(m)!;
      out.push({
        machine: m,
        friends,
        connected: friends.filter((f) => f.meta && isConnected(f.meta)).length,
      });
    }

    // 채널 그룹
    const ns = notifyStatus();
    if (ns) {
      const channels: Friend[] = [];
      channels.push({
        kind: "discord",
        id: "channel:discord",
        display: "Discord",
        subtitle: ns.discord_configured
          ? t("messenger.connected") || "connected"
          : t("messenger.add-bot") || "add bot →",
      });
      channels.push({
        kind: "telegram",
        id: "channel:telegram",
        display: "Telegram",
        subtitle: ns.telegram_configured
          ? t("messenger.connected") || "connected"
          : t("messenger.add-bot") || "add bot →",
      });
      if (channels.length > 0) {
        out.push({
          machine: "📱 채널",
          friends: channels,
          connected:
            (ns.discord_configured ? 1 : 0) + (ns.telegram_configured ? 1 : 0),
        });
      }
    }

    return out;
  });

  const friends = createMemo<Friend[]>(() =>
    groups().flatMap((g) => g.friends),
  );

  const selectedFriend = createMemo(() => {
    const id = selected();
    if (!id) return null;
    return friends().find((f) => f.id === id) ?? null;
  });

  // 스레드 모드 (L1) — messages 를 conversation_id 별 그룹화
  const threads = createMemo<ThreadSummary[]>(() => {
    const all = messages() ?? [];
    const map = new Map<string, MessageDto[]>();
    for (const m of all) {
      const cid = m.conversation_id || UNKNOWN_CONV;
      if (!map.has(cid)) map.set(cid, []);
      map.get(cid)!.push(m);
    }
    const list: ThreadSummary[] = [];
    for (const [cid, msgs] of map.entries()) {
      msgs.sort((a, b) => Date.parse(b.timestamp) - Date.parse(a.timestamp));
      const participants = Array.from(new Set(msgs.map((m) => m.sender)));
      list.push({
        conversation_id: cid,
        participants,
        message_count: msgs.length,
        last_at: msgs[0].timestamp,
        last_body: msgs[0].body.slice(0, 60),
      });
    }
    // 최근 활동순
    list.sort((a, b) => Date.parse(b.last_at) - Date.parse(a.last_at));
    return list;
  });

  return (
    <div
      class="messenger-shell"
      style={
        // friend 선택 시 우측 panel — 3-col. 아니면 2-col.
        selectedFriend()
          ? "grid-template-columns: 280px 1fr 360px;"
          : "grid-template-columns: 280px 1fr;"
      }
    >
      {/* 좌: 머신×세션 트리 + 스레드 모드 (Tier 1 + L1) */}
      <aside class="messenger-sidebar">
        {/* L1 — 좌측 상단 2-모드 탭 */}
        <div class="messenger-sidebar-mode" style="display:flex; gap:4px; padding:6px 8px; border-bottom:1px solid var(--border);">
          <button
            type="button"
            class={leftMode() === "agent" ? "active" : ""}
            onClick={() => setLeftMode("agent")}
            style="flex:1;"
          >
            🤖 에이전트
          </button>
          <button
            type="button"
            class={leftMode() === "thread" ? "active" : ""}
            onClick={() => setLeftMode("thread")}
            style="flex:1;"
          >
            🧵 스레드 ({threads().length})
          </button>
          {/* V11 — RoutingRule 모달 (agent↔agent internal) */}
          <button
            type="button"
            onClick={() => setShowRouting(true)}
            title="RoutingRule (V11)"
            style="flex:0;"
          >
            🔀
          </button>
        </div>
        <Show when={showRouting()}>
          <RoutingRulesModal onClose={() => setShowRouting(false)} />
        </Show>

        <header class="messenger-sidebar-head">
          <strong>
            {leftMode() === "agent"
              ? t("messenger.friends") || "친구"
              : "스레드"}
          </strong>
          <Show when={leftMode() === "agent"}>
            <button
              type="button"
              class="messenger-add-btn"
              title={t("messenger.add-friend-tip") || "peer 등록 / 봇 연결"}
              onClick={() => {
                alert(
                  t("messenger.add-friend-hint") ||
                    "친구 추가: 연결 탭의 [+ Peer] 또는 설정 탭의 [Discord/Telegram 봇 추가]",
                );
              }}
            >
              +
            </button>
          </Show>
        </header>

        {/* 정렬·필터 컨트롤 (S4) — 에이전트 모드만 */}
        <Show when={leftMode() === "agent"}>
        <div class="messenger-sidebar-ctrl" style="display:flex; gap:6px; padding:6px 8px; font-size:0.85em;">
          <select
            value={sortMode()}
            onChange={(e) => setSortMode(e.currentTarget.value as SortMode)}
            title="정렬"
          >
            <option value="activity">활동순</option>
            <option value="name">이름순</option>
          </select>
          <select
            value={connFilter()}
            onChange={(e) => setConnFilter(e.currentTarget.value as ConnFilter)}
            title="연결 필터"
          >
            <option value="all">전체</option>
            <option value="connected">연결만</option>
            <option value="offline">offline만</option>
          </select>
        </div>

        <div class="messenger-friend-list">
          <For each={groups()}>
            {(g) => {
              const isCollapsed = () => collapsed()[g.machine] === true;
              return (
                <div class="messenger-machine-group">
                  <div
                    class="messenger-machine-header"
                    onClick={() => toggleCollapse(g.machine)}
                    style="cursor:pointer;"
                  >
                    <span style="margin-right:4px;">
                      {isCollapsed() ? "▶" : "▼"}
                    </span>
                    🟢 {g.machine || "(unknown)"}{" "}
                    <span style="font-weight:400; color:var(--text-3); font-size:12px;">
                      ({g.friends.length} · {g.connected} 연결)
                    </span>
                  </div>
                  <Show when={!isCollapsed()}>
                    <ul style="margin:0; padding:0;">
                      <For each={g.friends}>
                        {(f) => (
                          <li
                            class={
                              selected() === f.id
                                ? "messenger-friend selected"
                                : "messenger-friend"
                            }
                            onClick={() => setSelected(f.id)}
                          >
                            <span class={`messenger-friend-icon kind-${f.kind}`}>
                              {f.kind === "peer"
                                ? "🤖"
                                : f.kind === "discord"
                                  ? "D"
                                  : "T"}
                            </span>
                            <span class="messenger-friend-text">
                              <span class="messenger-friend-name">{f.display}</span>
                              <span class="messenger-friend-sub">{f.subtitle}</span>
                            </span>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                </div>
              );
            }}
          </For>
          <Show when={(friends() ?? []).length === 0}>
            <div class="messenger-empty" style="padding:12px;">
              {t("messenger.no-friends") || "친구 없음 — + 버튼으로 추가"}
            </div>
          </Show>
        </div>
        </Show>

        {/* L1 — 스레드 모드 콘텐츠 */}
        <Show when={leftMode() === "thread"}>
          <div class="messenger-friend-list">
            <Show
              when={threads().length > 0}
              fallback={
                <div class="messenger-empty" style="padding:12px;">
                  스레드 없음 — 메시지 송수신 시 conversation_id 별 자동 생성
                </div>
              }
            >
              <For each={threads()}>
                {(th) => (
                  <div
                    class={
                      selectedThread() === th.conversation_id
                        ? "messenger-friend selected"
                        : "messenger-friend"
                    }
                    onClick={() => setSelectedThread(th.conversation_id)}
                    style="cursor:pointer; padding:8px;"
                  >
                    <div style="font-weight:600; font-size:0.9em;">
                      {th.conversation_id === UNKNOWN_CONV
                        ? "(no conv_id)"
                        : `#${th.conversation_id.slice(0, 8)}`}
                    </div>
                    <div style="font-size:0.8em; opacity:0.7;">
                      {th.participants.length} agents · {th.message_count} msg ·{" "}
                      {fmtTime(th.last_at)}
                    </div>
                    <div style="font-size:0.85em; opacity:0.85; margin-top:2px;">
                      {th.last_body}
                    </div>
                  </div>
                )}
              </For>
            </Show>
          </div>
        </Show>
      </aside>

      {/* 중: 대화 — 에이전트 모드 (friend 선택) or 스레드 모드 (conv_id 선택) */}
      <main class="messenger-thread">
        {/* 스레드 모드: 선택된 conversation_id 의 메시지 시간순 */}
        <Show when={leftMode() === "thread" && selectedThread()}>
          {() => {
            const cid = selectedThread()!;
            const th = createMemo(() => threads().find((x) => x.conversation_id === cid));
            const threadMsgs = createMemo(() =>
              (messages() ?? [])
                .filter((m) => (m.conversation_id || UNKNOWN_CONV) === cid)
                .sort((a, b) => Date.parse(a.timestamp) - Date.parse(b.timestamp)),
            );
            return (
              <>
                <header class="messenger-thread-head">
                  <h2>
                    🧵{" "}
                    {cid === UNKNOWN_CONV ? "(no conv_id)" : `#${cid.slice(0, 12)}`}
                  </h2>
                  <small>
                    참여 {th()?.participants.length ?? 0} · 메시지{" "}
                    {th()?.message_count ?? 0}
                  </small>
                </header>
                <section class="messenger-thread-body">
                  <Show
                    when={threadMsgs().length > 0}
                    fallback={
                      <div class="messenger-placeholder">메시지 없음</div>
                    }
                  >
                    <ul class="messenger-thread-list">
                      <For each={threadMsgs()}>
                        {(m) => (
                          <li class="messenger-thread-item">
                            <div class="messenger-thread-meta">
                              <span class="messenger-thread-sender">🤖 {m.sender}</span>
                              <span class="messenger-thread-time">{fmtTime(m.timestamp)}</span>
                              <button
                                type="button"
                                title="Hand-off — 다른 에이전트에 인계"
                                onClick={() => setHandoffSource(m)}
                                style="background:transparent; border:none; cursor:pointer; opacity:0.6; padding:0 4px;"
                              >
                                ↗
                              </button>
                            </div>
                            <div class="messenger-thread-body-text">{m.body}</div>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                </section>
              </>
            );
          }}
        </Show>

        {/* 에이전트 모드 (기존) */}
        <Show
          when={leftMode() === "agent" && selectedFriend()}
          fallback={
            <Show when={!(leftMode() === "thread" && selectedThread())}>
              <div class="messenger-thread-empty">
                <p>
                  {leftMode() === "agent"
                    ? t("messenger.select-friend") || "왼쪽에서 친구를 선택하세요."
                    : "왼쪽에서 스레드를 선택하세요."}
                </p>
                <p class="messenger-thread-hint">
                  Tier 2 — 에이전트/스레드 2-모드. 4-tuple 표시 (alias·machine·address).
                </p>
              </div>
            </Show>
          }
        >
          {(f) => {
            // 세션 친구 (tmux/claude_project/xgram_session) → SessionScreen (xterm.js, S5).
            const sessionKinds = new Set(["tmux", "claude_project", "xgram_session"]);
            if (sessionKinds.has(f().kind) && f().sessionMeta) {
              return (
                <SessionScreen
                  identifier={f().sessionMeta!.identifier}
                  display={f().display}
                />
              );
            }
            // peer 친구는 sender 로 필터, 채널(Discord/Telegram)은 일단 전체 보여줌.
            const filtered = createMemo<MessageDto[]>(() => {
              const all = messages() ?? [];
              if (f().kind !== "peer") return all;
              const alias = f().display;
              const addr = f().meta?.address?.toLowerCase();
              return all.filter((m) => {
                const s = m.sender.toLowerCase();
                return s === alias.toLowerCase() || (addr ? s === addr : false);
              });
            });

            return (
              <>
                <header class="messenger-thread-head">
                  <h2>🤖 {f().display}</h2>
                  {/* L2 4-tuple — alias · machine · address · fingerprint */}
                  <small>
                    {f().meta?.machine ? `${f().meta?.machine} · ` : ""}
                    {f().meta?.address?.slice(0, 18) || ""}
                    {f().meta?.public_key_hex
                      ? ` · ${fingerprint(f().meta!.public_key_hex)}`
                      : ""}
                  </small>
                </header>
                <section class="messenger-thread-body">
                  <Show
                    when={(filtered() ?? []).length > 0}
                    fallback={
                      <div class="messenger-placeholder">
                        {t("messenger.thread-empty") ||
                          `${f().display} 의 메시지 없음 — daemon 가동 + 메시지 도착 시 3초 내 표시됩니다.`}
                      </div>
                    }
                  >
                    <ul class="messenger-thread-list">
                      <For each={filtered().slice().reverse()}>
                        {(m) => (
                          <li class="messenger-thread-item">
                            <div class="messenger-thread-meta">
                              <span class="messenger-thread-sender">{m.sender}</span>
                              <span class="messenger-thread-time">{fmtTime(m.timestamp)}</span>
                              <button
                                type="button"
                                title="Hand-off — 다른 에이전트에 인계"
                                onClick={() => setHandoffSource(m)}
                                style="background:transparent; border:none; cursor:pointer; opacity:0.6; padding:0 4px;"
                              >
                                ↗
                              </button>
                            </div>
                            <div class="messenger-thread-body-text">{m.body}</div>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                </section>
                <PeerInput
                  friend={f()}
                  onSent={() => {
                    void refetchMessages();
                  }}
                />
              </>
            );
          }}
        </Show>
      </main>

      {/* 우: 12 탭 사이드 패널 (사양 §5 — peer 또는 session friend 선택 시) */}
      <Show
        when={
          leftMode() === "agent" &&
          selectedFriend() &&
          (selectedFriend()!.kind === "peer" ||
            selectedFriend()!.kind === "tmux" ||
            selectedFriend()!.kind === "claude_project" ||
            selectedFriend()!.kind === "xgram_session")
        }
      >
        {() => {
          const f = selectedFriend()!;
          // session friend → 합성 PeerMeta 로 변환 (탭 UI 재사용).
          const synthPeer = f.meta ?? {
            alias: f.display,
            address: f.sessionMeta?.identifier ?? "",
            public_key_hex: "",
            machine: undefined,
            last_seen: f.sessionMeta?.last_active_at ?? undefined,
          };
          return (
            <AgentSidePanel
              peer={synthPeer}
              onJumpToSettings={() => props.onJumpToSettings?.()}
            />
          );
        }}
      </Show>

      {/* L5 — Hand-off 모달 (메시지 옆 ↗ 버튼 클릭 시) */}
      <Show when={handoffSource()}>
        {() => (
          <HandoffModal
            source={handoffSource()!}
            peers={(peers() ?? []).filter((p) => p.alias !== handoffSource()!.sender)}
            onClose={() => setHandoffSource(null)}
            onSent={() => {
              setHandoffSource(null);
              void refetchMessages();
            }}
          />
        )}
      </Show>
    </div>
  );
}

// ── L5 Hand-off 모달 ──────────────────────────────────────────────
function HandoffModal(props: {
  source: MessageDto;
  peers: PeerDto[];
  onClose: () => void;
  onSent: () => void;
}) {
  const [target, setTarget] = createSignal<string>(props.peers[0]?.alias ?? "");
  const [threadMode, setThreadMode] = createSignal<"new" | "existing">("new");
  const [summary, setSummary] = createSignal(
    `[hand-off] ${props.source.sender} 가 보낸 메시지 인계 — 검토·후속 작업 부탁:\n"${props.source.body.slice(0, 200)}"`,
  );
  const [sending, setSending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function handoff() {
    if (!target()) {
      setError("대상 peer 가 없습니다 (다른 peer 1명 이상 필요)");
      return;
    }
    setSending(true);
    setError(null);
    try {
      // 새 스레드 = conversation_id 생략 (daemon 자동 생성).
      // 기존 스레드 = source 의 conversation_id 그대로.
      const conv = threadMode() === "existing" ? props.source.conversation_id : undefined;
      await invoke("peer_send", {
        alias: target(),
        body: summary(),
        ...(conv ? { conversation_id: conv } : {}),
      });
      props.onSent();
    } catch (e: any) {
      setError(typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      setSending(false);
    }
  }

  return (
    <div class="handoff-overlay" onClick={props.onClose}>
      <div class="handoff-modal" onClick={(e) => e.stopPropagation()}>
        <h3>↗ Hand-off — 메시지 인계</h3>
        <div class="source-preview">
          <div>출처: 🤖 {props.source.sender}</div>
          <div style="margin-top:4px;">
            {props.source.body.slice(0, 120)}
            {props.source.body.length > 120 ? "…" : ""}
          </div>
        </div>

        <div style="margin-bottom:10px;">
          <label>대상 peer</label>
          <select value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
            <For each={props.peers}>
              {(p) => <option value={p.alias}>{p.alias} · {p.machine || "(unknown)"}</option>}
            </For>
            <Show when={props.peers.length === 0}>
              <option value="">— 다른 peer 없음 —</option>
            </Show>
          </select>
        </div>

        <div style="margin-bottom:10px;">
          <label>자동 요약 (편집 가능)</label>
          <textarea
            rows={4}
            value={summary()}
            onInput={(e) => setSummary(e.currentTarget.value)}
          />
        </div>

        <div style="margin-bottom:14px;">
          <label>스레드 처리 (radio — 둘 중 하나)</label>
          <div class="radio-row">
            <label style="cursor:pointer;">
              <input
                type="radio"
                checked={threadMode() === "new"}
                onChange={() => setThreadMode("new")}
              />{" "}
              새 스레드 생성 (parent: 이 스레드)
            </label>
          </div>
          <div class="radio-row">
            <label style="cursor:pointer;">
              <input
                type="radio"
                checked={threadMode() === "existing"}
                onChange={() => setThreadMode("existing")}
              />{" "}
              기존 스레드에 추가 (conv: {props.source.conversation_id.slice(0, 10)}…)
            </label>
          </div>
        </div>

        <Show when={error()}>
          <div style="color:#f87171; font-size:12px; margin-bottom:8px;">{error()}</div>
        </Show>

        <div class="actions">
          <button type="button" onClick={props.onClose} disabled={sending()}>
            취소
          </button>
          <button
            type="button"
            class="primary"
            onClick={() => void handoff()}
            disabled={sending() || !target() || props.peers.length === 0}
          >
            {sending() ? "인계 중…" : "인계 →"}
          </button>
        </div>
      </div>
    </div>
  );
}

// 채널(Discord/Telegram) 친구는 입력 비활성, peer 만 송신 가능.
function PeerInput(props: { friend: Friend; onSent: () => void }) {
  const { t } = useI18n();
  const [text, setText] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const isPeer = () => props.friend.kind === "peer";

  // UI-MESSENGER-SPEC v1.3 §4.5 — 사용자 개입 = system priority (기본) / 일반 참여자.
  const [intervention, setIntervention] = createSignal(true);

  async function send() {
    const raw = text().trim();
    if (!raw) return;
    if (!isPeer()) {
      setError(t("messenger.send-peer-only") || "송신은 peer 친구에게만 가능 (Discord/Telegram 채널 송신은 별도)");
      return;
    }
    setSending(true);
    setError(null);
    try {
      // 개입 모드 = body 에 [개입] prefix (수신 측 LLM 이 system priority 로 인식).
      const body = intervention() ? `[개입] ${raw}` : raw;
      await invoke("peer_send", { alias: props.friend.display, body });
      setText("");
      props.onSent();
    } catch (e: any) {
      setError(typeof e === "string" ? e : (e?.message ?? String(e)));
    } finally {
      setSending(false);
    }
  }

  return (
    <footer class="messenger-thread-input">
      {/* L5/§4.5 — 입력 모드 토글 */}
      <div class="messenger-input-mode">
        <label>
          <input
            type="radio"
            checked={intervention()}
            onChange={() => setIntervention(true)}
          />{" "}
          개입 (system priority)
        </label>
        <label>
          <input
            type="radio"
            checked={!intervention()}
            onChange={() => setIntervention(false)}
          />{" "}
          일반 참여
        </label>
      </div>
      <div class="messenger-thread-input-row">
        <textarea
          rows={2}
          value={text()}
          onInput={(ev) => setText(ev.currentTarget.value)}
          placeholder={
            isPeer()
              ? intervention()
                ? "[개입] 메시지 — system priority 로 전송 (Enter, Shift+Enter 줄바꿈)"
                : (t("messenger.input-placeholder") || "메시지 입력")
              : (t("messenger.send-peer-only") || "Discord/Telegram 채널 송신은 별도")
          }
          disabled={!isPeer() || sending()}
          onKeyDown={(ev) => {
            if (ev.key === "Enter" && !ev.shiftKey) {
              ev.preventDefault();
              void send();
            }
          }}
        />
        <button
          type="button"
          disabled={!isPeer() || sending() || !text().trim()}
          onClick={() => void send()}
        >
          {sending()
            ? (t("messenger.sending") || "보내는 중…")
            : intervention()
              ? "[개입] 전송"
              : (t("messenger.send") || "전송")}
        </button>
      </div>
      <Show when={error()}>
        <div class="messenger-thread-error" role="alert">{error()}</div>
      </Show>
    </footer>
  );
}

