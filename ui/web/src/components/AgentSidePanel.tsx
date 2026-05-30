import { createEffect, createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";

// UI-MESSENGER-SPEC v1.3 §5 — 우측 12 탭 (S3 세로 사이드).
// Tier 3 MVP = 5 탭: 개요 · 역할 · 채널 바인딩 · 상태·리소스 · 지갑·결제.
// 색은 styles.css 의 --surface-* / --text-* 변수 사용 (다크/라이트 자동).

interface PeerMeta {
 alias: string;
 address: string;
 public_key_hex: string;
 machine?: string;
 last_seen?: string;
 // rc.92 D2 — capabilities
 description?: string | null;
 capabilities?: string[];
}

interface NotifyStatus {
 discord_configured: boolean;
 telegram_configured: boolean;
}

type TabId =
 | "overview"
 | "messenger"
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
const TABS: { id: TabId; label: string; icon: string}[] = [
 { id: "overview", label: "개요", icon: ""},
 { id: "messenger", label: "역할·메신저 등록", icon: ""},
 { id: "channel", label: "채널 바인딩", icon: ""},
 { id: "status", label: "상태·리소스", icon: ""},
 { id: "history", label: "히스토리", icon: ""},
 { id: "export", label: "가져오기·내보내기", icon: ""},
 { id: "wallet", label: "지갑·결제", icon: ""},
 { id: "tokens", label: "토큰", icon: ""},
 { id: "cron", label: "Cron", icon: ""},
 { id: "files", label: "파일·지침", icon: ""},
 { id: "notify", label: "알림", icon: ""},
 { id: "permissions", label: "권한·도구·MCP", icon: ""},
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
 style="display:flex; align-items:center; gap:6px; padding:6px 10px; text-align:left; width:100%;"
 >
 <span style="font-size:14px;">{tt.icon}</span>
 <span style="font-size:12px;">{tt.label}</span>
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
 <Show when={tab() === "messenger"}>
 <MessengerRegisterTab peer={props.peer} onJumpToSettings={props.onJumpToSettings} />
 </Show>
 <Show when={tab() === "channel"}>
 <ChannelTab notify={notify()} onJumpToSettings={props.onJumpToSettings} agentId={props.peer.alias} />
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

// ── 세션 import: 이 터미널 session 으로 직접 적재 ─────────────────
function SessionImportSection(props: { sessionId: string}) {
 const [text, setText] = createSignal("");
 const [busy, setBusy] = createSignal(false);
 const [result, setResult] = createSignal<string | null>(null);
 const [promptVisible, setPromptVisible] = createSignal(false);
 const [channel, setChannel] = createSignal<"paste" | "webhook" | "api" | "mcp">("paste");
 const [webhookData, setWebhookData] = createSignal<any>(null);

 async function ensureWebhook() {
 try {
 let d = await invoke<any>("memory_webhook_token");
 if (!d?.exists) d = await invoke<any>("memory_webhook_rotate");
 setWebhookData(d);
 } catch (e) { alert("webhook 발급 실패: " + e);}
 }

 // 채널-특화 단일 프롬프트 (Bearer 토큰 자동, webhook URL 자동 발급)
 const [bearerToken, setBearerToken] = createSignal<string>("");
 const buildPrompt = async (): Promise<string> => {
 const base = window.location.origin;
 const sid = props.sessionId;
 const ch = channel();
 // bearer 토큰: localStorage 의 session_token 또는 unlock 다시 해서 받기
 let bearer = bearerToken();
 if (!bearer && ch === "api") {
 bearer = localStorage.getItem("xgram_session_token") || "";
 if (!bearer) bearer = "<로그인 후 GUI 의 localStorage xgram_session_token 복사>";
 setBearerToken(bearer);
 }
 // webhook URL: 자동 발급
 let webhookUrl = "";
 if (ch === "webhook") {
 if (!webhookData()) await ensureWebhook();
 webhookUrl = webhookData()?.webhook_url || "";
 }
 const head = `# OpenXgram 메모리 가져오기 — 이 세션 (${sid}) 으로 import

당신은 외부 LLM 입니다. 현재 우리가 진행 중인 이 대화의 핵심을 OpenXgram 메모리로 보내야 합니다.

## 1. 무엇을 추출

5종 항목 중 해당하는 것들을 정리:
- **message**: 모든 의미 있는 발화·응답
- **episode**: 한 작업 흐름 요약
- **wiki_fact**: 영구 사실/정의 (위키 페이지로 격상)
- **pattern**: 사용자 반복 행동·선호·규칙
- **mistake**: 실수 + 교훈

## 2. 출력 형식 (JSON 단일 객체, session_id 는 그대로 유지)

\`\`\`json
{
  "openxgram_import_version": 1,
  "source_app": "Claude Desktop / ChatGPT / Cursor / Gemini / 기타",
  "session_id": "${sid}",
  "session_title": "한 줄 요약",
  "items": [
    {"type":"message","sender":"user","body":"...","timestamp":"2026-05-22T10:00:00Z"},
    {"type":"wiki_fact","page_id":"some-id","title":"제목","page_type":"concept","content":"# 제목\\n본문"},
    {"type":"pattern","pattern_type":"preference","description":"사용자는 ...","confidence":0.9},
    {"type":"mistake","intended_action":"...","actual_outcome":"...","failure_reason":"...","lesson":"...","severity":5}
  ]
}
\`\`\`

## 3. 보내는 방법 (선택: ${ch})

`;
 let body = "";
 if (ch === "paste") {
 body = `위 JSON 을 그대로 W님께 답장으로 출력하세요. W님이 OpenXgram → 메신저 → 사이드패널 → 내보내기 탭 → textarea 에 붙여넣고 "이 세션에 import" 클릭.`;
 } else if (ch === "webhook") {
 body = `다음 webhook URL 로 직접 POST 하세요 (Bearer 인증 없음, URL 자체가 토큰):

\`\`\`bash
curl -X POST "${webhookUrl}" \\
  -H "Content-Type: application/json" \\
  -d '<<위 JSON 그대로>>'
\`\`\`

당신이 도구 (예: \`fetch\`/\`shell\`) 를 가지고 있으면 직접 실행하세요. 없으면 사용자가 위 명령을 실행해 줄 수 있도록 출력만 하세요.`;
 } else if (ch === "api") {
 body = `다음 curl 로 POST 하세요 (Bearer 토큰 포함):

\`\`\`bash
curl -X POST "${base}/v1/gui/memory/import/bundle" \\
  -H "Authorization: Bearer ${bearer}" \\
  -H "Content-Type: application/json" \\
  -d '<<위 JSON 그대로>>'
\`\`\`

⚠️ Bearer 토큰은 W님 개인 세션 토큰 — 외부에 노출 금지.`;
 } else if (ch === "mcp") {
 body = `Claude Desktop / Cursor 에 OpenXgram MCP 가 등록되어 있으면 직접 도구 호출:

\`\`\`
memory_import_bundle({
  openxgram_import_version: 1,
  session_id: "${sid}",
  session_title: "...",
  items: [...]
})
\`\`\`

MCP 등록 안 됐으면 \`xgram init\` 자동 통합 후 Claude Desktop 재시작.`;
 }
 return head + body + `

---

이제 지금까지의 이 대화 전체를 분석해서 위 JSON 형식으로 정리한 뒤, **위 "보내는 방법" 그대로 실행 또는 출력**해 주세요.`;
 };

 async function copyPrompt() {
 try {
 const p = await buildPrompt();
 await navigator.clipboard.writeText(p);
 alert("프롬프트 복사됨 — 외부 LLM 채팅창에 붙여넣기");
 } catch (e) { alert("실패: " + e);}
 }
 async function showPrompt() {
 setPromptVisible(true);
 const p = await buildPrompt();
 (document.getElementById(`prompt-preview-${props.sessionId.replace(/[^a-z0-9]/gi,'')}`) as HTMLElement | null)
 ?.replaceChildren(document.createTextNode(p));
 }

 async function doImport() {
 const raw = text().trim();
 if (!raw) return;
 setBusy(true);
 try {
 // 자동 감지: JSON bundle 이면 그대로, 아니면 한 줄 = 한 메시지 (jsonl) 또는 markdown
 let bundle: any;
 if (raw.startsWith("{")) {
 bundle = JSON.parse(raw);
 // session_id override → 이 터미널로 적재
 bundle.session_id = props.sessionId;
 if (!bundle.items) bundle.items = [];
 } else if (raw.includes('"sender"')) {
 // jsonl
 const items = raw.split('\n').filter(l => l.trim()).map(l => {
 const m = JSON.parse(l);
 return { type: "message", sender: m.sender || "imported", body: m.body || m.content || l,
 timestamp: m.timestamp || new Date().toISOString()};
 });
 bundle = { session_id: props.sessionId, source_app: "manual-paste", items};
 } else {
 // plain text → 한 메시지로
 bundle = {
 session_id: props.sessionId,
 source_app: "manual-paste",
 items: [{ type: "message", sender: "imported", body: raw, timestamp: new Date().toISOString()}]
 };
 }
 const r = await invoke<any>("memory_import_bundle", bundle);
 setResult(`✓ ${r.items_processed ?? 0} 항목 적재, messages ${r.inserted?.messages ?? 0}, wiki ${r.inserted?.wiki_pages ?? 0}, patterns ${r.inserted?.patterns ?? 0}, episodes ${r.inserted?.episodes ?? 0}, mistakes ${r.inserted?.mistakes ?? 0}`);
 setText("");
 } catch (e) { setResult("실패: " + String(e));}
 finally { setBusy(false);}
 }
 return (
 <section style="margin-top:14px; padding-top:10px; border-top:1px solid var(--border);">
 <strong style="font-size:13px;">가져오기 (Import → 이 세션)</strong>
 <p style="font-size:11px; color:var(--text-3); margin:4px 0;">
 <strong>이 세션</strong> ({props.sessionId.slice(0,30)}) 으로 외부 LLM 의 대화·메모리를 적재.
 </p>

 {/* 1단계 — 채널 선택 */}
 <div style="margin-top:8px;">
 <div style="font-size:11px; color:var(--text-3); margin-bottom:4px;">1) 외부 LLM 이 어떻게 보낼지 선택:</div>
 <div style="display:flex; gap:4px; flex-wrap:wrap;">
 <button class={"link-btn " + (channel() === "paste" ? "active" : "")}
 onClick={() => setChannel("paste")} style="font-size:11px;">A) 결과 붙여넣기</button>
 <button class={"link-btn " + (channel() === "webhook" ? "active" : "")}
 onClick={async () => { setChannel("webhook"); if (!webhookData()) await ensureWebhook();}}
 style="font-size:11px;">B) Webhook</button>
 <button class={"link-btn " + (channel() === "api" ? "active" : "")}
 onClick={() => setChannel("api")} style="font-size:11px;">C) API curl</button>
 <button class={"link-btn " + (channel() === "mcp" ? "active" : "")}
 onClick={() => setChannel("mcp")} style="font-size:11px;">D) MCP (Claude Desktop)</button>
 </div>
 </div>

 {/* 2단계 — 단일 통합 프롬프트 (채널·토큰·URL 모두 포함) */}
 <div style="margin-top:10px;">
 <div style="display:flex; justify-content:space-between; align-items:center; font-size:11px; color:var(--text-3);">
 <span>2) 선택한 채널 ({channel()}) 까지 포함된 통합 프롬프트:</span>
 <div>
 <button class="link-btn" onClick={() => promptVisible() ? setPromptVisible(false) : showPrompt()} style="font-size:11px;">
 {promptVisible() ? "접기" : "보기"}
 </button>
 <button class="link-btn" style="font-size:11px; margin-left:4px; background:#06c; color:white;"
 onClick={copyPrompt}>
 📋 통합 프롬프트 복사
 </button>
 </div>
 </div>
 <Show when={promptVisible()}>
 <pre id={`prompt-preview-${props.sessionId.replace(/[^a-z0-9]/gi,'')}`} style="margin-top:6px; padding:8px; background:var(--surface-2); border-radius:4px; font-size:10px; max-height:240px; overflow:auto; white-space:pre-wrap; line-height:1.4;">(로딩 중...)</pre>
 </Show>
 </div>

 {/* 3단계 — 채널별 가이드 */}
 <div style="margin-top:10px;">
 <Show when={channel() === "paste"}>
 <div style="font-size:11px; color:var(--text-3); margin-bottom:4px;">3) LLM 응답 JSON 을 아래에 붙여넣고 import:</div>
 <textarea
 value={text()}
 onInput={(e) => setText(e.currentTarget.value)}
 placeholder='{"openxgram_import_version":1, "session_id":"...", "items":[...]}'
 rows={6}
 style="width:100%; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-family:monospace; font-size:11px; box-sizing:border-box;"
 />
 <div style="display:flex; gap:6px; margin-top:4px;">
 <button class="link-btn" onClick={doImport} disabled={busy() || !text().trim()}
 style="background:#06c; color:white;">{busy() ? "import 중…" : "이 세션에 import"}</button>
 <button class="link-btn" onClick={() => { setText(""); setResult(null);}}>지우기</button>
 </div>
 </Show>
 <Show when={channel() === "webhook"}>
 <Show when={webhookData()?.webhook_url} fallback={
 <button class="link-btn" onClick={ensureWebhook}>+ Webhook URL 발급</button>
 }>
 <div style="font-size:11px; color:var(--text-3);">3) 이 URL 을 LLM 에 알려주면 직접 push 가능 (Bearer 없이):</div>
 <div style="background:var(--surface-2); padding:8px; border-radius:4px; margin:4px 0; font-family:monospace; font-size:10px; word-break:break-all;">{webhookData()?.webhook_url}</div>
 <button class="link-btn" onClick={() => { navigator.clipboard.writeText(webhookData()?.webhook_url ?? ""); alert("URL 복사됨");}}>📋 URL 복사</button>
 </Show>
 </Show>
 <Show when={channel() === "api"}>
 <div style="font-size:11px; color:var(--text-3); margin-bottom:4px;">3) LLM 또는 스크립트가 다음 curl 실행 (Bearer 토큰 필요):</div>
 <pre style="background:var(--surface-2); padding:8px; border-radius:4px; font-size:10px; white-space:pre-wrap; word-break:break-all;">{`curl -X POST "${window.location.origin}/v1/gui/memory/import/bundle" \\
  -H "Authorization: Bearer <SESSION_TOKEN>" \\
  -H "Content-Type: application/json" \\
  --data-binary @bundle.json`}</pre>
 </Show>
 <Show when={channel() === "mcp"}>
 <div style="font-size:11px; color:var(--text-3);">3) Claude Desktop / Cursor 에 OpenXgram MCP 등록되어 있으면, LLM 이 직접 호출:</div>
 <pre style="background:var(--surface-2); padding:8px; border-radius:4px; font-size:10px;">{`memory_import_bundle({\n  openxgram_import_version: 1,\n  session_id: "${props.sessionId}",\n  items: [...]\n})`}</pre>
 <p style="font-size:10px; color:var(--text-3);">MCP 등록: <code>xgram init</code> 자동 통합 — Claude Desktop 의 mcp.json 에 openxgram 추가.</p>
 </Show>
 </div>

 <Show when={result()}>
 <div style="margin-top:8px; padding:6px; background:var(--surface-2); border-radius:4px; font-size:11px;">{result()}</div>
 </Show>
 </section>
 );
}

