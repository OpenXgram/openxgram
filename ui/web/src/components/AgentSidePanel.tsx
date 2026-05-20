import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// UI-MESSENGER-SPEC v1.3 §5 — 우측 12 탭 (S3 세로 사이드).
// Tier 3 MVP = 5 탭: 개요 · 역할 · 채널 바인딩 · 상태·리소스 · 지갑·결제.
// 색은 styles.css 의 --surface-* / --text-* 변수 사용 (다크/라이트 자동).

interface PeerMeta {
  alias: string;
  address: string;
  public_key_hex: string;
  machine?: string;
  last_seen?: string;
}

interface NotifyStatus {
  discord_configured: boolean;
  telegram_configured: boolean;
}

type TabId = "overview" | "role" | "channel" | "status" | "wallet";

const TABS: { id: TabId; label: string; icon: string }[] = [
  { id: "overview", label: "개요", icon: "📋" },
  { id: "role", label: "역할", icon: "🎭" },
  { id: "channel", label: "채널 바인딩", icon: "📡" },
  { id: "status", label: "상태·리소스", icon: "📊" },
  { id: "wallet", label: "지갑·결제", icon: "💰" },
];

function fingerprint(pubkeyHex: string): string {
  const t = pubkeyHex.replace(/^0x/, "");
  return t.length < 16 ? t : `${t.slice(0, 8)}…${t.slice(-8)}`;
}

async function fetchNotify(): Promise<NotifyStatus | null> {
  try {
    return await invoke<NotifyStatus>("notify_status");
  } catch {
    return null;
  }
}

export function AgentSidePanel(props: {
  peer: PeerMeta;
  onJumpToSettings: () => void;
}) {
  const [tab, setTab] = createSignal<TabId>("overview");
  const [notify] = createResource(fetchNotify);

  return (
    <aside class="messenger-sidepanel">
      <nav class="messenger-sidepanel-nav">
        <For each={TABS}>
          {(tt) => (
            <button
              type="button"
              class={tab() === tt.id ? "active" : ""}
              onClick={() => setTab(tt.id)}
              title={tt.label}
            >
              {tt.icon}
            </button>
          )}
        </For>
      </nav>

      <div class="messenger-sidepanel-content">
        <h3>
          {TABS.find((t) => t.id === tab())?.icon}{" "}
          {TABS.find((t) => t.id === tab())?.label}
        </h3>
        <Show when={tab() === "overview"}>
          <Overview peer={props.peer} />
        </Show>
        <Show when={tab() === "role"}>
          <RoleTab peer={props.peer} onJumpToSettings={props.onJumpToSettings} />
        </Show>
        <Show when={tab() === "channel"}>
          <ChannelTab notify={notify()} onJumpToSettings={props.onJumpToSettings} />
        </Show>
        <Show when={tab() === "status"}>
          <StatusTab peer={props.peer} />
        </Show>
        <Show when={tab() === "wallet"}>
          <WalletTab peer={props.peer} />
        </Show>
      </div>
    </aside>
  );
}

// ── 탭 1: 개요 (L2 4-tuple) ─────────────────────────────────────
function Overview(props: { peer: PeerMeta }) {
  return (
    <div>
      <Row label="alias" value={props.peer.alias} />
      <Row label="display_name" value={props.peer.alias} />
      <Row label="machine" value={props.peer.machine || "(unknown)"} />
      <Row
        label="address"
        value={props.peer.address ? `${props.peer.address.slice(0, 18)}…` : "(없음)"}
        mono
      />
      <Row label="public_key" value={fingerprint(props.peer.public_key_hex)} mono />
      <Row label="last_seen" value={props.peer.last_seen || "한 번도 본 적 없음"} />
      <p class="messenger-sidepanel-hint">
        ULID Agent ID·display_name 편집·세션 마이그레이션 등은 Tier 4+.
      </p>
    </div>
  );
}

// ── 탭 2: 역할 (L3 auto_respond 뷰) ─────────────────────────────
function RoleTab(props: { peer: PeerMeta; onJumpToSettings: () => void }) {
  return (
    <div>
      <Row label="역할" value="researcher (기본)" />
      <Row label="오케스트레이션" value="워커" />
      <hr style="margin:10px 0; opacity:0.2;" />
      <strong style="font-size:12px;">L3 auto_respond</strong>
      <Row label="정책" value="자율 행동 카드 기본값 따름" />
      <Row label="역할 default" value="true (researcher)" />
      <button class="link-btn" type="button" onClick={props.onJumpToSettings}>
        🔗 자율 행동 카드 (예정)
      </button>
      <p class="messenger-sidepanel-hint">
        역할 프리셋 변경·시스템 프롬프트 편집·호출 가능 대상은 Tier 4+.
      </p>
    </div>
  );
}

// ── 탭 3: 채널 바인딩 (안티패턴 1 — 토큰 입력 X) ────────────────
function ChannelTab(props: { notify: NotifyStatus | null; onJumpToSettings: () => void }) {
  return (
    <div>
      <p style="font-size:12px; margin-bottom:8px;">
        이 세션이 응답할 채널 — 📱 채널 카드 등록 후 여기서 바인딩 선택.
      </p>
      <Row label="디스코드" value={props.notify?.discord_configured ? "✓ 연결됨" : "(미연결)"} />
      <Row label="텔레그램" value={props.notify?.telegram_configured ? "✓ 연결됨" : "(미연결)"} />
      <button class="link-btn" type="button" onClick={props.onJumpToSettings}>
        🔗 채널 카드 (Settings → 알림 채널)
      </button>
      <p class="messenger-sidepanel-hint">
        세션별 멘션 트리거·권한 토글은 Tier 4+. <strong>봇 토큰 입력 X</strong> (마스터 = 📱 채널 카드).
      </p>
    </div>
  );
}

// ── 탭 4: 상태·리소스 ──────────────────────────────────────────
function StatusTab(props: { peer: PeerMeta }) {
  return (
    <div>
      <Row label="last_seen" value={props.peer.last_seen || "—"} />
      <Row label="alias" value={props.peer.alias} />
      <p class="messenger-sidepanel-hint">
        실시간 리소스 (CPU·RAM·GPU·컨텍스트·서브에이전트 트리·heartbeat) 는 Tier 4+
        (daemon 측 텔레메트리 API 신설 필요).
      </p>
    </div>
  );
}

// ── 탭 5: 지갑·결제 ─────────────────────────────────────────────
function WalletTab(props: { peer: PeerMeta }) {
  return (
    <div>
      <Row label="주소" value={props.peer.address.slice(0, 22) + "…"} mono />
      <p class="messenger-sidepanel-hint">
        서브 지갑 (HD 파생) · 잔액 · 한도 정책 · M-6 자동 충전 은 Tier 4+ (서브 지갑 백엔드 +
        마스터 ↔ 서브 이체 API 신설 필요).
      </p>
      <p style="font-size:12px; margin-top:8px;">
        <strong>마스터 지갑</strong> = 🔑 신원 카드 (예정).
      </p>
    </div>
  );
}

// ── 공용 ──
function Row(props: { label: string; value: string; mono?: boolean }) {
  return (
    <div class="messenger-sidepanel-row">
      <span class="label">{props.label}</span>
      <span class={`value${props.mono ? " mono" : ""}`}>{props.value}</span>
    </div>
  );
}
