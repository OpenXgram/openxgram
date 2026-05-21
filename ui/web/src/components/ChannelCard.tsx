import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@/api/client";
import { NotifySetup } from "./NotifySetup";
import { Breadcrumb } from "./Breadcrumb";

// UI-CHANNEL-SPEC v1.0 §3 — 📱 채널 카드 (PRD §0 #4: 인간 친화 채널).
// 5 탭: 인박스 / 사람 / 채널 등록 / 라우팅 / 모더레이션.

type Tab = "inbox" | "person" | "register" | "routing" | "moderation";

interface MessageDto {
  id: string;
  sender: string;
  body: string;
  timestamp: string;
}

async function fetchInboxMessages(): Promise<MessageDto[]> {
  try {
    const all = await invoke<MessageDto[]>("messages_recent", { limit: 200 });
    // 인간 채널 sender 만 필터 — discord:* / telegram:* prefix
    return all.filter((m) => /^(discord|telegram|slack):/i.test(m.sender));
  } catch {
    return [];
  }
}

export function ChannelCard(props: { onBack: () => void }) {
  const [tab, setTab] = createSignal<Tab>("inbox");
  const [inbox] = createResource(fetchInboxMessages);

  return (
    <div class="card-page">
      <Breadcrumb cardName="📱 채널" onReturn={props.onBack} />
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">📱</span>
        <h1>채널</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #4 — 인간 친화 채널</div>
      <div class="card-page-oneline">
        Discord·Telegram·Slack·카카오·WhatsApp·Web — 사람 중심 인박스 + 봇 라이프사이클 + 사람별 정책
      </div>

      <nav style="display:flex; gap:4px; margin-bottom:14px;">
        <button class={"link-btn " + (tab() === "inbox" ? "active" : "")} onClick={() => setTab("inbox")}>📥 인박스</button>
        <button class={"link-btn " + (tab() === "person" ? "active" : "")} onClick={() => setTab("person")}>👤 사람</button>
        <button class={"link-btn " + (tab() === "register" ? "active" : "")} onClick={() => setTab("register")}>➕ 채널 등록</button>
        <button class={"link-btn " + (tab() === "routing" ? "active" : "")} onClick={() => setTab("routing")}>🔀 라우팅</button>
        <button class={"link-btn " + (tab() === "moderation" ? "active" : "")} onClick={() => setTab("moderation")}>🛡️ 모더레이션</button>
      </nav>

      <Show when={tab() === "inbox"}>
        <section class="card-section">
          <h3>📥 인박스 — 사양 §3.1 (M-5)</h3>
          <p class="placeholder-note">
            모든 채널·모든 사람의 메시지 통합 타임라인. 사람 클릭 시 그 사람과의 대화. 메시지 클릭 시 💬 메신저로 점프.
          </p>
          <Show when={(inbox() ?? []).length === 0}>
            <div class="card-section-row"><span class="value">인박스 비어있음 (채널 봇 미설정 또는 메시지 없음)</span></div>
          </Show>
          <For each={inbox() ?? []}>
            {(m) => (
              <div class="card-section-row" style="border-bottom:1px solid var(--border); padding:8px 0;">
                <span class="label">{m.sender}</span>
                <span class="value">{m.body.slice(0, 80)}{m.body.length > 80 ? "…" : ""}</span>
              </div>
            )}
          </For>
        </section>
      </Show>

      <Show when={tab() === "person"}>
        <PersonSection />
      </Show>

      <Show when={tab() === "register"}>
        <section class="card-section">
          <h3>➕ 채널 등록 — 사양 §3.3 (M-1 M-2 M-3)</h3>
          <p class="placeholder-note">
            봇 토큰은 자동으로 🗝️ Vault 에 저장 (안티패턴 #3 준수). 본 카드는 등록·연결 테스트만.
          </p>
          <NotifySetup />
        </section>
        <DiscordDiagnosticSection />
      </Show>

      <Show when={tab() === "routing"}>
        <RoutingSection />
      </Show>

      <Show when={tab() === "moderation"}>
        <ModerationSection />
      </Show>
    </div>
  );
}

function PersonSection() {
  const [people] = createResource<any[]>(async () => { try { return await invoke<any[]>("channel_people"); } catch { return []; } });
  return (
    <section class="card-section">
      <h3>👤 사람 — 사양 §3.2 (M-6)</h3>
      <p style="font-size:12px; color:var(--text-3);">PersonId 통합 (Discord·Telegram·Slack 같은 사람).</p>
      <Show when={(people() ?? []).length === 0}>
        <div style="font-size:12px; padding:4px 0;">아직 사람 메시지 없음 (봇이 메시지 받으면 표시).</div>
      </Show>
      <For each={people() ?? []}>{(p) => (
        <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
          <strong>{p.person_id}</strong>
          <span style="color:var(--text-3); margin-left:8px;">{p.msg_count} 메시지 · 마지막 {p.last_at}</span>
        </div>
      )}</For>
    </section>
  );
}

function ModerationSection() {
  const [blocks, { refetch: refetchB }] = createResource<any[]>(async () => { try { return await invoke<any[]>("channel_blocks_list"); } catch { return []; } });
  const [limits, { refetch: refetchL }] = createResource<any[]>(async () => { try { return await invoke<any[]>("channel_limits_list"); } catch { return []; } });
  const [blockPid, setBlockPid] = createSignal("");
  const [reason, setReason] = createSignal("");
  const [limPid, setLimPid] = createSignal("");
  const [daily, setDaily] = createSignal(100);
  async function block() {
    if (!blockPid()) return;
    try { await invoke("channel_block_add", { person_id: blockPid(), reason: reason() }); setBlockPid(""); setReason(""); await refetchB(); } catch (e) { alert(String(e)); }
  }
  async function setLimit() {
    if (!limPid()) return;
    try { await invoke("channel_limit_set", { person_id: limPid(), daily_limit: daily() }); setLimPid(""); await refetchL(); } catch (e) { alert(String(e)); }
  }
  return (
    <>
      <section class="card-section">
        <h3>🛡️ 차단 — 사양 §3.5 (M-10)</h3>
        <div style="display:flex; gap:4px; margin-bottom:6px;">
          <input value={blockPid()} onInput={(e) => setBlockPid(e.currentTarget.value)} placeholder="person_id (discord:user123)" style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
          <input value={reason()} onInput={(e) => setReason(e.currentTarget.value)} placeholder="사유" style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
          <button class="link-btn" onClick={block}>🚫 차단</button>
        </div>
        <For each={blocks() ?? []}>{(b) => (
          <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);"><code>{b.person_id}</code> · {b.reason} · {b.blocked_at}</div>
        )}</For>
      </section>
      <section class="card-section">
        <h3>📊 사람별 일 한도 — 사양 §3.5</h3>
        <div style="display:flex; gap:4px; margin-bottom:6px;">
          <input value={limPid()} onInput={(e) => setLimPid(e.currentTarget.value)} placeholder="person_id" style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
          <input type="number" value={daily()} onInput={(e) => setDaily(parseInt(e.currentTarget.value) || 100)} placeholder="일 한도" style="width:100px; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
          <button class="link-btn" onClick={setLimit}>설정</button>
        </div>
        <For each={limits() ?? []}>{(l) => (
          <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);"><code>{l.person_id}</code> · {l.today_used}/{l.daily_limit}</div>
        )}</For>
      </section>
    </>
  );
}

