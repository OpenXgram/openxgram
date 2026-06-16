import { createSignal, createResource, Show, For } from "solid-js";
import { invoke } from "../api/client";

// rc.335 Phase 4b — "에이전트 추가"(남의 에이전트 사용) 상호 동의 inbox.
//   소유자(owner) 뷰: 받은 사용 요청(incoming, pending)을 보고 가격을 책정해 수락 또는 거절.
//   요청자(requester) 뷰: 내가 보낸 요청(outgoing)의 상태(대기/수락+가격/거절)를 확인.
// 카카오톡 셸 스타일 — 표 금지(목록만), 모달 재사용 패턴.

interface AgentRequest {
  id: string;
  requester: string;
  requester_machine?: string | null;
  target_agent: string;
  target_owner?: string | null;
  target_machine?: string | null;
  status: string; // pending|accepted|rejected|revoked
  price_amount?: number | null;
  price_unit?: string | null;
  currency?: string | null;
  terms?: string | null;
  direction: string; // incoming|outgoing
  created_at_kst: string;
  decided_at_kst?: string | null;
}

const PRICE_UNITS: { v: string; label: string }[] = [
  { v: "per_call", label: "호출당 (per_call)" },
  { v: "per_token", label: "토큰당 (per_token)" },
  { v: "subscription", label: "구독 (subscription)" },
  { v: "flat", label: "정액 (flat)" },
];

function statusBadge(s: string): { text: string; bg: string; fg: string } {
  switch (s) {
    case "accepted": return { text: "수락됨", bg: "#1d2a20", fg: "#3fb950" };
    case "rejected": return { text: "거절됨", bg: "#2a1d1d", fg: "#f85149" };
    case "revoked": return { text: "철회됨", bg: "#26221a", fg: "#d29922" };
    default: return { text: "대기 중", bg: "#1a2230", fg: "#58a6ff" };
  }
}