// ── 탭 1: 개요 (L2 4-tuple) ─────────────────────────────────────
function Overview(props: { peer: PeerMeta}) {
 // identifier = peer.address (tmux 면 "tmux:name", peer 면 alias).
 const identifier = () => props.peer.address || props.peer.alias;
 const [aliases, { refetch}] = createResource<any>(async () => {
 try { return await invoke("session_aliases");} catch { return {};}
 });
 const currentDisplay = () => aliases()?.[identifier()]?.display_name ?? props.peer.alias;
 const [editing, setEditing] = createSignal(false);
 const [draft, setDraft] = createSignal("");
 async function save() {
 const name = draft().trim();
 if (!name) { alert("이름은 비울 수 없음"); return;}
 try {
 await invoke("session_alias_set", { identifier: identifier(), display_name: name});
 await refetch();
 setEditing(false);
 } catch (e) { alert("저장 실패: " + e);}
 }
 return (
 <div>
 <Row label="alias" value={props.peer.alias} />
 <div style="display:flex; align-items:center; gap:8px; padding:4px 0; border-bottom:1px solid var(--border);">
 <span style="min-width:80px; font-size:12px; color:var(--text-3);">display_name</span>
 <Show when={!editing()} fallback={
 <>
 <input
 value={draft()}
 onInput={(e) => setDraft(e.currentTarget.value)}
 maxlength="64"
 style="flex:1; padding:4px 6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:3px; font-size:12px;"
 onKeyDown={(e) => { if (e.key === "Enter") save(); if (e.key === "Escape") setEditing(false);}}
 />
 <button class="link-btn" onClick={save} style="font-size:11px;">저장</button>
 <button class="link-btn" onClick={() => setEditing(false)} style="font-size:11px;">취소</button>
 </>
 }>
 <span style="flex:1; font-size:12px;">{currentDisplay()}</span>
 <button class="link-btn" onClick={() => { setDraft(currentDisplay()); setEditing(true);}} style="font-size:11px;">편집</button>
 </Show>
 </div>
 <Row label="machine" value={props.peer.machine || "(unknown)"} />
 <Row
 label="address"
 value={props.peer.address ? `${props.peer.address.slice(0, 18)}…` : "(없음)"}
 mono
 />
 <Row label="public_key" value={fingerprint(props.peer.public_key_hex)} mono />
 <Row label="last_seen" value={props.peer.last_seen || "한 번도 본 적 없음"} />
 <p class="messenger-sidepanel-hint" style="margin-top:10px;">
 가져오기·내보내기는 우측 사이드패널 <strong>"가져오기·내보내기"</strong> 탭에서.
 </p>
 <p class="messenger-sidepanel-hint">
 display_name 은 DB v32 session_aliases 에 영구 저장. 사이드바에도 자동 반영 (다음 sessions poll 후).
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
function RoleTab(props: { peer: PeerMeta; onJumpToSettings: () => void}) {
 const [policies] = createResource(fetchRolePolicies);
 return (
 <div>
 <Row label="현재 역할" value="researcher (기본)" />
 <Row label="오케스트레이션" value="워커" />
 {/* rc.92 D2 — capabilities 표시 */}
 <Show when={props.peer.description}>
 <Row label="설명" value={props.peer.description!} />
 </Show>
 <Show when={(props.peer.capabilities?.length ?? 0) > 0}>
 <div style="padding:4px 0; border-bottom:1px dashed var(--border);">
 <span style="font-size:11px; color:var(--text-3); display:block; margin-bottom:3px;">capabilities</span>
 <div style="display:flex; flex-wrap:wrap; gap:4px;">
 <For each={props.peer.capabilities}>
 {(c) => <span style="font-size:11px; padding:1px 6px; background:var(--surface-2); border-radius:8px;">{c}</span>}
 </For>
 </div>
 </div>
 </Show>
 <Show when={!props.peer.description && (props.peer.capabilities?.length ?? 0) === 0}>
 <p style="font-size:11px; color:var(--text-3); padding:4px 0;">
 💡 capabilities 미등록 — 이 세션 Claude pane 에서 <code>register_subagent(role, description, capabilities)</code> 호출하면 자동 표시됩니다.
 </p>
 </Show>
 <hr style="margin:10px 0; opacity:0.2;" />
 <strong style="font-size:12px;">L3 + V1 — 역할별 auto_respond 마스터 정책</strong>
 <p class="messenger-sidepanel-hint">
 마스터 = {policies()?.master_card ?? "자율 행동 카드"}. 본 탭은 view.
 </p>
 <For each={policies()?.roles ?? []}>
 {(r) => (
 <div style="display:flex; justify-content:space-between; padding:3px 0; font-size:12px; border-bottom:1px dashed var(--border);">
 <span>{r.role}</span>
 <span style={r.auto_respond_default ? "color:#5fa;" : "color:var(--text-3);"}>
 {r.auto_respond_default ? " auto" : "× manual"} · max {r.max_concurrent}
 </span>
 </div>
)}
 </For>
 <button class="link-btn" type="button" onClick={props.onJumpToSettings} style="margin-top:10px;">
 자율 행동 카드 (마스터 편집)
 </button>
 </div>
);
}

// ── 탭 (메신저 등록) — 이 세션의 LLM 을 OpenXgram 메신저 에이전트로 등록.
// 외부 채널 바인딩(Discord/Telegram)과 별개. messenger_enabled=true 면 다른 peer 의
// list_peers 응답에 자동 노출 + group_name 으로 peer_send fan-out 대상.
interface AgentCapDto {
 alias: string;
 role: string | null;
 description: string | null;
 capabilities: string | null;
 tool_list: string | null;
 project_path: string | null;
 group_name: string | null;
 messenger_enabled: boolean;
 orchestration_role: string | null;
 special_instructions: string | null;
}
// rc.135 — 카탈로그-메신저 통합. 메신저 등록 탭에서 직접 173 템플릿 선택 → 폼 자동 채움.
interface TemplateDtoMini {
 id: string;
 category: string;
 name: string;
 description: string | null;
 emoji: string | null;
 vibe: string | null;
 body: string;
}
// rc.135b — 카테고리 drill-down + 짙은 배경.
// step 1: 카테고리 카드 그리드 (17개)
// step 2: 카테고리 클릭 시 그 안의 템플릿만 표시 + ← 뒤로
function CatalogPickerModal(props: {
 onClose: () => void;
 onSelect: (t: TemplateDtoMini) => void;
}) {
 const [templates] = createResource<TemplateDtoMini[]>(async () => {
 try { return await invoke<TemplateDtoMini[]>("agent_templates_list");} catch { return [];}
});
 const [category, setCategory] = createSignal<string | null>(null);
 const categoryStats = () => {
 const map = new Map<string, number>();
 (templates() ?? []).forEach((t) => map.set(t.category, (map.get(t.category) ?? 0) + 1));
 return Array.from(map.entries()).sort((a, b) => a[0].localeCompare(b[0]));
};
 const inCategory = () => {
 const c = category();
 if (!c) return [];
 return (templates() ?? []).filter((t) => t.category === c);
};
 // 카테고리별 emoji 매핑 (UX 향상)
 const catEmoji: Record<string, string> = {
 academic: "🎓", design: "🎨", engineering: "⚙️", finance: "💰",
 "game-development": "🎮", integrations: "🔌", marketing: "📣",
 "paid-media": "💸", product: "📦", "project-management": "📋",
 sales: "💼", scripts: "📜", "spatial-computing": "🥽",
 specialized: "🧠", strategy: "♟️", support: "🛟", testing: "🧪",
};
 return (
 <div onClick={props.onClose}
 style="position:fixed; inset:0; background:rgba(0,0,0,0.85); backdrop-filter:blur(4px); z-index:9999; display:flex; align-items:center; justify-content:center;">
 <div onClick={(e) => e.stopPropagation()}
 style="background:#0f1320; border:1px solid #3a4a6a; border-radius:10px; padding:16px; max-width:900px; width:92%; max-height:84vh; overflow:auto; box-shadow:0 20px 60px rgba(0,0,0,0.7);">
 <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:12px; padding-bottom:10px; border-bottom:1px solid #2a3550;">
 <Show when={category()} fallback={
 <strong style="font-size:15px; color:#e6e9f0;">📚 에이전트 카탈로그 — 카테고리 선택 ({(templates() ?? []).length}개 / {categoryStats().length} 카테고리)</strong>
 }>
 <button onClick={() => setCategory(null)}
 style="background:#1a2236; color:#e6e9f0; padding:5px 12px; border:1px solid #3a4a6a; border-radius:4px; cursor:pointer; font-size:12px;">
 ← 카테고리로
 </button>
 <strong style="font-size:14px; color:#e6e9f0; margin:0 12px; flex:1;">{catEmoji[category()!] || "📁"} {category()} ({inCategory().length})</strong>
 </Show>
 <button onClick={props.onClose}
 style="background:#2a1a1a; color:#e6e9f0; padding:5px 12px; border:1px solid #5a3a3a; border-radius:4px; cursor:pointer; font-size:12px;">
 닫기 ✕
 </button>
 </div>

 {/* Step 1: 카테고리 그리드 */}
 <Show when={!category()}>
 <Show when={(templates() ?? []).length === 0} fallback={
 <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(160px, 1fr)); gap:10px;">
 <For each={categoryStats()}>
 {([cat, count]) => (
 <div onClick={() => setCategory(cat)}
 style="padding:14px; border:1px solid #2a3550; border-radius:6px; cursor:pointer; background:#1a2236; text-align:center; transition:all 0.15s;"
 onMouseOver={(e) => e.currentTarget.style.background = '#243150'}
 onMouseOut={(e) => e.currentTarget.style.background = '#1a2236'}>
 <div style="font-size:30px; margin-bottom:6px;">{catEmoji[cat] || "📁"}</div>
 <div style="font-size:13px; color:#e6e9f0; font-weight:bold; margin-bottom:4px;">{cat}</div>
 <div style="font-size:11px; color:#8a92a8;">{count} 개</div>
 </div>
)}
 </For>
 </div>
 }>
 <div style="text-align:center; padding:40px 20px; color:#8a92a8;">
 ⚠ 카탈로그 비어있음.<br />
 홈 → 📚 에이전트 카탈로그 → 🔄 갱신 으로 fetch.
 </div>
 </Show>
 </Show>

 {/* Step 2: 그 카테고리 내 카드들 */}
 <Show when={category()}>
 <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(240px, 1fr)); gap:10px;">
 <For each={inCategory()}>
 {(t) => (
 <div onClick={() => { props.onSelect(t); props.onClose();}}
 style="padding:10px; border:1px solid #2a3550; border-radius:6px; cursor:pointer; background:#1a2236; transition:all 0.15s;"
 onMouseOver={(e) => e.currentTarget.style.background = '#243150'}
 onMouseOut={(e) => e.currentTarget.style.background = '#1a2236'}>
 <div style="display:flex; align-items:center; gap:6px; margin-bottom:4px;">
 <span style="font-size:18px;">{t.emoji || "🤖"}</span>
 <strong style="font-size:13px; color:#e6e9f0;">{t.name}</strong>
 </div>
 <Show when={t.vibe}><div style="font-size:11px; color:#a0a8c0; font-style:italic; margin-bottom:4px;">"{t.vibe}"</div></Show>
 <Show when={t.description}><div style="font-size:11px; color:#8a92a8; max-height:60px; overflow:hidden; line-height:1.4;">{t.description}</div></Show>
 </div>
)}
 </For>
 </div>
 </Show>
 </div>
 </div>
);
}

