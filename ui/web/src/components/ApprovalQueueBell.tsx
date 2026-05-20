import { createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { invoke } from "@/api/client";

// UI-MESSENGER-SPEC v1.3 §7.1 + §7.3 + V4 — 헤더 🔔 통합 승인 큐.
// L6 차등 만료: payment 24h / pending_session 7d / risky_action 1h /
// external_call 24h / channel_moderation 7d. 만료 시 자동 거절.
// V4: 화이트리스트 매칭 시 자동 승인 가능 (단 payment·risky_action 제외).

interface ApprovalItem {
  id: string;
  kind: string;
  title: string;
  detail: string;
  created_at: string;
  expires_at: string;
  source_card: string;
}
interface ApprovalQueueDto {
  items: ApprovalItem[];
  policy: {
    payment_ttl_hours: number;
    pending_session_ttl_hours: number;
    risky_action_ttl_hours: number;
    external_call_ttl_hours: number;
    channel_moderation_ttl_hours: number;
    auto_approve_on_whitelist_match: boolean;
    never_auto_approve: string[];
  };
}

async function fetchApprovals(): Promise<ApprovalQueueDto | null> {
  try {
    return await invoke<ApprovalQueueDto>("approvals");
  } catch {
    return null;
  }
}

function kindLabel(k: string): string {
  return {
    payment: "💰 결제",
    pending_session: "🆕 미연결 세션 등록",
    risky_action: "⚠️ 위험 동작",
    external_call: "🌐 외부 호출",
    channel_moderation: "🛡️ 채널 모더레이션",
  }[k] || k;
}

function ttlRemaining(iso: string): string {
  const ms = Date.parse(iso) - Date.now();
  if (ms <= 0) return "만료";
  const h = Math.floor(ms / 3600000);
  if (h > 24) return `${Math.floor(h / 24)}d`;
  if (h >= 1) return `${h}h`;
  const m = Math.floor(ms / 60000);
  return `${m}m`;
}

export function ApprovalQueueBell() {
  const [open, setOpen] = createSignal(false);
  const [q, { refetch }] = createResource(fetchApprovals);
  const t = setInterval(() => void refetch(), 10000); // 10s 폴링
  onCleanup(() => clearInterval(t));

  const count = () => q()?.items.length ?? 0;

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(!open())}
        title="승인 큐 (L6 차등 만료)"
        style={
          "background:transparent; border:1px solid var(--border); border-radius:4px;" +
          " padding:4px 10px; cursor:pointer; color:var(--text-1); font-size:13px;"
        }
      >
        🔔 {count() > 0 ? <strong>{count()}</strong> : "0"}
      </button>
      <Show when={open()}>
        <div
          onClick={() => setOpen(false)}
          style="position:fixed; inset:0; background:rgba(0,0,0,0.4); z-index:50;"
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={
              "position:fixed; top:64px; right:20px; width:440px; max-height:70vh;" +
              " overflow:auto; background:var(--surface-1); color:var(--text-1);" +
              " border:1px solid var(--border); border-radius:8px; padding:16px;" +
              " box-shadow:0 12px 32px rgba(0,0,0,0.3);"
            }
          >
            <h3 style="margin:0 0 12px;">🔔 승인 큐 — UI-MESSENGER-SPEC v1.3 §7</h3>
            <Show when={count() === 0}>
              <p style="color:var(--text-3);">대기 항목 없음.</p>
            </Show>
            <For each={q()?.items ?? []}>
              {(it) => (
                <div
                  style={
                    "border-bottom:1px solid var(--border); padding:8px 0;" +
                    " display:flex; justify-content:space-between; gap:8px;"
                  }
                >
                  <div style="flex:1;">
                    <div style="font-weight:600;">{kindLabel(it.kind)}</div>
                    <div style="font-size:12px; margin-top:2px;">{it.title}</div>
                    <div style="font-size:11px; color:var(--text-3); margin-top:2px;">
                      {it.detail}
                    </div>
                    <div style="font-size:11px; color:var(--text-3); margin-top:2px;">
                      만료까지 {ttlRemaining(it.expires_at)} · source:{" "}
                      {it.source_card}
                    </div>
                  </div>
                </div>
              )}
            </For>
            <div
              style={
                "margin-top:12px; padding-top:8px; border-top:1px solid var(--border);" +
                " font-size:11px; color:var(--text-3);"
              }
            >
              L6 차등 만료: 결제 {q()?.policy.payment_ttl_hours}h ·
              미연결 등록 {q()?.policy.pending_session_ttl_hours}h ·
              위험 {q()?.policy.risky_action_ttl_hours}h ·
              외부 호출 {q()?.policy.external_call_ttl_hours}h ·
              채널 모더레이션 {q()?.policy.channel_moderation_ttl_hours}h.
              V4: 화이트리스트 자동 승인 — 단 결제·위험은 절대 자동 X.
            </div>
          </div>
        </div>
      </Show>
    </>
  );
}