function DiscordDiagnosticSection() {
  const [d, { refetch }] = createResource<any>(async () => { try { return await invoke<any>("notify_discord_diagnostic"); } catch (e) { return { error: String(e) }; } });
  return (
    <section class="card-section">
      <h3>🔍 Discord 봇 진단</h3>
      <button class="link-btn" onClick={() => refetch()}>↻ 진단 다시 실행</button>
      <Show when={d()}>
        <div class="card-section-row"><span class="label">상태</span><span class="value">{d()?.summary || d()?.error}</span></div>
        <div class="card-section-row"><span class="label">token_status</span><span class="value">{d()?.token_status}</span></div>
        <div class="card-section-row"><span class="label">봇 이름</span><span class="value">{d()?.bot_username}</span></div>
        <div class="card-section-row"><span class="label">guild 가입 수</span><span class="value">{d()?.guild_count}</span></div>
        <div class="card-section-row"><span class="label">install_permissions</span><span class="value mono">{d()?.install_permissions}</span></div>
        <div class="card-section-row"><span class="label">scopes</span><span class="value mono">{JSON.stringify(d()?.install_scopes)}</span></div>
        <div class="card-section-row"><span class="label">channel_id</span><span class="value mono">{d()?.channel_id_configured}</span></div>
        <div class="card-section-row"><span class="label">channel 접근</span><span class="value">{d()?.channel_access_ok ? "✅ 200" : `❌ ${d()?.channel_access_status}`}</span></div>
        <Show when={d()?.needs_reinvite && d()?.reinvite_url}>
          <div style="margin-top:8px; padding:10px; background:#fee; border:1px solid #f88; border-radius:4px;">
            <p style="margin:0 0 6px; color:#c00; font-weight:bold;">⚠ 봇 권한 부족 — 아래 URL 로 재초대:</p>
            <a href={d().reinvite_url} target="_blank" style="word-break:break-all; color:#06c; font-family:monospace; font-size:11px;">{d().reinvite_url}</a>
            <p style="margin:6px 0 0; font-size:11px; color:#666;">권한 = View Channels + Send Messages + Read Message History (68608)</p>
          </div>
        </Show>
      </Show>
    </section>
  );
}

function RoutingSection() {
  const [r] = createResource(async () => { try { return await invoke<any>("channel_routing"); } catch { return null; } });
  return (
    <section class="card-section">
      <h3>🔀 라우팅 (인간 ↔ 에이전트) — 사양 §3.4 (M-7 V-4)</h3>
      <Show when={r()}>
        <div class="card-section-row"><span class="label">scope</span><span class="value">{r()?.scope}</span></div>
        <div class="card-section-row"><span class="label">기본 멘션 트리거</span><span class="value mono">{r()?.default_mention_trigger}</span></div>
        <div class="card-section-row"><span class="label">기본 권한</span><span class="value">{r()?.default_permission}</span></div>
      </Show>
      <p class="placeholder-note">에이전트↔에이전트 라우팅은 💬 메신저 V11 (다른 마스터). 사용자 규칙 추가 UI = Phase 2.</p>
    </section>
  );
}