function MessengerRegisterTab(props: { peer: PeerMeta; onJumpToSettings: () => void}) {
 const alias = () => props.peer.alias;
 const [agents, { refetch}] = createResource<AgentCapDto[]>(async () => {
 try { return await invoke<AgentCapDto[]>("agents_list");} catch { return [];}
});
 // rc.135 — 카탈로그 picker
 const [showCatalog, setShowCatalog] = createSignal(false);
 function applyTemplate(t: TemplateDtoMini) {
 if (t.name) setRole(t.name);
 if (t.description) setDescription(t.description);
 setInstContent(t.body);
 setMsg(`✓ 카탈로그 적용: ${t.name} — 저장 버튼으로 확정`);
}
 // 통합: 기존 RoleTab 의 role policies (L3 + V1 마스터 정책) view
 const [policies] = createResource(fetchRolePolicies);
 // rc.129 — cwd/AGENT.md inline 편집
 const [instContent, setInstContent] = createSignal("");
 const [instFile, setInstFile] = createSignal("");
 const [instExists, setInstExists] = createSignal(false);
 const [instBusy, setInstBusy] = createSignal(false);
 const [instMsg, setInstMsg] = createSignal<string | null>(null);
 async function loadInstructions() {
 try {
 const r = await invoke<any>("agents_instructions_get", { alias: alias()});
 if (r?.ok) {
 let c = r.content || "";
 // rc.130 — 빈 파일 이면 placeholder template 자동 채움
 if (!c.trim()) {
 c = `# ${alias()}\n\n## 역할\n(예: PRD 작성, Rust 코어 구현, 테스트·검증)\n\n## 능력\n- ...\n- ...\n\n## 특수 지침\n(예외 처리, 보안 룰, 특별 행동 양식 등)\n\n---\n\nOpenXgram 표준 운영 가이드 (peer 통신·발신·Discord 카드 형식·오케스트레이션) 는 \`~/oxg.md\` 참조.\n`;
 }
 setInstContent(c);
 setInstFile(r.file || "");
 setInstExists(!!r.exists);
 }
} catch (e) { /* silent */}
}
 // rc.183 — 이슈 #8 fix: alias 가 진짜 변경됐을 때만 reload.
// 이전: 매 reactive trigger 마다 loadInstructions → setInstContent → textarea reset → 입력 안 됨.
let lastLoadedAlias: string | null = null;
createEffect(() => {
 const a = alias();
 if (a === lastLoadedAlias) return;  // 같은 alias 면 skip
 lastLoadedAlias = a;
 loadInstructions();
 // rc.130 — 진입 시 auto-detect 자동 호출 (수동 버튼 클릭 불필요)
 autoDetect();
});
 async function saveInstructions() {
 setInstBusy(true); setInstMsg(null);
 try {
 const r = await invoke<any>("agents_instructions_save", {
 alias: alias(), content: instContent(),
});
 setInstMsg(`✓ 저장: ${r?.file} (${r?.bytes} bytes)`);
 setInstExists(true);
 // 저장 후 auto-detect 재실행 → 폼 새 내용 반영
 await autoDetect();
} catch (e) { setInstMsg(`✗ ${e}`);} finally { setInstBusy(false);}
}
 const current = () => (agents() ?? []).find((a) => a.alias === alias()) || null;
 // 기존 등록된 orchestration_role 목록 (autocomplete 용)
 const existingOrchRoles = () => {
 const set = new Set<string>();
 (agents() ?? []).forEach((a) => { if (a.orchestration_role) set.add(a.orchestration_role);});
 return Array.from(set);
};
 const [role, setRole] = createSignal("");
 const [description, setDescription] = createSignal("");
 const [groupName, setGroupName] = createSignal("");
 const [orchRole, setOrchRole] = createSignal("");
 const [specialInst, setSpecialInst] = createSignal("");
 const [toolListJson, setToolListJson] = createSignal("");
 const [projectPath, setProjectPath] = createSignal("");
 const [msg, setMsg] = createSignal<string | null>(null);
 const [busy, setBusy] = createSignal(false);

 // 현재 등록 상태 로드되면 입력 필드 채움
 createEffect(() => {
 const c = current();
 if (c) {
 setRole(c.role || ""); setDescription(c.description || ""); setGroupName(c.group_name || "");
 setOrchRole(c.orchestration_role || ""); setSpecialInst(c.special_instructions || "");
 setToolListJson(c.tool_list || ""); setProjectPath(c.project_path || "");
 }
 });

 async function autoDetect(opts?: {manual?: boolean}) {
 // rc.161 — peer / tmux not found 같은 알려진 fail 은 silent. 사용자 버튼 클릭 시만 메시지.
 const manual = opts?.manual === true;
 if (manual) setBusy(true);
 if (manual) setMsg("🔍 감지 중...");
 try {
 const r = await invoke<any>("agents_auto_detect", { alias: alias()});
 if (r?.ok) {
 if (r.description) setDescription(r.description);
 if (r.tool_list) setToolListJson(r.tool_list);
 if (r.project_path) setProjectPath(r.project_path);
 if (manual) setMsg(`✓ 자동 감지 완료 (${r.project_path || "?"})`);
 } else {
 const errMsg = r?.error || "감지 실패";
 // tmux session not found / no cwd — peer 거나 stale alias. silent.
 if (manual && !/tmux session not found|cwd 추출 실패/.test(errMsg)) setMsg(`✗ ${errMsg}`);
 }
} catch (e) {
 const s = String(e);
 if (manual && !/tmux session not found|cwd 추출 실패/.test(s)) setMsg(`✗ ${s}`);
} finally { if (manual) setBusy(false);}
}

 async function save(enabled: boolean) {
 setBusy(true); setMsg(null);
 try {
 await invoke("agents_register", {
 alias: alias(),
 role: role().trim() || "agent",  // rc.158 — 빈 role default. backend NOT NULL 위반 회피
 description: description().trim() || null,
 group_name: groupName().trim() || null,
 orchestration_role: orchRole().trim() || null,
 special_instructions: specialInst().trim() || null,
 tool_list: toolListJson().trim() || null,
 project_path: projectPath().trim() || null,
 messenger_enabled: enabled,
});
 setMsg(`✓ 저장 (messenger_enabled=${enabled})`);
 await refetch();
} catch (e) { setMsg(`✗ ${e}`);} finally { setBusy(false);}
}

 return (
 <div>
 <p style="font-size:12px; color:var(--text-3); margin-bottom:10px;">
 이 세션(<code>{alias()}</code>) 을 OpenXgram 메신저 에이전트로 등록.<br />
 등록 후 다른 LLM 이 <code>openxgram.list_peers</code> 호출 시 자동으로 이 에이전트를 인지.
 group 지정 시 <code>peer_send(alias=group)</code> 으로 한 번에 fan-out.
 </p>
 <Show when={current()?.messenger_enabled} fallback={
 <div style="padding:6px 10px; font-size:11px; background:rgba(220,53,69,0.15); border-radius:4px; margin-bottom:8px;">
 ⚠ 미등록 또는 비활성 상태 — 다른 peer 에게 안 보임
 </div>
 }>
 <div style="padding:6px 10px; font-size:11px; background:rgba(35,134,54,0.2); border-radius:4px; margin-bottom:8px;">
 ✓ 메신저 활성 — 다른 peer 의 list_peers 에 노출 중
 </div>
 </Show>
 <div style="display:flex; flex-direction:column; gap:6px;">
 <div style="display:flex; gap:6px; flex-wrap:wrap;">
 <button class="link-btn" disabled={busy()} onClick={() => autoDetect({manual: true})}
 style="background:#3a4a6a; color:white; padding:6px 14px; border:none; border-radius:4px;">
 🔍 자동 감지 (CLAUDE.md + .mcp.json)
 </button>
 <button class="link-btn" type="button" onClick={() => setShowCatalog(true)}
 style="background:#5c2d91; color:white; padding:6px 14px; border:none; border-radius:4px;"
 title="agency-agents 173개 템플릿에서 선택 → role/description/AGENT.md 자동 채움">
 📚 카탈로그 적용 (173개)
 </button>
 </div>
 <label style="font-size:11px; color:var(--text-3);">역할 (role) — 짧은 직책</label>
 <input value={role()} onInput={(e) => setRole(e.currentTarget.value)}
 placeholder="예: PRD 작성, Rust 코어 구현, 테스트·검증" style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <label style="font-size:11px; color:var(--text-3);">오케스트레이션 역할 (자유 입력, 기존 list autocomplete)</label>
 <input value={orchRole()} onInput={(e) => setOrchRole(e.currentTarget.value)} list="orch-roles-list"
 placeholder="예: coordinator / worker / reviewer / researcher / specialist:rust ..." style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <datalist id="orch-roles-list">
 <For each={existingOrchRoles()}>{(r) => <option value={r} />}</For>
 </datalist>
 {/* rc.130 — description 폼 제거. AGENT.md 의 내용이 자동으로 description 으로 사용됨 (아래 \"지침 직접 편집\" 섹션). */}
 <p style="font-size:11px; color:var(--text-3); padding:4px 6px; background:var(--surface-2); border-radius:4px; margin:0;">
 💡 <strong>설명</strong>은 아래 <strong>📝 지침 직접 편집</strong> (cwd/AGENT.md) 의 내용이 자동 사용됨.
 </p>
 <label style="font-size:11px; color:var(--text-3);">그룹 (선택, peer_send fan-out 단위)</label>
 <input value={groupName()} onInput={(e) => setGroupName(e.currentTarget.value)}
 placeholder="예: prd-team / dev-team / portal-team" style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <label style="font-size:11px; color:var(--text-3);">특수 지침 (선택)</label>
 <textarea value={specialInst()} onInput={(e) => setSpecialInst(e.currentTarget.value)}
 placeholder="예외 처리, 보안 룰, 특별 행동 양식 등" rows={2}
 style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-family:inherit;" />
 <Show when={projectPath()}>
 <label style="font-size:11px; color:var(--text-3);">프로젝트 경로 (자동 감지됨)</label>
 <code style="font-size:10px; padding:4px 6px; background:var(--surface-2); border-radius:3px;">{projectPath()}</code>
 </Show>
 <Show when={toolListJson()}>
 <label style="font-size:11px; color:var(--text-3);">MCP 도구 (자동 감지됨, read-only)</label>
 <code style="font-size:10px; padding:4px 6px; background:var(--surface-2); border-radius:3px; word-break:break-all;">{toolListJson()}</code>
 </Show>
 <div style="display:flex; gap:6px; margin-top:4px;">
 <button class="link-btn" disabled={busy()} onClick={() => save(true)}
 style="background:#238636; color:white; padding:6px 14px; border:none; border-radius:4px;">
 ▶ 저장 + 메신저 활성
 </button>
 <button class="link-btn" disabled={busy()} onClick={() => save(false)}
 style="background:var(--surface-2); color:var(--text-1); padding:6px 14px; border:1px solid var(--border); border-radius:4px;">
 저장만 (비활성)
 </button>
 </div>
 <Show when={msg()}>
 <div style={`padding:6px 10px; font-size:11px; border-radius:4px; background:${msg()!.startsWith("✓") || msg()!.startsWith("🔍") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>{msg()}</div>
 </Show>
 </div>

 {/* rc.129 — 지침 파일 (cwd/AGENT.md) inline 편집 */}
 <hr style="margin:14px 0 8px; opacity:0.2;" />
 <strong style="font-size:12px;">📝 지침 직접 편집 <code style="font-size:10px;">{instFile() || "AGENT.md"}</code></strong>
 <p class="messenger-sidepanel-hint" style="margin:4px 0;">
 이 에이전트의 역할·규칙을 마크다운으로 작성. 저장 시 cwd/AGENT.md 갱신 + 자동 감지 재실행 (위 폼 갱신).
 {!instExists() && <span style="color:#d29922;"> (아직 파일 없음 — 저장 시 생성)</span>}
 </p>
 <textarea value={instContent()} onInput={(e) => setInstContent(e.currentTarget.value)} rows={10}
 placeholder={`# 에이전트 정체성\\n\\nrole: ...\\ndescription: ...\\ncapabilities: [...]\\n\\n## 특수 지침\\n...`}
 style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-family:monospace; font-size:11px; width:100%; box-sizing:border-box;" />
 <div style="display:flex; gap:6px; margin-top:4px;">
 <button class="link-btn" disabled={instBusy()} onClick={saveInstructions}
 style="background:#238636; color:white; padding:6px 14px; border:none; border-radius:4px;">
 💾 저장 (cwd/AGENT.md)
 </button>
 <button class="link-btn" disabled={instBusy()} onClick={loadInstructions}
 style="padding:6px 14px;">↻ 다시 불러오기</button>
 </div>
 <Show when={instMsg()}>
 <div style={`padding:6px 10px; font-size:11px; border-radius:4px; margin-top:4px; background:${instMsg()!.startsWith("✓") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>{instMsg()}</div>
 </Show>

 {/* L3 + V1 — 역할별 auto_respond 마스터 정책 (기존 RoleTab 통합) */}
 <hr style="margin:14px 0 8px; opacity:0.2;" />
 <strong style="font-size:12px;">L3 + V1 — 역할별 auto_respond 마스터 정책</strong>
 <p class="messenger-sidepanel-hint" style="margin:4px 0;">
 마스터 = {policies()?.master_card ?? "자율 행동 카드"}. 본 섹션은 view.
 </p>
 <For each={policies()?.roles ?? []}>
 {(r) => (
 <div style="display:flex; justify-content:space-between; padding:3px 0; font-size:11px; border-bottom:1px dashed var(--border);">
 <span>{r.role}</span>
 <span style={r.auto_respond_default ? "color:#5fa;" : "color:var(--text-3);"}>
 {r.auto_respond_default ? " auto" : "× manual"} · max {r.max_concurrent}
 </span>
 </div>
)}
 </For>
 <button class="link-btn" type="button" onClick={props.onJumpToSettings} style="margin-top:8px;">
 자율 행동 카드 (마스터 편집)
 </button>
 <Show when={showCatalog()}>
 <CatalogPickerModal
 onClose={() => setShowCatalog(false)}
 onSelect={applyTemplate}
 />
 </Show>
 </div>
);
}

// ── 탭 3: 채널 바인딩 (메신저 §5 탭 3) — 세션별 채널 + 권한·멘션 트리거 ─────
interface BindingDto {
 id: string;
 platform: string;
 channel_ref: string;
 bot_label: string | null;
 bot_id?: string | null;
 mention_trigger: string | null;
 permission: string;
 active: boolean;
}
function ChannelTab(props: { notify: NotifyStatus | null; onJumpToSettings: () => void; agentId: string}) {
 const [bindings, { refetch}] = createResource(() => props.agentId, async (aid) => {
 try { return await invoke<BindingDto[]>("session_bindings_list", { agent_id: aid});} catch { return [];}
});
 const [bots, { refetch: refetchBots}] = createResource<any[]>(async () => {
 try { return await invoke<any[]>("discord_bots_list");} catch { return [];}
});
 const [platform, setPlatform] = createSignal("discord");
 const [channelRef, setChannelRef] = createSignal("");
 const [mention, setMention] = createSignal("");
 const [botId, setBotId] = createSignal("");
 const [busy, setBusy] = createSignal(false);
 const [testResult, setTestResult] = createSignal<string | null>(null);
 // rc.92 통합 — botId 변경 시 그 봇의 채널 list 자동 조회
 // botId 가 비어있으면(=봇 미선택) fetch 자체 skip — source falsy 시 createResource fetcher 미실행
 const [channelOpts] = createResource(
 () => (platform() === "discord" && botId() ? { bid: botId()} : null),
 async ({bid}) => {
 try {
 const r = await invoke<any>("discord_bot_channels", { bot_id: bid});
 return r?.channels ?? [];
 } catch { return [];}
 },
);
 // 봇 추가 inline 폼 (Discord) — token 만 입력, alias 는 검증 후 자동
 const [showAddBot, setShowAddBot] = createSignal(false);
 const [newBotToken, setNewBotToken] = createSignal("");
 // 봇 추가 inline 폼 (Telegram — single, notify.toml 저장)
 const [showAddTg, setShowAddTg] = createSignal(false);
 const [tgToken, setTgToken] = createSignal("");
 async function addTgBotInline() {
 if (!tgToken().trim()) { alert("Telegram bot token 필요 (BotFather)"); return; }
 setBusy(true);
 try {
 const v = await invoke<any>("notify_telegram_validate", { token: tgToken().trim() });
 alert("✓ 검증 통과: " + (v?.bot_username || "unknown") + "\n다음: 메시지 1개 보내고 '자동감지' 클릭 → chat_id 등록 → 저장.");
 setShowAddTg(false);
 // chat_id 자동감지 단계로 진행 — 본 form 닫고 사용자가 자동감지 버튼 누름.
 } catch (e) { alert("validate 실패: " + e); }
 finally { setBusy(false); }
 }
 async function addBotInline() {
 const tok = newBotToken().trim();
 if (!tok) { alert("Discord Bot Token 필요"); return;}
 setBusy(true);
 try {
 // 1) token 검증 → bot_label(= bot 이름) 자동 획득 (사용자가 alias 만들 필요 없음)
 let autoAlias = "";
 try {
 const v = await invoke<{bot_label: string}>("notify_discord_validate", { token: tok});
 autoAlias = v?.bot_label || "";
 } catch (e) {
 alert("Token 검증 실패: " + e + "\nDeveloper Portal 에서 Reset Token 후 다시 시도.");
 setBusy(false);
 return;
 }
 // 2) 등록 (alias 자리에 검증된 bot 이름)
 const r = await invoke<any>("discord_bots_add", { alias: autoAlias || `bot-${tok.slice(-6)}`, bot_token: tok});
 alert("✓ 봇 등록: " + (r.bot_username || autoAlias));
 setNewBotToken(""); setShowAddBot(false);
 await refetchBots();
 } catch (e) { alert("실패: " + e);} finally { setBusy(false);}
}
 async function add() {
 if (!channelRef()) { alert("channel_id 또는 chat_id 입력 필요"); return; }
 setBusy(true);
 try {
 await invoke("session_binding_add", {
 agent_id: props.agentId,
 platform: platform(),
 channel_ref: channelRef(),
 mention_trigger: mention(),
 permission: "reply",
 bot_id: botId() || null,
});
 setChannelRef("");
 setMention("");
 setBotId("");
 await refetch();
} catch (e) { alert("저장 실패: " + e); } finally { setBusy(false);}
}
 async function del(id: string) {
 if (!confirm("이 바인딩 삭제?")) return;
 setBusy(true);
 try {
 await invoke("session_binding_delete", { agent_id: props.agentId, binding_id: id});
 await refetch();
} finally { setBusy(false);}
}
 async function testChannel(b: BindingDto) {
 setTestResult(null);
 setBusy(true);
 try {
 const r = await invoke<{ok: boolean; message?: string}>("notify_channel_test", {
 platform: b.platform,
 channel_ref: b.channel_ref,
 text: `[OpenXgram test] ${new Date().toLocaleString("ko-KR")} — 바인딩 ${b.id} OK`,
 bot_id: b.bot_id ?? undefined,
 agent_id: props.agentId,
});
 setTestResult(r.ok ? `✓ 전송 성공 (${b.platform}:${b.channel_ref})` : `✗ ${r.message || "실패"}`);
} catch (e) {
 setTestResult(`✗ ${e}`);
} finally { setBusy(false);}
}
 return (
 <div>
 <p style="font-size:12px; margin-bottom:8px;">
 이 세션 (<code>{props.agentId}</code>) 의 채널 바인딩 — 메시지 양방향 + 풀 액세스. 아래 "+ 봇" 으로 토큰 등록 후 채널 선택.
 </p>
 <strong style="font-size:12px;">바인딩 추가</strong>
 <div style="display:flex; flex-direction:column; gap:6px; margin-top:6px;">
 {/* platform + bot 선택 같은 줄 */}
 <div style="display:flex; gap:4px; flex-wrap:wrap;">
 <select value={platform()} onChange={(e) => setPlatform(e.currentTarget.value)}
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="discord">Discord</option>
 <option value="telegram">Telegram</option>
 <option value="slack">Slack</option>
 <option value="web">Web</option>
 </select>
 <Show when={platform() === "discord"}>
 <Show when={(bots() ?? []).length > 0}>
 <select value={botId()} onChange={(e) => setBotId(e.currentTarget.value)}
 style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="">— 봇 선택 —</option>
 <For each={bots() ?? []}>
 {(b) => <option value={b.id}>{b.alias} ({b.bot_user_id?.slice(0, 8)})</option>}
 </For>
 </select>
 </Show>
 <button type="button" class="link-btn" onClick={() => setShowAddBot(!showAddBot())}
 title="새 디스코드 봇 등록 (토큰 입력)"
 style="padding:4px 8px; background:var(--surface-2);">+ 봇</button>
 </Show>
 </div>
 {/* 봇 추가 inline 폼 — Discord Bot Token 한 칸만. 봇 이름은 등록 시 자동 획득. */}
 <Show when={showAddBot()}>
 <div style="padding:8px; background:var(--surface-2); border:1px solid var(--border); border-radius:4px;">
 <label style="display:block; font-size:11px; color:var(--text-3); margin-bottom:4px;">
 🔑 <strong>Discord Bot Token</strong> — discord.com/developers/applications → Bot → Reset Token
 </label>
 <input value={newBotToken()} onInput={(e) => setNewBotToken(e.currentTarget.value)}
 placeholder="MTQ4NDYx... 토큰을 여기 붙여넣기" type="password" autocomplete="off"
 style="width:100%; padding:6px; margin-bottom:6px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:3px; box-sizing:border-box;" />
 <button type="button" class="link-btn" onClick={addBotInline} disabled={busy()}
 style="background:#238636; color:white; padding:6px 12px; border:none; border-radius:3px;">
 ▶ 봇 등록 (자동 검증 + 이름 획득)
 </button>
 <button type="button" class="link-btn" onClick={() => setShowAddBot(false)} style="margin-left:4px;">취소</button>
 </div>
 </Show>
 {/* 채널 선택 — 봇 선택 후에만 표시 (토큰 등록 → 봇 선택 → 채널 목록 자동 조회) */}
 <Show when={platform() === "discord"}>
 <Show when={(bots() ?? []).length === 0}>
 <div style="padding:6px 8px; font-size:11px; color:#d29922; background:rgba(210,153,34,0.1); border-radius:4px;">
 ⚠ 등록된 봇이 없습니다. 위 <strong>"+ 봇"</strong> 을 눌러 토큰을 먼저 입력하세요.
 </div>
 </Show>
 <Show when={(bots() ?? []).length > 0 && !botId()}>
 <div style="padding:6px 8px; font-size:11px; color:var(--text-3); background:var(--surface-2); border-radius:4px;">
 위 dropdown 에서 봇을 선택하면 채널 목록이 자동으로 나타납니다.
 </div>
 </Show>
 <Show when={botId()}>
 <Show when={(channelOpts() ?? []).length > 0}
 fallback={<div style="padding:6px 8px; font-size:11px; color:#f85149; background:rgba(248,81,73,0.1); border-radius:4px;">
 ⚠ 이 봇이 가입한 서버가 없거나 권한 부족 — Developer Portal 에서 봇을 서버에 재초대 필요.
 </div>}>
 <select value={channelRef()} onChange={(e) => setChannelRef(e.currentTarget.value)}
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="">— 채널 선택 —</option>
 <For each={channelOpts() ?? []}>
 {(c: any) => <option value={c.channel_id}>{c.guild_name} / #{c.channel_name}</option>}
 </For>
 </select>
 </Show>
 </Show>
 </Show>
 <Show when={platform() === "telegram"}>
 <div style="display:flex; flex-direction:column; gap:6px;">
 {/* Telegram 봇 토큰 등록 (single, notify.toml) */}
 <button type="button" class="link-btn" onClick={() => setShowAddTg(!showAddTg())}
 style="padding:4px 8px; background:var(--surface-2); align-self:flex-start;">+ Telegram 봇 등록 (token)</button>
 <Show when={showAddTg()}>
 <div style="padding:8px; background:var(--surface-2); border:1px solid var(--border); border-radius:4px;">
 <input value={tgToken()} onInput={(e) => setTgToken(e.currentTarget.value)}
 placeholder="Telegram Bot Token (BotFather 발급)" type="password"
 style="width:100%; padding:4px; margin-bottom:4px; background:var(--surface); color:var(--text-1); border:1px solid var(--border); border-radius:3px; box-sizing:border-box;" />
 <button type="button" class="link-btn" onClick={addTgBotInline} disabled={busy()}
 style="background:#06c; color:white; padding:4px 10px; border:none; border-radius:3px;">▶ 검증 + 등록</button>
 <button type="button" class="link-btn" onClick={() => setShowAddTg(false)} style="margin-left:4px;">취소</button>
 </div>
 </Show>
 <div style="display:flex; gap:4px;">
 <input value={channelRef()} onInput={(e) => setChannelRef(e.currentTarget.value)}
 placeholder="chat_id (자동감지 권장)"
 style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <button class="link-btn" style="background:#06c; color:white; padding:4px 8px; white-space:nowrap;"
 onClick={async () => {
 try {
 const r = await invoke<any>("notify_telegram_detect_chat_saved");
 if (r?.found && r?.chat_id) setChannelRef(String(r.chat_id));
 else alert(r?.hint || "chat_id 감지 실패");
 } catch (e) { alert("실패: " + e);}
 }}>▶ 자동감지</button>
 </div>
 </div>
 </Show>
 <Show when={platform() !== "discord" && platform() !== "telegram"}>
 <input value={channelRef()} onInput={(e) => setChannelRef(e.currentTarget.value)}
 placeholder="channel"
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 </Show>
 </div>
 <div style="display:flex; gap:4px; margin-top:6px; flex-wrap:wrap;">
 <input value={mention()} onInput={(e) => setMention(e.currentTarget.value)}
 placeholder="멘션 (선택, 비우면 모든 메시지)"
 title="채널 메시지에 이 문자열이 포함될 때만 세션으로 전달. 예: @researcher / @all"
 style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <Show when={platform() === "discord" && (bots() ?? []).length > 0}>
 <select value={botId()} onChange={(e) => setBotId(e.currentTarget.value)}
 title="이 채널에 사용할 봇"
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="">— 봇 선택 —</option>
 <For each={bots() ?? []}>
 {(b) => <option value={b.id}>{b.alias}</option>}
 </For>
 </select>
 </Show>
 <button class="link-btn" onClick={add} disabled={busy()}
 style="background:#238636; color:white; padding:4px 10px; border:none; border-radius:4px;">
 ▶ 바인딩 저장
 </button>
 </div>
 <p style="font-size:11px; color:var(--text-3); margin-top:4px;">
 💡 바인딩 = 채널 ↔ 세션 양방향. 채널 메시지는 세션 pane 으로, 세션 응답은 채널로 자동 reply.
 </p>
 <hr style="margin:8px 0; opacity:0.2;" />
 <strong style="font-size:12px;">활성 바인딩 ({bindings()?.length ?? 0})</strong>
 <Show when={testResult()}>
 <div style={`margin-top:4px; padding:4px 8px; font-size:11px; border-radius:4px; background:${testResult()!.startsWith("✓") ? "rgba(35,134,54,0.2)" : "rgba(220,53,69,0.2)"};`}>
 {testResult()}
 </div>
 </Show>
 <For each={bindings() ?? []}>
 {(b) => (
 <div style="padding:6px; border-bottom:1px solid var(--border); font-size:12px;">
 <div style="display:flex; justify-content:space-between; align-items:center; gap:4px;">
 <div style="flex:1; min-width:0;">
 <strong>{b.platform}</strong> · <code style="word-break:break-all;">{b.channel_ref}</code>
 {b.mention_trigger ? <span style="color:var(--text-3);"> · 멘션: <code>{b.mention_trigger}</code></span> : <span style="color:var(--text-3);"> · 모든 메시지</span>}
 </div>
 <button class="link-btn" onClick={() => testChannel(b)} disabled={busy()}
 title="이 채널에 테스트 메시지 전송"
 style="padding:2px 8px;">▶ 테스트</button>
 <button class="link-btn" onClick={() => del(b.id)} disabled={busy()}
 style="padding:2px 8px; color:#f85149;">삭제</button>
 </div>
 </div>
)}
 </For>
 <button class="link-btn" type="button" onClick={props.onJumpToSettings} style="margin-top:8px;">
 채널 카드 (봇 토큰 마스터)
 </button>
 </div>
);
}

// ── 탭 4: 상태·리소스 ──────────────────────────────────────────
function StatusTab(props: { peer: PeerMeta}) {
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
 master: { address: string | null; free_micro: number; last_synced_at: string};
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
function WalletTab(props: { peer: PeerMeta}) {
 const [w, { refetch}] = createResource(fetchWallets);
 const [busy, setBusy] = createSignal(false);
 const [err, setErr] = createSignal<string | null>(null);
 const ownWallet = () =>
 w()?.sub_wallets.find((s) => s.agent_id === props.peer.alias) || null;
 async function createWallet() {
 setBusy(true);
 setErr(null);
 try {
 await invoke("wallet_create", { agent_id: props.peer.alias});
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
 <strong style="font-size:12px;">마스터 지갑 ( 신원)</strong>
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
 value={s().auto_topup_enabled ? " 활성" : "비활성"}
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
 <p style="color:#f88; font-size:11px; margin-top:8px;"> {err()}</p>
 </Show>
 <p class="messenger-sidepanel-hint">
 L4: derivation_index 영구 점유 (Decommissioned 도 재사용 X). 마스터 지갑 고급 = 신원 카드.
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
 return await invoke<MessageItem[]>("messages_recent", { limit: 50});
} catch {
 return [];
}
}
function HistoryTab(props: { peer: PeerMeta}) {
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

// ── 탭 6: 가져오기·내보내기 (사양 §5 탭 6) — 서버 export API + import bundle ──
function ExportTab(props: { peer: PeerMeta}) {
 const identifier = () => props.peer.address || props.peer.alias;
 const safe = () => identifier().slice(0, 40);
 return (
 <div>
 <section style="margin-bottom:12px;">
 <strong style="font-size:13px;">내보내기 (이 세션)</strong>
 <p style="font-size:11px; color:var(--text-3); margin:4px 0;">
 messages 테이블에 적재된 모든 메시지 다운로드 (Claude Code .jsonl 자동 ingest 포함).
 </p>
 <div style="display:flex; gap:6px; flex-wrap:wrap; margin-top:6px;">
 <a class="link-btn" href={`/v1/gui/memory/export/session/${encodeURIComponent(identifier())}?format=md`}
 download={`session-${safe()}.md`} style="text-decoration:none;">.md</a>
 <a class="link-btn" href={`/v1/gui/memory/export/session/${encodeURIComponent(identifier())}?format=jsonl`}
 download={`session-${safe()}.jsonl`} style="text-decoration:none;">.jsonl</a>
 <a class="link-btn" href={`/v1/gui/memory/migration/export/${encodeURIComponent(identifier())}`}
 download={`migration-${safe()}.json`} style="text-decoration:none;">migration .json</a>
 </div>
 </section>
 <SessionImportSection sessionId={identifier()} />
 </div>
 );
}

// ── 탭 8: 토큰 (사양 §5 탭 8, S6 합산) ──
function TokensTab(props: { peer: PeerMeta}) {
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
function CronTab(props: { onJumpToSettings: () => void}) {
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
 자율 행동 카드 (모든 cron)
 </button>
 </div>
);
}

// ── 탭 10: 파일·지침 (사양 §5 탭 10) ──
function FilesTab(props: { peer: PeerMeta}) {
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
function NotifyTab(_props: { notify: NotifyStatus | null}) {
 return (
 <div>
 <p style="font-size:12px; color:var(--text-3); margin-bottom:8px;">
 알림은 채널 바인딩(채널 바인딩 탭) 을 통해 발신됩니다. 봇 등록·채널 선택은 거기서.
 </p>
 <p class="messenger-sidepanel-hint">
 규칙 추가 (비용 한도 80%·1h 무응답·서브에이전트 3회 실패·Cron 실패) — 백엔드 notification_rules 테이블 신설 필요.
 </p>
 </div>
);
}

// ── 탭 12: 권한·도구·MCP (사양 §5 탭 12, V9 default-deny) ──
function PermissionsTab(props: { onJumpToSettings: () => void}) {
 return (
 <div>
 <strong style="font-size:12px;">도구 권한 (현재 default-deny)</strong>
 <Row label="파일 read" value="" />
 <Row label="파일 write (cwd)" value="" />
 <Row label="파일 delete" value="" />
 <Row label="shell 실행" value="" />
 <Row label="네트워크 (allowlist)" value="" />
 <Row label="외부 LLM 호출" value="" />
 <Row label="결제 (서브 지갑 한도)" value="" />
 <hr style="margin:10px 0; opacity:0.2;" />
 <strong style="font-size:12px;">MCP 서버 ( Vault·MCP 카드)</strong>
 <p class="messenger-sidepanel-hint">
 외부 DID allowlist (N9 default-deny) 마스터 = 신원 카드. 세션 override 불가 (V9).
 </p>
 <button class="link-btn" type="button" onClick={props.onJumpToSettings}>
 도구·Vault·MCP 카드
 </button>
 </div>
);
}

// ── 공용 ──
function Row(props: { label: string; value: string; mono?: boolean}) {
 return (
 <div class="messenger-sidepanel-row">
 <span class="label">{props.label}</span>
 <span class={`value${props.mono ? " mono" : ""}`}>{props.value}</span>
 </div>
);
}
