import { createResource, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// UI-CARDS-IDENTITY v1.1 §0~§2 — 홈 대시보드 8 카드 (4 가치 + 4 토대).
// 사용자가 unlock 직후 첫 화면. 카드 클릭 → 해당 탭/뷰.
// 사양 정본: PRD-OpenXgram v1.4 §0 + UI-CARDS-IDENTITY v1.1.

export type CardId =
  | "messenger"
  | "memory"
  | "external"
  | "channel"
  | "identity"
  | "autonomy"
  | "ops"
  | "vault";

interface CardDef {
  id: CardId;
  group: "value" | "foundation";
  icon: string;
  title: string;             // UI 단축 라벨
  prdName: string;           // PRD v1.4 §0 정식 명칭
  oneLine: string;           // 한 줄 정체성 (UI-CARDS-IDENTITY §2.X)
  implStatus: "ready" | "partial" | "placeholder";
}

const CARDS: CardDef[] = [
  {
    id: "messenger",
    group: "value",
    icon: "💬",
    title: "메신저",
    prdName: "에이전트간 메신저",
    oneLine: "모든 대화의 라이브 무대 + 사용자 개입",
    implStatus: "ready",
  },
  {
    id: "memory",
    group: "value",
    icon: "🧠",
    title: "기억",
    prdName: "기억·학습",
    oneLine: "내 AI의 위키피디아 + 패턴/실수 보드",
    implStatus: "partial",
  },
  {
    id: "external",
    group: "value",
    icon: "🌐",
    title: "외부 에이전트",
    prdName: "외부 에이전트·결제",
    oneLine: "다른 AI 시스템과의 게이트웨이 (마켓·A2A·ANP·x402)",
    implStatus: "partial",
  },
  {
    id: "channel",
    group: "value",
    icon: "📱",
    title: "채널",
    prdName: "인간 친화 채널",
    oneLine: "Discord·Telegram·Slack·Web — 사람과의 인박스",
    implStatus: "partial",
  },
  {
    id: "identity",
    group: "foundation",
    icon: "🔑",
    title: "신원",
    prdName: "자기주권 신원",
    oneLine: "DID · 마스터 지갑 · 잠금 (모든 카드의 기반)",
    implStatus: "partial",
  },
  {
    id: "autonomy",
    group: "foundation",
    icon: "⏰",
    title: "자율 행동",
    prdName: "자율 행동",
    oneLine: "Cron · Reflection · SelfTrigger (잠자는 동안 수익)",
    implStatus: "partial",
  },
  {
    id: "ops",
    group: "foundation",
    icon: "⚙️",
    title: "운영·생존",
    prdName: "운영·생존",
    oneLine: "Daemon · 머신 · 백업 (OpenXgram 자체)",
    implStatus: "partial",
  },
  {
    id: "vault",
    group: "foundation",
    icon: "🗝️",
    title: "도구·Vault·MCP",
    prdName: "도구·Vault·MCP",
    oneLine: "시크릿 · MCP 서버 · 도구 권한",
    implStatus: "partial",
  },
];

interface NotifyStatus {
  discord_configured: boolean;
  telegram_configured: boolean;
}

interface PeerDto {
  alias: string;
}

async function fetchSummary() {
  const [peers, notify, wikiPages, mcpServers, audit, autonomyHistory, status, externalDir] = await Promise.all([
    invoke<PeerDto[]>("peers_list").catch(() => [] as PeerDto[]),
    invoke<NotifyStatus>("notify_status").catch(
      () => ({ discord_configured: false, telegram_configured: false }) as NotifyStatus,
    ),
    invoke<any[]>("wiki_list").catch(() => []),
    invoke<any[]>("vault_mcp_servers_list").catch(() => []),
    invoke<any[]>("identity_audit").catch(() => []),
    invoke<any[]>("autonomy_history").catch(() => []),
    invoke<any>("status").catch(() => null),
    invoke<any>("external_directory").catch(() => null),
  ]);
  return {
    peerCount: peers.length,
    notify,
    wikiCount: wikiPages.length,
    mcpCount: mcpServers.length,
    auditCount: audit.length,
    autonomyCount: autonomyHistory.length,
    statusOk: !!status?.initialized || !!status?.alias,
    externalAgents: (externalDir?.external_agents ?? []).length,
  };
}

export function HomeDashboard(props: { onOpen: (id: CardId) => void }) {
  const [summary] = createResource(fetchSummary);

  function badge(card: CardDef): string {
    const s = summary();
    if (!s) return "";
    switch (card.id) {
      case "messenger":
        return s.peerCount > 0 ? `${s.peerCount} peer` : "peer 0";
      case "channel": {
        const n = (s.notify.discord_configured ? 1 : 0) + (s.notify.telegram_configured ? 1 : 0);
        return n > 0 ? `${n} 채널 연결` : "미연결";
      }
      default:
        return "";
    }
  }

  function dynImpl(card: CardDef): "ready" | "partial" | "placeholder" {
    const s = summary();
    if (!s) return card.implStatus;
    switch (card.id) {
      case "messenger": return s.peerCount > 0 ? "ready" : "partial";
      case "memory": return s.wikiCount > 0 ? "ready" : "partial";
      case "external": return s.externalAgents > 0 ? "ready" : "placeholder";
      case "channel": {
        const n = (s.notify.discord_configured ? 1 : 0) + (s.notify.telegram_configured ? 1 : 0);
        return n >= 2 ? "ready" : n === 1 ? "partial" : "placeholder";
      }
      case "identity": return s.statusOk ? (s.auditCount > 0 ? "ready" : "partial") : "placeholder";
      case "autonomy": return s.autonomyCount > 0 ? "ready" : "partial";
      case "vault": return s.mcpCount > 0 ? "ready" : "partial";
      case "ops": return s.statusOk ? "partial" : "placeholder";
      default: return card.implStatus;
    }
  }
  function statusLabel(s: "ready" | "partial" | "placeholder"): string {
    return s === "ready" ? "✅ 사용 가능" : s === "partial" ? "🟡 일부 구현" : "⏳ 예정";
  }

  return (
    <div class="home-dashboard">
      <header class="home-dashboard-head">
        <h1>OpenXgram 홈</h1>
        <p class="muted">
          8 카드 = <strong>4 가치 + 4 토대</strong> · PRD v1.4 §0
        </p>
      </header>

      <section class="home-dashboard-group">
        <h2>가치 (보는 것)</h2>
        <div class="home-cards-grid">
          <For each={CARDS.filter((c) => c.group === "value")}>
            {(c) => (
              <CardTile
                card={c}
                badge={badge(c)}
                status={statusLabel(c.implStatus)}
                onClick={() => props.onOpen(c.id)}
              />
            )}
          </For>
        </div>
      </section>

      <section class="home-dashboard-group">
        <h2>토대 (지원하는 것)</h2>
        <div class="home-cards-grid">
          <For each={CARDS.filter((c) => c.group === "foundation")}>
            {(c) => (
              <CardTile
                card={c}
                badge={badge(c)}
                status={statusLabel(c.implStatus)}
                onClick={() => props.onOpen(c.id)}
              />
            )}
          </For>
        </div>
      </section>
    </div>
  );
}

function CardTile(props: {
  card: CardDef;
  badge: string;
  status: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      class={`home-card status-${props.card.implStatus}`}
      onClick={props.onClick}
    >
      <div class="home-card-icon">{props.card.icon}</div>
      <div class="home-card-title">{props.card.title}</div>
      <div class="home-card-prd">{props.card.prdName}</div>
      <div class="home-card-oneline">{props.card.oneLine}</div>
      <div class="home-card-foot">
        <span class="home-card-status">{props.status}</span>
        <Show when={props.badge}>
          <span class="home-card-badge">{props.badge}</span>
        </Show>
      </div>
    </button>
  );
}
