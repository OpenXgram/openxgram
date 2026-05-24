import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";
import { NotifySetup} from "./NotifySetup";
import { Breadcrumb} from "./Breadcrumb";

// UI-CHANNEL-SPEC v1.0 §3 — 채널 카드 (PRD §0 #4: 인간 친화 채널).
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
 const all = await invoke<MessageDto[]>("messages_recent", { limit: 200});
 // 인간 채널 sender 만 필터 — discord:* / telegram:* prefix
 return all.filter((m) => /^(discord|telegram|slack):/i.test(m.sender));
} catch {
 return [];
}
}

export function ChannelCard(props: { onBack: () => void}) {
 const [tab, setTab] = createSignal<Tab>("inbox");
 const [inbox] = createResource(fetchInboxMessages);

 return (
 <div class="card-page">
 <Breadcrumb cardName=" 채널" onReturn={props.onBack} />
 <div class="card-page-head">
 <span class="icon"></span>
 <h1>채널</h1>
 </div>
 <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #4 — 인간 친화 채널</div>
 <div class="card-page-oneline">
 Discord·Telegram·Slack·카카오·WhatsApp·Web — 사람 중심 인박스 + 봇 라이프사이클 + 사람별 정책
 </div>

 <nav style="display:flex; gap:4px; margin-bottom:14px; position:relative; z-index:5;">
 <button type="button" class={"link-btn " + (tab() === "inbox" ? "active" : "")} onClick={() => setTab("inbox")} style="position:relative; z-index:5;"> 인박스</button>
 <button type="button" class={"link-btn " + (tab() === "person" ? "active" : "")} onClick={() => setTab("person")} style="position:relative; z-index:5;"> 사람</button>
 <button type="button" class={"link-btn " + (tab() === "register" ? "active" : "")} onClick={() => setTab("register")} style="position:relative; z-index:5;"> 채널 등록</button>
 <button type="button" class={"link-btn " + (tab() === "routing" ? "active" : "")} onClick={() => setTab("routing")} style="position:relative; z-index:5;"> 라우팅</button>
 <button type="button" class={"link-btn " + (tab() === "moderation" ? "active" : "")} onClick={() => setTab("moderation")} style="position:relative; z-index:5;"> 모더레이션</button>
 </nav>

 <Show when={tab() === "inbox"}>
 <section class="card-section">
 <h3> 인박스 — 사양 §3.1 (M-5)</h3>
 <p class="placeholder-note">
 모든 채널·모든 사람의 메시지 통합 타임라인. 사람 클릭 시 그 사람과의 대화. 메시지 클릭 시 메신저로 점프.
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
 <ChannelsSummarySection />
 <section class="card-section">
 <h3> 채널 등록 — 사양 §3.3 (M-1 M-2 M-3)</h3>
 <p class="placeholder-note">
 봇 토큰은 자동으로 Vault 에 저장 (안티패턴 #3 준수). 본 카드는 등록·연결 테스트만.
 </p>
 <NotifySetup />
 </section>
 <MultiBotsSection />
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
 const [people] = createResource<any[]>(async () => { try { return await invoke<any[]>("channel_people");} catch { return [];}});
 return (
 <section class="card-section">
 <h3> 사람 — 사양 §3.2 (M-6)</h3>
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
 const [blocks, { refetch: refetchB}] = createResource<any[]>(async () => { try { return await invoke<any[]>("channel_blocks_list");} catch { return [];}});
 const [limits, { refetch: refetchL}] = createResource<any[]>(async () => { try { return await invoke<any[]>("channel_limits_list");} catch { return [];}});
 const [blockPid, setBlockPid] = createSignal("");
 const [reason, setReason] = createSignal("");
 const [limPid, setLimPid] = createSignal("");
 const [daily, setDaily] = createSignal(100);
 async function block() {
 if (!blockPid()) return;
 try { await invoke("channel_block_add", { person_id: blockPid(), reason: reason()}); setBlockPid(""); setReason(""); await refetchB();} catch (e) { alert(String(e));}
}
 async function setLimit() {
 if (!limPid()) return;
 try { await invoke("channel_limit_set", { person_id: limPid(), daily_limit: daily()}); setLimPid(""); await refetchL();} catch (e) { alert(String(e));}
}
 return (
 <>
 <section class="card-section">
 <h3> 차단 — 사양 §3.5 (M-10)</h3>
 <div style="display:flex; gap:4px; margin-bottom:6px;">
 <input value={blockPid()} onInput={(e) => setBlockPid(e.currentTarget.value)} placeholder="person_id (discord:user123)" style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <input value={reason()} onInput={(e) => setReason(e.currentTarget.value)} placeholder="사유" style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <button class="link-btn" onClick={block}> 차단</button>
 </div>
 <For each={blocks() ?? []}>{(b) => (
 <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);"><code>{b.person_id}</code> · {b.reason} · {b.blocked_at}</div>
)}</For>
 </section>
 <section class="card-section">
 <h3> 사람별 일 한도 — 사양 §3.5</h3>
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

// rc.92 — 채널 카드 상단 종합 정보 (모든 봇 + 가입 서버 + binding 통계).
function ChannelsSummarySection() {
 const [s, { refetch}] = createResource<any>(async () => {
 try { return await invoke<any>("channels_summary");} catch (e) { return { error: String(e)};}
});
 return (
 <section class="card-section">
 <h3> 연결된 봇 · 채널 종합 정보</h3>
 <button type="button" class="link-btn" onClick={() => refetch()} style="margin-bottom:8px;">↻ 새로고침</button>
 <Show when={s()?.error}>
 <div style="color:#f85149; font-size:12px;">에러: {s()?.error}</div>
 </Show>
 <Show when={s() && !s()?.error}>
 {/* Discord 봇 카드들 */}
 <strong style="font-size:13px; display:block; margin:8px 0 6px;">📨 Discord 봇 ({s()?.discord?.bots_count ?? 0})</strong>
 <Show when={(s()?.discord?.bots ?? []).length === 0}>
 <div style="font-size:12px; color:var(--text-3); padding:4px 0;">등록된 디스코드 봇 없음. 아래 wizard 에서 추가.</div>
 </Show>
 <For each={s()?.discord?.bots ?? []}>
 {(b: any) => (
 <div style="padding:10px; margin-bottom:6px; background:var(--surface-2); border:1px solid var(--border); border-radius:6px;">
 <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:4px;">
 <div>
 <strong style="font-size:13px;">{b.bot_username || b.alias || "?"}</strong>
 <span style="font-size:11px; color:var(--text-3); margin-left:6px;">{b.source}</span>
 </div>
 <span style="font-size:11px; color:var(--text-3);">id=<code>{b.bot_id}</code></span>
 </div>
 <Show when={b.error}>
 <div style="font-size:11px; color:#f85149;">⚠️ {b.error}</div>
 </Show>
 <div style="font-size:11px; color:var(--text-3);">token=<code>{b.token_prefix}...</code> · 가입 서버 <strong>{b.guilds_count}</strong>개</div>
 <Show when={(b.guilds ?? []).length > 0}>
 <div style="margin-top:4px;">
 <For each={b.guilds}>
 {(g: any) => (
 <div style="font-size:11px; padding:3px 8px; margin:2px 0; background:var(--surface); border-radius:3px;">
 <strong>{g.name}</strong> <code style="color:var(--text-3);">{g.id}</code>
 {g.owner ? <span style="margin-left:6px; color:#d29922;">(owner)</span> : null}
 </div>
 )}
 </For>
 </div>
 </Show>
 </div>
 )}
 </For>
 {/* Telegram 봇 */}
 <strong style="font-size:13px; display:block; margin:10px 0 6px;">✈️ Telegram 봇</strong>
 <Show when={!s()?.telegram}>
 <div style="font-size:12px; color:var(--text-3); padding:4px 0;">Telegram 봇 미등록.</div>
 </Show>
 <Show when={s()?.telegram}>
 <div style="padding:10px; background:var(--surface-2); border:1px solid var(--border); border-radius:6px;">
 <strong style="font-size:13px;">@{s()?.telegram?.bot_username}</strong>
 <span style="font-size:11px; color:var(--text-3); margin-left:6px;">id={s()?.telegram?.bot_id}</span>
 <Show when={s()?.telegram?.error}>
 <div style="font-size:11px; color:#f85149;">⚠️ {s()?.telegram?.error}</div>
 </Show>
 <div style="font-size:11px; color:var(--text-3);">token=<code>{s()?.telegram?.token_prefix}...</code></div>
 </div>
 </Show>
 {/* 바인딩 통계 */}
 <strong style="font-size:13px; display:block; margin:10px 0 6px;">🔗 채널 바인딩 ({Object.values(s()?.bindings?.stats_per_platform ?? {}).reduce((a: number, b: any) => a + (b as number), 0)})</strong>
 <Show when={(s()?.bindings?.stats_per_channel ?? []).length === 0}>
 <div style="font-size:12px; color:var(--text-3); padding:4px 0;">등록된 바인딩 없음. AgentSidePanel 의 채널 바인딩 탭에서 추가.</div>
 </Show>
 <For each={s()?.bindings?.stats_per_channel ?? []}>
 {(bc: any) => (
 <div style="font-size:11px; padding:4px 8px; margin:2px 0; background:var(--surface-2); border-radius:3px;">
 <strong>{bc.platform}</strong> · <code>{bc.channel_ref}</code> · {bc.count} 바인딩
 </div>
 )}
 </For>
 </Show>
 </section>
 );
}

// rc.92 — 멀티 디스코드 봇 관리 (채널·세션별 다른 봇 연결).
function MultiBotsSection() {
 const [bots, { refetch}] = createResource<any[]>(async () => {
 try { return await invoke<any[]>("discord_bots_list");} catch { return [];}
});
 const [alias, setAlias] = createSignal("");
 const [token, setToken] = createSignal("");
 const [busy, setBusy] = createSignal(false);
 const [msg, setMsg] = createSignal<string | null>(null);
 async function add() {
 if (!alias().trim() || !token().trim()) { alert("alias + bot token 필요"); return;}
 setBusy(true); setMsg(null);
 try {
 const r = await invoke<any>("discord_bots_add", { alias: alias().trim(), bot_token: token().trim()});
 setMsg("✓ 추가됨: " + (r.bot_username || r.alias) + " (id=" + r.id + "). daemon 재시작 시 listener 자동 spawn.");
 setAlias(""); setToken("");
 await refetch();
} catch (e) { setMsg("✗ " + e);} finally { setBusy(false);}
}
 async function del(id: string) {
 if (!confirm("이 봇 삭제? (bindings 의 bot_id 가 NULL 되어 default 봇 사용)")) return;
 setBusy(true);
 try { await invoke("discord_bots_delete", { id}); await refetch();} finally { setBusy(false);}
}
 return (
 <section class="card-section">
 <h3> 멀티 디스코드 봇 — 채널·세션별 다른 봇 연결</h3>
 <p style="font-size:12px; color:var(--text-3); margin-bottom:8px;">
 default 봇 (위 notify.toml) 외에 추가로 여러 봇 등록. 바인딩 시 봇 선택. 다른 메이커 봇 공존 가능.
 </p>
 <div style="display:flex; flex-direction:column; gap:6px; margin-bottom:8px;">
 <div style="display:flex; flex-direction:column; gap:3px;">
 <label style="font-size:11px; color:var(--text-3);">봇 alias (표시명)</label>
 <input value={alias()} onInput={(e) => setAlias(e.currentTarget.value)} placeholder="예: 내 봇 / 친구 봇 / 마케팅 봇"
 style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 </div>
 <div style="display:flex; flex-direction:column; gap:3px;">
 <label style="font-size:11px; color:var(--text-3);">Discord Bot Token (Developer Portal 에서 Reset Token 으로 발급)</label>
 <input value={token()} onInput={(e) => setToken(e.currentTarget.value)} placeholder="MTQ4NDYx..." type="password"
 style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 </div>
 <button type="button" class="link-btn" onClick={add} disabled={busy()}
 style="background:#238636; color:white; padding:8px 14px; border:none; border-radius:4px; align-self:flex-start;">
 ▶ 봇 추가 (token 자동 검증)
 </button>
 </div>
 <Show when={msg()}>
 <div style={`padding:6px 10px; font-size:11px; border-radius:4px; background:${msg()!.startsWith("✓") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>{msg()}</div>
 </Show>
 <strong style="font-size:12px; display:block; margin-top:10px;">등록된 봇 ({bots()?.length ?? 0})</strong>
 <For each={bots() ?? []}>
 {(b) => (
 <div style="display:flex; justify-content:space-between; align-items:center; padding:6px 0; border-bottom:1px solid var(--border); font-size:12px;">
 <div style="flex:1; min-width:0;">
 <strong>{b.alias}</strong>
 <span style="color:var(--text-3); margin-left:6px;">id=<code>{b.id}</code> · bot_user_id=<code>{b.bot_user_id || "?"}</code> · {b.token_prefix}</span>
 </div>
 <button type="button" class="link-btn" onClick={() => del(b.id)} disabled={busy()} style="color:#f85149;">삭제</button>
 </div>
)}
 </For>
 <p class="hint" style="font-size:11px; color:var(--text-3); margin-top:8px;">
 💡 새 봇 추가 후 daemon 재시작 필요 (listener spawn). 봇 등록 시 token 자동 검증 (Discord users/@me).
 </p>
 </section>
);
}

function DiscordDiagnosticSection() {
 const [d, { refetch}] = createResource<any>(async () => { try { return await invoke<any>("notify_discord_diagnostic");} catch (e) { return { error: String(e)};}});
 return (
 <section class="card-section">
 <h3> Discord 봇 진단</h3>
 <button class="link-btn" onClick={() => refetch()}>↻ 진단 다시 실행</button>
 <Show when={d()}>
 <div class="card-section-row"><span class="label">상태</span><span class="value">{d()?.summary || d()?.error}</span></div>
 <div class="card-section-row"><span class="label">token_status</span><span class="value">{d()?.token_status}</span></div>
 <div class="card-section-row"><span class="label">봇 이름</span><span class="value">{d()?.bot_username}</span></div>
 <div class="card-section-row"><span class="label">guild 가입 수</span><span class="value">{d()?.guild_count}</span></div>
 <div class="card-section-row"><span class="label">install_permissions</span><span class="value mono">{d()?.install_permissions}</span></div>
 <div class="card-section-row"><span class="label">scopes</span><span class="value mono">{JSON.stringify(d()?.install_scopes)}</span></div>
 <div class="card-section-row"><span class="label">channel_id</span><span class="value mono">{d()?.channel_id_configured}</span></div>
 <div class="card-section-row"><span class="label">channel 접근</span><span class="value">{d()?.channel_access_ok ? " 200" : ` ${d()?.channel_access_status}`}</span></div>
 <Show when={d()?.needs_reinvite && d()?.reinvite_url}>
 <div style="margin-top:8px; padding:10px; background:#fee; border:1px solid #f88; border-radius:4px;">
 <p style="margin:0 0 6px; color:#c00; font-weight:bold;"> 봇 권한 부족 — 아래 URL 로 재초대:</p>
 <a href={d().reinvite_url} target="_blank" style="word-break:break-all; color:#06c; font-family:monospace; font-size:11px;">{d().reinvite_url}</a>
 <p style="margin:6px 0 0; font-size:11px; color:#666;">권한 = View Channels + Send Messages + Read Message History (68608)</p>
 </div>
 </Show>
 </Show>
 </section>
);
}

function RoutingSection() {
 const [r] = createResource(async () => { try { return await invoke<any>("channel_routing");} catch { return null;}});
 return (
 <section class="card-section">
 <h3> 라우팅 (인간 ↔ 에이전트) — 사양 §3.4 (M-7 V-4)</h3>
 <Show when={r()}>
 <div class="card-section-row"><span class="label">scope</span><span class="value">{r()?.scope}</span></div>
 <div class="card-section-row"><span class="label">기본 멘션 트리거</span><span class="value mono">{r()?.default_mention_trigger}</span></div>
 <div class="card-section-row"><span class="label">기본 권한</span><span class="value">{r()?.default_permission}</span></div>
 </Show>
 <p class="placeholder-note">에이전트↔에이전트 라우팅은 메신저 V11 (다른 마스터). 사용자 규칙 추가 UI = Phase 2.</p>
 </section>
);
}
