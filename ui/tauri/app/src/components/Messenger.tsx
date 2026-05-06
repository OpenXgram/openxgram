import { createMemo, createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

// v0.1 — 통합 메신저 허브 첫 컷.
//   좌측: 친구 목록 = OpenXgram peer + Discord/Telegram 연결 상태 통합
//   중앙: 선택된 대화 (현재는 placeholder, v0.2에서 L0 messages 스레드 + 송신 라우팅 연결)
//   친구 추가 = `xgram peer add` / `setup discord` / `setup telegram`. 메신저에서 "친구 추가" 버튼이 그 흐름을 연다.

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
          {(f) => (
            <>
              <header class="messenger-thread-head">
                <h2>{f().display}</h2>
                <small>{f().subtitle}</small>
              </header>
              <section class="messenger-thread-body">
                <div class="messenger-placeholder">
                  {t("messenger.thread-placeholder") ||
                    `${f().display} 와의 대화 — v0.2에서 L0 messages 스레드를 시간순 노출.`}
                </div>
              </section>
              <footer class="messenger-thread-input">
                <textarea
                  rows={2}
                  placeholder={
                    t("messenger.input-placeholder") ||
                    "메시지를 입력 (v0.2에서 transport 자동 라우팅)"
                  }
                  disabled
                />
                <button type="button" disabled>
                  {t("messenger.send") || "보내기"}
                </button>
              </footer>
            </>
          )}
        </Show>
      </main>
    </div>
  );
}
