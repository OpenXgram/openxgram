import { createResource, For, Show} from "solid-js";
import { invoke} from "@/api/client";

// UI-CARDS-IDENTITY v1.1 §0~§2 — 홈 대시보드 8 카드 (4 가치 + 4 토대).
// 사용자가 unlock 직후 첫 화면. 카드 클릭 → 해당 탭/뷰.
// 사양 정본: PRD-OpenXgram v1.4 §0 + UI-CARDS-IDENTITY v1.1.

export type CardId =
 | "messenger"
 | "workflows"
 | "memory"
 | "external"
 | "identity"
 | "autonomy"
 | "ops"
 | "vault";

interface CardDef {
 id: CardId;
 group: "value" | "foundation";
 icon: string;
 title: string; // UI 단축 라벨
 prdName: string; // PRD v1.4 §0 정식 명칭
 oneLine: string; // 한 줄 정체성 (UI-CARDS-IDENTITY §2.X)
 implStatus: "ready" | "partial" | "placeholder";
}

const CARDS: CardDef[] = [
 {
 id: "messenger",
 group: "value",
 icon: "",
 title: "메신저",
 prdName: "에이전트간 메신저",
 oneLine: "모든 대화의 라이브 무대 + 사용자 개입",
 implStatus: "ready",
},
 {
 id: "workflows",
 group: "value",
 icon: "",
 title: "오케스트레이션",
 prdName: "워크플로우 / 오케스트레이션",
 oneLine: "여러 에이전트 sequential/parallel/DAG 구성 + 실행 + monitoring",
 implStatus: "ready",
},
 {
 id: "memory",
 group: "value",
 icon: "",
 title: "기억",
 prdName: "기억·학습",
 oneLine: "내 AI의 위키피디아 + 패턴/실수 보드",
 implStatus: "partial",
},
 {
 id: "external",
 group: "value",
 icon: "",
 title: "외부 에이전트",
 prdName: "외부 에이전트·결제",
 oneLine: "다른 AI 시스템과의 게이트웨이 (마켓·A2A·ANP·x402)",
 implStatus: "partial",
},
 {
 id: "identity",
 group: "foundation",
 icon: "",
 title: "신원",
 prdName: "자기주권 신원",
 oneLine: "DID · 마스터 지갑 · 잠금 (모든 카드의 기반)",
 implStatus: "partial",
},
 {
 id: "autonomy",
 group: "foundation",
 icon: "",
 title: "자율 행동",
 prdName: "자율 행동",
 oneLine: "Cron · Reflection · SelfTrigger (잠자는 동안 수익)",
 implStatus: "partial",
},
 {
 id: "ops",
 group: "foundation",
 icon: "",
 title: "운영·생존",
 prdName: "운영·생존",
 oneLine: "Daemon · 머신 · 백업 (OpenXgram 자체)",
 implStatus: "partial",
},
 {
 id: "vault",
 group: "foundation",
 icon: "",
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
 const [peers, sessions, notify, wikiPages, mcpServers, audit, autonomyHistory, status, externalDir] = await Promise.all([
 invoke<PeerDto[]>("peers_list").catch(() => [] as PeerDto[]),
 invoke<any>("sessions_list").catch(() => null),
 invoke<NotifyStatus>("notify_status").catch(
 () => ({ discord_configured: false, telegram_configured: false}) as NotifyStatus,
),
 invoke<any[]>("wiki_pages_list").catch(() => []),
 invoke<any[]>("vault_mcp_servers_list").catch(() => []),
 invoke<any[]>("identity_audit").catch(() => []),
 invoke<any[]>("autonomy_history").catch(() => []),
 invoke<any>("status").catch(() => null),
 invoke<any>("external_directory").catch(() => null),
 ]);
 const sessList = (sessions?.sessions ?? []) as any[];
 const tmuxCount = sessList.filter((s: any) => s.kind === "tmux").length;
 const claudeCount = sessList.filter((s: any) => s.kind === "claude_project").length;
 return {
 peerCount: peers.length,
 sessionCount: sessList.length,
 tmuxCount,
 claudeCount,
 notify,
 wikiCount: wikiPages.length,
 mcpCount: mcpServers.length,
 auditCount: audit.length,
 autonomyCount: autonomyHistory.length,
 statusOk: !!status?.initialized || !!status?.alias,
 externalAgents: (externalDir?.external_agents ?? []).length,
};
}

export function HomeDashboard(props: { onOpen: (id: CardId) => void}) {
 const [summary] = createResource(fetchSummary);

 function badge(card: CardDef): string {
 const s = summary();
 if (!s) return "";
 switch (card.id) {
 case "messenger": {
 const parts: string[] = [];
 if (s.tmuxCount) parts.push(`tmux ${s.tmuxCount}`);
 if (s.claudeCount) parts.push(`Claude ${s.claudeCount}`);
 if (s.peerCount) parts.push(`P2P peer ${s.peerCount}`);
 return parts.length ? parts.join(" · ") : "세션 없음";
 }
 case "memory": return `${s.wikiCount} 위키`;
 case "external": return `${s.externalAgents} 에이전트`;
 case "identity": return `${s.auditCount} audit`;
 case "autonomy": return `${s.autonomyCount} event`;
 case "vault": return `${s.mcpCount} MCP`;
 case "ops": return s.statusOk ? " daemon" : " down";
 default: return "";
}
}

 function dynImpl(card: CardDef): "ready" | "partial" | "placeholder" {
 const s = summary();
 if (!s) return "partial";
 // endpoint 응답 OK 하면 ready (UI 작동 가능). 데이터 개수 무관.
 // 진짜 미연결 (statusOk=false) 인 경우만 placeholder.
 if (!s.statusOk) return "placeholder";
 return "ready";
}
 function statusLabel(s: "ready" | "partial" | "placeholder"): string {
 return s === "ready" ? " 사용 가능" : s === "partial" ? " 일부 구현" : "⏳ 예정";
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
 status={statusLabel(dynImpl(c))}
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
 status={statusLabel(dynImpl(c))}
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
