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

type TabId =
  | "overview"
  | "role"
  | "channel"
  | "status"
  | "history"
  | "export"
  | "wallet"
  | "tokens"
  | "cron"
  | "files"
  | "notify"
  | "permissions";

// 사양 §5 12 탭 (S3 세로 사이드).
const TABS: { id: TabId; label: string; icon: string }[] = [
  { id: "overview", label: "개요", icon: "📋" },
  { id: "role", label: "역할", icon: "🎭" },
  { id: "channel", label: "채널 바인딩", icon: "📡" },
  { id: "status", label: "상태·리소스", icon: "📊" },
  { id: "history", label: "히스토리", icon: "📜" },
  { id: "export", label: "내보내기", icon: "📤" },
  { id: "wallet", label: "지갑·결제", icon: "💰" },
  { id: "tokens", label: "토큰", icon: "🔢" },
  { id: "cron", label: "Cron", icon: "⏰" },
  { id: "files", label: "파일·지침", icon: "📁" },
  { id: "notify", label: "알림", icon: "🔔" },
  { id: "permissions", label: "권한·도구·MCP", icon: "🛡️" },
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
        <Show when={tab() === "history"}>
          <HistoryTab peer={props.peer} />
        </Show>
        <Show when={tab() === "export"}>
          <ExportTab peer={props.peer} />
        </Show>
        <Show when={tab() === "wallet"}>
          <WalletTab peer={props.peer} />
        </Show>
        <Show when={tab() === "tokens"}>
          <TokensTab peer={props.peer} />
        </Show>
        <Show when={tab() === "cron"}>
          <CronTab onJumpToSettings={props.onJumpToSettings} />
        </Show>
        <Show when={tab() === "files"}>
          <FilesTab peer={props.peer} />
        </Show>
        <Show when={tab() === "notify"}>
          <NotifyTab notify={notify()} />
        </Show>
        <Show when={tab() === "permissions"}>
          <PermissionsTab onJumpToSettings={props.onJumpToSettings} />
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

// ── 탭 2: 역할 (L3 + V1 auto_respond 마스터 정책 view) ─────────
interface RolePolicyItem {
  role: string;
  auto_respond_default: boolean;
  max_concurrent: number;
}
interface RolePolicyDto {
  master_card: string;
  roles: RolePolicyItem[];
}
async function fetchRolePolicies(): Promise<RolePolicyDto | null> {
  try {
    return await invoke<RolePolicyDto>("role_policies");
  } catch {
    return null;
  }
}
function RoleTab(props: { peer: PeerMeta; onJumpToSettings: () => void }) {
  const [policies] = createResource(fetchRolePolicies);
  return (
    <div>
      <Row label="현재 역할" value="researcher (기본)" />
      <Row label="오케스트레이션" value="워커" />
      <hr style="margin:10px 0; opacity:0.2;" />
      <strong style="font-size:12px;">L3 + V1 — 역할별 auto_respond 마스터 정책</strong>
      <p class="messenger-sidepanel-hint">
        마스터 = {policies()?.master_card ?? "⏰ 자율 행동 카드"}. 본 탭은 view.
      </p>
      <For each={policies()?.roles ?? []}>
        {(r) => (
          <div style="display:flex; justify-content:space-between; padding:3px 0; font-size:12px; border-bottom:1px dashed var(--border);">
            <span>{r.role}</span>
            <span style={r.auto_respond_default ? "color:#5fa;" : "color:var(--text-3);"}>
              {r.auto_respond_default ? "✓ auto" : "× manual"} · max {r.max_concurrent}
            </span>
          </div>
        )}
      </For>
      <button class="link-btn" type="button" onClick={props.onJumpToSettings} style="margin-top:10px;">
        🔗 자율 행동 카드 (마스터 편집)
      </button>
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

// ── 탭 5: 지갑·결제 (M-3 + M-6 + L4 + S6 + V8) ─────────────────
interface SubWalletDto {
  agent_id: string;
  derivation_index: number;
  derived_address: string;
  allocated_micro: number;
  spent_micro: number;
  earned_micro: number;
  balance_micro: number;
  daily_limit_micro: number;
  monthly_limit_micro: number;
  auto_topup_enabled: boolean;
  auto_topup_threshold_micro: number;
  auto_topup_amount_micro: number;
  status: string;
}
interface WalletsDto {
  master: { address: string | null; free_micro: number; last_synced_at: string };
  sub_wallets: SubWalletDto[];
  next_hd_index: number;
}
async function fetchWallets(): Promise<WalletsDto | null> {
  try {
    return await invoke<WalletsDto>("wallets_list");
  } catch {
    return null;
  }
}
function fmtUsd(micro: number): string {
  return `$${(micro / 1_000_000).toFixed(2)}`;
}
function WalletTab(props: { peer: PeerMeta }) {
  const [w, { refetch }] = createResource(fetchWallets);
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  const ownWallet = () =>
    w()?.sub_wallets.find((s) => s.agent_id === props.peer.alias) || null;
  async function createWallet() {
    setBusy(true);
    setErr(null);
    try {
      await invoke("wallet_create", { agent_id: props.peer.alias });
      await refetch();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function topup(amountUsd: number) {
    setBusy(true);
    setErr(null);
    try {
      await invoke("wallet_topup", {
        agent_id: props.peer.alias,
        amount_micro: Math.round(amountUsd * 1_000_000),
      });
      await refetch();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div>
      <strong style="font-size:12px;">마스터 지갑 (🔑 신원)</strong>
      <Row label="주소" value={w()?.master.address || "(미설정)"} mono />
      <Row label="free 잔액" value={w() ? fmtUsd(w()!.master.free_micro) : "—"} />
      <hr style="margin:10px 0; opacity:0.2;" />
      <strong style="font-size:12px;">서브 지갑 (m/44'/.../N)</strong>
      <Show
        when={ownWallet()}
        fallback={
          <div>
            <p class="messenger-sidepanel-hint">이 에이전트의 서브 지갑이 없습니다. L4 next index = {w()?.next_hd_index ?? "—"} (영구 점유).</p>
            <button class="link-btn" type="button" onClick={createWallet} disabled={busy()}>
              + 서브 지갑 생성 (HD 자동 할당)
            </button>
          </div>
        }
      >
        {(s) => (
          <>
            <Row label="HD index" value={`m/44'/.../${s().derivation_index} (L4 영구)`} />
            <Row label="주소" value={s().derived_address.slice(0, 22) + "…"} mono />
            <Row label="allocated" value={fmtUsd(s().allocated_micro)} />
            <Row label="spent (S6 합산)" value={fmtUsd(s().spent_micro)} />
            <Row label="earned" value={fmtUsd(s().earned_micro)} />
            <Row label="balance" value={fmtUsd(s().balance_micro)} />
            <hr style="margin:8px 0; opacity:0.2;" />
            <Row label="일 한도 (S6)" value={fmtUsd(s().daily_limit_micro)} />
            <Row label="월 한도" value={fmtUsd(s().monthly_limit_micro)} />
            <Row
              label="M-6 자동 충전"
              value={s().auto_topup_enabled ? "✓ 활성" : "비활성"}
            />
            <hr style="margin:8px 0; opacity:0.2;" />
            <strong style="font-size:12px;">V8 — 마스터 → 서브 이체</strong>
            <div style="display:flex; gap:6px; margin-top:6px;">
              <button class="link-btn" type="button" onClick={() => topup(1)} disabled={busy()}>↑ $1</button>
              <button class="link-btn" type="button" onClick={() => topup(5)} disabled={busy()}>↑ $5</button>
              <button class="link-btn" type="button" onClick={() => topup(10)} disabled={busy()}>↑ $10</button>
            </div>
          </>
        )}
      </Show>
      <Show when={err()}>
        <p style="color:#f88; font-size:11px; margin-top:8px;">⚠ {err()}</p>
      </Show>
      <p class="messenger-sidepanel-hint">
        L4: derivation_index 영구 점유 (Decommissioned 도 재사용 X). 마스터 지갑 고급 = 🔑 신원 카드.
      </p>
    </div>
  );
}

// ── 탭 5: 히스토리 (사양 §5 탭 5) — /v1/gui/messages 활용 ──
interface MessageItem {
  id: string;
  sender: string;
  body: string;
  timestamp: string;
}
async function fetchMessages(): Promise<MessageItem[]> {
  try {
    return await invoke<MessageItem[]>("messages_recent", { limit: 50 });
  } catch {
    return [];
  }
}
function HistoryTab(props: { peer: PeerMeta }) {
  const [msgs] = createResource(fetchMessages);
  return (
    <div>
      <p style="font-size:12px; margin-bottom:8px;">
        이 에이전트가 관여한 최근 메시지 (peer messages_recent 필터).
      </p>
      <Show when={(msgs() ?? []).length === 0} fallback={null}>
        <p class="messenger-sidepanel-hint">메시지 없음.</p>
      </Show>
      <For
        each={(msgs() ?? [])
          .filter(
            (m) =>
              m.sender === props.peer.alias ||
              m.sender === props.peer.address?.toLowerCase(),
          )
          .slice(0, 15)}
      >
        {(m) => (
          <div style="border-bottom:1px solid var(--border); padding:6px 0; font-size:12px;">
            <div style="color:var(--text-3); font-size:10px;">{m.timestamp}</div>
            <div>{m.body.slice(0, 100)}{m.body.length > 100 ? "…" : ""}</div>
          </div>
        )}
      </For>
      <p class="messenger-sidepanel-hint">
        시간 범위·검색·미연결 시기 명령 포함 (사양 §5 탭 5) — 백엔드 history API 신설 시 확장.
      </p>
    </div>
  );
}

// ── 탭 6: 내보내기 (사양 §5 탭 6) — 클라이언트 측 ──
function ExportTab(props: { peer: PeerMeta }) {
  const [fmt, setFmt] = createSignal<"md" | "json" | "txt">("md");
  async function doExport() {
    const msgs = await fetchMessages();
    const filtered = msgs.filter((m) => m.sender === props.peer.alias);
    let body = "";
    if (fmt() === "json") {
      body = JSON.stringify(filtered, null, 2);
    } else if (fmt() === "md") {
      body = filtered
        .map((m) => `**${m.sender}** _(${m.timestamp})_\n${m.body}\n`)
        .join("\n---\n");
    } else {
      body = filtered.map((m) => `[${m.timestamp}] ${m.sender}: ${m.body}`).join("\n");
    }
    const blob = new Blob([body], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${props.peer.alias}-export.${fmt() === "md" ? "md" : fmt() === "json" ? "json" : "txt"}`;
    a.click();
    URL.revokeObjectURL(url);
  }
  return (
    <div>
      <div style="display:flex; gap:6px; margin-bottom:8px;">
        <label><input type="radio" name="fmt" checked={fmt() === "md"} onChange={() => setFmt("md")} /> Markdown</label>
        <label><input type="radio" name="fmt" checked={fmt() === "json"} onChange={() => setFmt("json")} /> JSON</label>
        <label><input type="radio" name="fmt" checked={fmt() === "txt"} onChange={() => setFmt("txt")} /> Text</label>
      </div>
      <button class="link-btn" type="button" onClick={doExport}>💾 다운로드</button>
      <p class="messenger-sidepanel-hint">
        시스템 프롬프트·도구 호출 raw 포함 토글, 이어가기 프롬프트 생성 (사양 §5 탭 6) — 백엔드 export API 확장 시.
      </p>
    </div>
  );
}

// ── 탭 8: 토큰 (사양 §5 탭 8, S6 합산) ──
function TokensTab(props: { peer: PeerMeta }) {
  return (
    <div>
      <Row label="LLM 토큰 합계 (24h)" value="(데이터 없음)" />
      <Row label="x402 결제 합계" value="(데이터 없음)" />
      <Row label="세션 비용 합산" value="(데이터 없음)" />
      <p class="messenger-sidepanel-hint">
        S6 합산 정책 (LLM 토큰비 + x402 결제). 백엔드 token_usage 테이블 + GET /v1/gui/sessions/{id}/tokens 신설 필요.
      </p>
    </div>
  );
}

// ── 탭 9: Cron (사양 §5 탭 9) — 기존 /v1/gui/schedule 활용 ──
interface ScheduleItem {
  id: string;
  expr: string;
  task: string;
}
async function fetchSchedule(): Promise<ScheduleItem[]> {
  try {
    return await invoke<ScheduleItem[]>("schedule_list");
  } catch {
    return [];
  }
}
function CronTab(props: { onJumpToSettings: () => void }) {
  const [items] = createResource(fetchSchedule);
  return (
    <div>
      <p style="font-size:12px; margin-bottom:8px;">이 세션의 활성 스케줄. (현재는 daemon 전체 cron 표시)</p>
      <Show when={(items() ?? []).length === 0} fallback={null}>
        <p class="messenger-sidepanel-hint">스케줄 없음.</p>
      </Show>
      <For each={items() ?? []}>
        {(s) => (
          <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);">
            <div><strong>{s.expr}</strong></div>
            <div style="color:var(--text-3);">{s.task}</div>
          </div>
        )}
      </For>
      <button class="link-btn" type="button" onClick={props.onJumpToSettings}>
        🔗 자율 행동 카드 (모든 cron)
      </button>
    </div>
  );
}

// ── 탭 10: 파일·지침 (사양 §5 탭 10) ──
function FilesTab(props: { peer: PeerMeta }) {
  return (
    <div>
      <Row label="작업 디렉토리" value="(미상 — peer machine 확장 필요)" />
      <Row label="git 상태" value="(데이터 없음)" />
      <p class="messenger-sidepanel-hint">
        파일 트리 (최대 5단) · CLAUDE.md / AGENTS.md / PRD 참조 마킹. 백엔드 GET /v1/gui/sessions/{id}/files 신설 필요.
      </p>
    </div>
  );
}

// ── 탭 11: 알림 (사양 §5 탭 11) ──
function NotifyTab(props: { notify: NotifyStatus | null }) {
  return (
    <div>
      <Row label="디스코드" value={props.notify?.discord_configured ? "✓ 연결됨" : "(미연결)"} />
      <Row label="텔레그램" value={props.notify?.telegram_configured ? "✓ 연결됨" : "(미연결)"} />
      <p class="messenger-sidepanel-hint">
        규칙 추가 (비용 한도 80%·1h 무응답·서브에이전트 3회 실패·Cron 실패) — 백엔드 notification_rules 테이블 신설 필요.
      </p>
    </div>
  );
}

// ── 탭 12: 권한·도구·MCP (사양 §5 탭 12, V9 default-deny) ──
function PermissionsTab(props: { onJumpToSettings: () => void }) {
  return (
    <div>
      <strong style="font-size:12px;">도구 권한 (현재 default-deny)</strong>
      <Row label="파일 read" value="✓" />
      <Row label="파일 write (cwd)" value="✓" />
      <Row label="파일 delete" value="✗" />
      <Row label="shell 실행" value="✓" />
      <Row label="네트워크 (allowlist)" value="✓" />
      <Row label="외부 LLM 호출" value="✓" />
      <Row label="결제 (서브 지갑 한도)" value="✓" />
      <hr style="margin:10px 0; opacity:0.2;" />
      <strong style="font-size:12px;">MCP 서버 (🗝️ Vault·MCP 카드)</strong>
      <p class="messenger-sidepanel-hint">
        외부 DID allowlist (N9 default-deny) 마스터 = 🔑 신원 카드. 세션 override 불가 (V9).
      </p>
      <button class="link-btn" type="button" onClick={props.onJumpToSettings}>
        🔗 도구·Vault·MCP 카드
      </button>
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