export function AgentRequestsInbox(props: { onClose: () => void }) {
  const [tab, setTab] = createSignal<"incoming" | "outgoing">("incoming");
  const [reqs, { refetch }] = createResource(
    () => tab(),
    async (role) => {
      const r = await invoke<{ requests?: AgentRequest[] }>("agent_requests_list", { role });
      return Array.isArray(r?.requests) ? r.requests : [];
    },
  );

  // 수락 모달 상태 — 책정 가격 입력.
  const [acceptId, setAcceptId] = createSignal<string | null>(null);
  const [priceAmount, setPriceAmount] = createSignal("");
  const [priceUnit, setPriceUnit] = createSignal("per_call");
  const [currency, setCurrency] = createSignal("USDC");
  const [terms, setTerms] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  function openAccept(id: string) {
    setAcceptId(id);
    setPriceAmount("");
    setPriceUnit("per_call");
    setCurrency("USDC");
    setTerms("");
    setErr(null);
  }

  async function confirmAccept() {
    const id = acceptId();
    if (!id) return;
    const amt = parseFloat(priceAmount());
    if (!isFinite(amt) || amt < 0) { setErr("0 이상의 가격을 입력하세요."); return; }
    setBusy(true);
    setErr(null);
    try {
      await invoke("agent_request_accept", {
        id,
        price_amount: amt,
        price_unit: priceUnit(),
        currency: currency().trim() || "USDC",
        terms: terms().trim() || undefined,
      });
      setAcceptId(null);
      await refetch();
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  async function decide(id: string, action: "reject" | "revoke") {
    setBusy(true);
    try {
      await invoke(action === "reject" ? "agent_request_reject" : "agent_request_revoke", { id });
      await refetch();
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="modal" style="max-width:480px;">
        <h2>🤝 에이전트 사용 요청</h2>
        <p class="sub">받은 요청은 가격을 책정해 수락하거나 거절합니다. 보낸 요청은 상태를 확인합니다.</p>

        <div class="seg" style="margin-bottom:10px;">
          <div class={`s${tab() === "incoming" ? " on" : ""}`} onClick={() => setTab("incoming")}>
            📥 받은 요청 (소유자)
          </div>
          <div class={`s${tab() === "outgoing" ? " on" : ""}`} onClick={() => setTab("outgoing")}>
            📤 보낸 요청 (내 상태)
          </div>
        </div>

        <Show when={err()}>
          <div style="color:#ff6b6b;font-size:12px;margin:6px 0;">⚠ {err()}</div>
        </Show>

        <Show when={!reqs.loading} fallback={<div class="hint">불러오는 중…</div>}>
          <Show when={(reqs() ?? []).length > 0} fallback={
            <div class="hint" style="padding:14px 6px;text-align:center;">
              {tab() === "incoming" ? "받은 사용 요청이 없습니다." : "보낸 요청이 없습니다."}
            </div>
          }>
            <div style="display:flex;flex-direction:column;gap:6px;max-height:340px;overflow-y:auto;">
              <For each={reqs()}>
                {(req) => {
                  const b = statusBadge(req.status);
                  return (
                    <div style="padding:9px 11px;border-radius:9px;border:1px solid #2a2f3a;background:#15171c;">
                      <div style="display:flex;align-items:center;gap:8px;">
                        <span style="font-size:13px;color:#cfe3d6;font-weight:600;">
                          {tab() === "incoming" ? req.requester : req.target_agent}
                        </span>
                        <span style="font-size:11px;color:#6b7280;">
                          {tab() === "incoming" ? `→ ${req.target_agent}` : `@ ${req.target_machine ?? ""}`}
                        </span>
                        <span style={`margin-left:auto;font-size:11px;padding:2px 8px;border-radius:10px;background:${b.bg};color:${b.fg};`}>
                          {b.text}
                        </span>
                      </div>
                      <Show when={req.status === "accepted" && req.price_amount != null}>
                        <div style="margin-top:4px;font-size:12px;color:#3fb950;">
                          💰 {req.price_amount} {req.currency ?? "USDC"} / {req.price_unit ?? "per_call"}
                          <Show when={req.terms}><span style="color:#8b94a3;"> · {req.terms}</span></Show>
                        </div>
                      </Show>
                      <div style="margin-top:3px;font-size:10px;color:#5b6270;">{req.created_at_kst}</div>

                      {/* 소유자 액션 — pending 일 때만 수락(가격)/거절. */}
                      <Show when={tab() === "incoming" && req.status === "pending"}>
                        <div style="display:flex;gap:6px;margin-top:7px;">
                          <button class="btn-go" style="flex:1;" disabled={busy()}
                            onClick={() => openAccept(req.id)}>수락 + 가격 책정</button>
                          <button class="btn-q" disabled={busy()}
                            onClick={() => void decide(req.id, "reject")}>거절</button>
                        </div>
                      </Show>
                      {/* 소유자 — 이미 수락한 건 철회 가능. */}
                      <Show when={tab() === "incoming" && req.status === "accepted"}>
                        <div style="display:flex;gap:6px;margin-top:7px;">
                          <button class="btn-q" disabled={busy()}
                            onClick={() => void decide(req.id, "revoke")}>접근 철회</button>
                        </div>
                      </Show>
                    </div>
                  );
                }}
              </For>
            </div>
          </Show>
        </Show>

        {/* 가격 책정 모달 (수락) */}
        <Show when={acceptId()}>
          <div style="margin-top:12px;padding:11px;border-radius:9px;border:1px solid #2f5d3a;background:#1a221c;">
            <div style="font-size:13px;color:#cfe3d6;font-weight:600;margin-bottom:8px;">💰 사용 가격 책정 (소유자)</div>
            <div style="display:flex;flex-direction:column;gap:8px;">
              <div style="display:flex;align-items:center;gap:8px;">
                <span style="font-size:12px;color:#8b94a3;width:48px;flex:none;">가격</span>
                <input class="ctl" style="flex:1;" type="number" min="0" step="0.0001"
                  value={priceAmount()} onInput={(e) => setPriceAmount(e.currentTarget.value)}
                  placeholder="0.05" />
                <input class="ctl" style="width:72px;" value={currency()}
                  onInput={(e) => setCurrency(e.currentTarget.value)} placeholder="USDC" />
              </div>
              <div style="display:flex;align-items:center;gap:8px;">
                <span style="font-size:12px;color:#8b94a3;width:48px;flex:none;">단위</span>
                <select class="ctl" style="flex:1;" value={priceUnit()}
                  onChange={(e) => setPriceUnit(e.currentTarget.value)}>
                  <For each={PRICE_UNITS}>{(u) => <option value={u.v}>{u.label}</option>}</For>
                </select>
              </div>
              <input class="ctl" value={terms()} onInput={(e) => setTerms(e.currentTarget.value)}
                placeholder="이용 조건 (선택)" />
              <div class="hint" style="font-size:11px;color:#b5a98f;">
                수락하면 요청자는 격리(fresh worktree)에서만 이 에이전트를 구동하고, 사용량은 이 가격으로 과금 원장에 기록됩니다.
                실제 USDC 정산은 결제 인프라 책임입니다.
              </div>
              <div style="display:flex;gap:6px;">
                <button class="btn-go" style="flex:1;" disabled={busy()} onClick={() => void confirmAccept()}>
                  {busy() ? "수락 중…" : "수락 (가격 확정)"}
                </button>
                <button class="btn-q" disabled={busy()} onClick={() => setAcceptId(null)}>취소</button>
              </div>
            </div>
          </div>
        </Show>

        <div class="modal-foot">
          <button class="btn-q" onClick={() => props.onClose()} disabled={busy()}>닫기</button>
        </div>
      </div>
    </div>
  );
}
