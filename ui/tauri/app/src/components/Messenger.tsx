import { createMemo, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

// v0.2-α — 활동 흐름 모니터링 컷.
//   좌측: 친구 목록 = OpenXgram peer + Discord/Telegram 연결 상태 통합
//   중앙: 최근 L0 messages 스레드 — 친구 선택 시 해당 sender 로 필터, 미선택 시 전체.
//          3초 간격 자동 새로고침. 송신은 v0.2-β (현재 disabled + CLI 안내).
//   친구 추가 = `xgram peer add` / `setup discord` / `setup telegram`. 메신저에서 "친구 추가" 버튼이 그 흐름을 연다.

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

type FriendKind = "peer" | "discord" | "telegram";

interface Friend {
  kind: FriendKind;
  id: string;            // peer.alias 또는 "discord" / "telegram"
  display: string;       // 화면에 보일 이름
  subtitle: string;      // 화면 보조 (주소·last_seen·"connected" 등)
  meta?: PeerDto;        // peer일 경우 원본 데이터
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

export function Messenger() {
  const { t } = useI18n();
  const [peers] = createResource(fetchPeers);
  const [notifyStatus] = createResource(fetchNotifyStatus);
  const [selected, setSelected] = createSignal<string | null>(null);
  const [messages, { refetch: refetchMessages }] = createResource(fetchMessages);

  // 3초 간격 메시지 폴링 — 활동 흐름 모니터링.
  const pollTimer = setInterval(() => {
    void refetchMessages();
  }, 3000);
  onCleanup(() => clearInterval(pollTimer));

  // peer + Discord/Telegram 상태를 한 친구 목록으로 합침.
  const friends = createMemo<Friend[]>(() => {
    const list: Friend[] = [];

    for (const p of peers() ?? []) {
      list.push({
        kind: "peer",
        id: `peer:${p.alias}`,
        display: p.alias,
        subtitle: p.last_seen
          ? `${fingerprint(p.public_key_hex)} · ${p.last_seen}`
          : fingerprint(p.public_key_hex),
        meta: p,
      });
    }

    const ns = notifyStatus();
    if (ns) {
      list.push({
        kind: "discord",
        id: "channel:discord",
        display: "Discord",
        subtitle: ns.discord_configured
          ? t("messenger.connected") || "connected"
          : t("messenger.add-bot") || "add bot →",
      });
      list.push({
        kind: "telegram",
        id: "channel:telegram",
        display: "Telegram",
        subtitle: ns.telegram_configured
          ? t("messenger.connected") || "connected"
          : t("messenger.add-bot") || "add bot →",
      });
    }
    return list;
  });

  const selectedFriend = createMemo(() => {
    const id = selected();
    if (!id) return null;
    return friends().find((f) => f.id === id) ?? null;
  });

  return (
    <div class="messenger-shell">
      {/* 좌: 친구 목록 */}
      <aside class="messenger-sidebar">
        <header class="messenger-sidebar-head">
          <strong>{t("messenger.friends") || "친구"}</strong>
          <button
            type="button"
            class="messenger-add-btn"
            title={t("messenger.add-friend-tip") || "peer 등록 / 봇 연결"}
            onClick={() => {
              // v0.2에서 모달로 분기 — 지금은 기존 탭으로 안내
              alert(
                t("messenger.add-friend-hint") ||
                  "v0.2: 친구 추가 모달\n현재는 Peers 탭에서 peer 등록, Notify 탭에서 Discord/Telegram 연결",
              );
            }}
          >
            +
          </button>
        </header>
        <ul class="messenger-friend-list">
          <For each={friends()}>
            {(f) => (
              <li
                class={selected() === f.id ? "messenger-friend selected" : "messenger-friend"}
                onClick={() => setSelected(f.id)}
              >
                <span class={`messenger-friend-icon kind-${f.kind}`}>
                  {f.kind === "peer" ? "◆" : f.kind === "discord" ? "D" : "T"}
                </span>
                <span class="messenger-friend-text">
                  <span class="messenger-friend-name">{f.display}</span>
                  <span class="messenger-friend-sub">{f.subtitle}</span>
                </span>
              </li>
            )}
          </For>
          <Show when={(friends() ?? []).length === 0}>
            <li class="messenger-empty">
              {t("messenger.no-friends") || "친구 없음 — + 버튼으로 추가"}
            </li>
          </Show>
        </ul>
      </aside>

      {/* 중: 대화 */}
      <main class="messenger-thread">
        <Show
          when={selectedFriend()}
          fallback={
            <div class="messenger-thread-empty">
              <p>{t("messenger.select-friend") || "왼쪽에서 친구를 선택하세요."}</p>
              <p class="messenger-thread-hint">
                {t("messenger.thread-v01-hint") ||
                  "v0.1 — 친구 목록만. v0.2에서 메시지 스레드(L0) + 송신 라우팅 연결 예정."}
              </p>
            </div>
          }
        >
          {(f) => {
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
                  <h2>{f().display}</h2>
                  <small>{f().subtitle}</small>
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

  async function send() {
    const body = text().trim();
    if (!body) return;
    if (!isPeer()) {
      setError(t("messenger.send-peer-only") || "송신은 peer 친구에게만 가능 (Discord/Telegram 채널 송신은 별도)");
      return;
    }
    setSending(true);
    setError(null);
    try {
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
      <textarea
        rows={2}
        value={text()}
        onInput={(ev) => setText(ev.currentTarget.value)}
        placeholder={
          isPeer()
            ? (t("messenger.input-placeholder") || "메시지 입력 (Enter 보내기, Shift+Enter 줄바꿈)")
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
      <button type="button" disabled={!isPeer() || sending() || !text().trim()} onClick={() => void send()}>
        {sending() ? (t("messenger.sending") || "보내는 중…") : (t("messenger.send") || "보내기")}
      </button>
      <Show when={error()}>
        <div class="messenger-thread-error" role="alert">{error()}</div>
      </Show>
    </footer>
  );
}

