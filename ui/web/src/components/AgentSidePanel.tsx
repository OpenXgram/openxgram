import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// UI-MESSENGER-SPEC v1.3 §5 — 우측 12 탭 (S3 세로 사이드).
// Tier 3 MVP = 5 탭: 개요 · 역할 · 채널 바인딩 · 상태·리소스 · 지갑·결제.
// 나머지 7 탭 (히스토리·내보내기·토큰·Cron·파일·알림·권한) 은 Tier 4+.

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

type TabId =
  | "overview"
  | "role"
  | "channel"
  | "status"
  | "wallet";

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
    <aside
      class="messenger-sidepanel"
      style="display:flex; flex-direction:row; border-left:1px solid rgba(255,255,255,0.08); background:rgba(0,0,0,0.15);"
    >
      {/* 세로 사이드 탭 (S3) */}
      <nav style="display:flex; flex-direction:column; width:48px; padding:6px 0; border-right:1px solid rgba(255,255,255,0.06);">
        <For each={TABS}>
          {(t) => (
            <button
              type="button"
              onClick={() => setTab(t.id)}
              title={t.label}
              style={`padding:8px 0; font-size:1.1em; background:${tab() === t.id ? "rgba(96,165,250,0.18)" : "transparent"}; border:none; cursor:pointer;`}
            >
              {t.icon}
            </button>
          )}
        </For>
      </nav>

      {/* 탭 콘텐츠 */}
      <div style="flex:1; padding:12px; overflow-y:auto; font-size:0.9em;">
        <h3 style="margin:0 0 8px 0; font-size:1em;">
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
      <Row
        label="display_name"
        value={props.peer.alias /* 현재 별도 display_name X — alias 사용 */}
      />
      <Row label="machine" value={props.peer.machine || "(unknown)"} />
      <Row
        label="address"
        value={props.peer.address ? `${props.peer.address.slice(0, 18)}…` : "(없음)"}
        mono
      />
      <Row label="public_key" value={fingerprint(props.peer.public_key_hex)} mono />
      <Row
        label="last_seen"
        value={props.peer.last_seen || "한 번도 본 적 없음"}
      />
      <hr style="margin:12px 0; opacity:0.2;" />
      <p style="font-size:0.85em; opacity:0.7;">
        ULID Agent ID·display_name 편집·세션 마이그레이션 등은 Tier 4 에서.
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
      <hr style="margin:12px 0; opacity:0.2;" />
      <p style="font-size:0.85em; margin-bottom:6px;">
        <strong>L3 auto_respond</strong>
      </p>
      <Row label="정책" value="자율 행동 카드 기본값 따름" />
      <Row label="역할 default" value="true (researcher)" />
      <button
        type="button"
        onClick={props.onJumpToSettings}
        style="margin-top:8px; padding:6px 10px; font-size:0.85em;"
      >
        🔗 자율 행동 카드 (예정)
      </button>
      <hr style="margin:12px 0; opacity:0.2;" />
      <p style="font-size:0.85em; opacity:0.7;">
        역할 프리셋 변경·시스템 프롬프트 편집·호출 가능 대상은 Tier 4+.
      </p>
    </div>
  );
}

// ── 탭 3: 채널 바인딩 (안티패턴 1 — 토큰 입력 X) ────────────────
function ChannelTab(props: {
  notify: NotifyStatus | null;
  onJumpToSettings: () => void;
}) {
  return (
    <div>
      <p style="font-size:0.85em; margin-bottom:8px;">
        이 세션이 응답할 채널 — 📱 채널 카드 등록 후 여기서 바인딩 선택.
      </p>
      <Row
        label="디스코드"
        value={props.notify?.discord_configured ? "✓ 연결됨" : "(미연결)"}
      />
      <Row
        label="텔레그램"
        value={props.notify?.telegram_configured ? "✓ 연결됨" : "(미연결)"}
      />
      <button
        type="button"
        onClick={props.onJumpToSettings}
        style="margin-top:8px; padding:6px 10px; font-size:0.85em;"
      >
        🔗 채널 카드 (Settings → 알림 채널)
      </button>
      <hr style="margin:12px 0; opacity:0.2;" />
      <p style="font-size:0.85em; opacity:0.7;">
        세션별 멘션 트리거·권한 토글은 Tier 4+. <strong>봇 토큰 입력 X</strong>{" "}
        (마스터 = 📱 채널 카드).
      </p>
    </div>
  );
}

// ── 탭 4: 상태·리소스 (Tier 4 에서 실시간 데이터 연결) ──────────
function StatusTab(props: { peer: PeerMeta }) {
  return (
    <div>
      <Row label="last_seen" value={props.peer.last_seen || "—"} />
      <Row label="alias" value={props.peer.alias} />
      <hr style="margin:12px 0; opacity:0.2;" />
      <p style="font-size:0.85em; opacity:0.7;">
        실시간 리소스 (CPU·RAM·GPU·컨텍스트·서브에이전트 트리·heartbeat) 는
        Tier 4+ (daemon 측 텔레메트리 API 신설 필요).
      </p>
    </div>
  );
}

// ── 탭 5: 지갑·결제 (M-3 — 마스터=신원, 서브=메신저) ────────────
function WalletTab(props: { peer: PeerMeta }) {
  return (
    <div>
      <Row label="주소" value={props.peer.address.slice(0, 22) + "…"} mono />
      <p style="font-size:0.85em; opacity:0.7; margin-top:8px;">
        서브 지갑 (HD 파생) · 잔액 · 한도 정책 · M-6 자동 충전 은 Tier 4+
        (서브 지갑 백엔드 + 마스터 ↔ 서브 이체 API 신설 필요).
      </p>
      <hr style="margin:12px 0; opacity:0.2;" />
      <p style="font-size:0.85em;">
        <strong>마스터 지갑</strong> = 🔑 신원 카드 (예정).
      </p>
    </div>
  );
}

// ── 공용 ──
function Row(props: { label: string; value: string; mono?: boolean }) {
  return (
    <div style="display:flex; gap:8px; padding:3px 0; font-size:0.85em;">
      <span style="opacity:0.6; min-width:90px;">{props.label}</span>
      <span
        style={`flex:1; ${props.mono ? "font-family:monospace; font-size:0.85em;" : ""}`}
      >
        {props.value}
      </span>
    </div>
  );
}
