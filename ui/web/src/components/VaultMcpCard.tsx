import { createResource, createSignal, For, Show} from "solid-js";
import { VaultView} from "./VaultView";
import { Breadcrumb} from "./Breadcrumb";
import { invoke} from "@/api/client";

async function fetchMcpServers(): Promise<any[]> { try { return await invoke("vault_mcp_servers_list");} catch { return [];}}
async function fetchToolCatalog(): Promise<any[]> { try { return await invoke("vault_tool_catalog");} catch { return [];}}

// UI-VAULT-MCP-SPEC v1.0 §3 — 도구·Vault·MCP 카드 (PRD §0 #8).
// 4 탭: 시크릿 · MCP 서버 · 도구 카탈로그 · 감사 로그.

type Tab = "secret" | "mcp" | "tool" | "audit";

export function VaultMcpCard(props: { onBack: () => void}) {
 const [tab, setTab] = createSignal<Tab>("secret");

 return (
 <div class="card-page">
 <Breadcrumb cardName=" 도구·Vault·MCP" onReturn={props.onBack} />
 <div class="card-page-head">
 <span class="icon"></span>
 <h1>도구·Vault·MCP</h1>
 </div>
 <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #8 — 도구·Vault·MCP</div>
 <div class="card-page-oneline">
 시크릿 저장 (ChaCha20-Poly1305) · MCP 서버 등록 · 도구 카탈로그 · default-deny 정책 · 감사 로그
 </div>

 <nav style="display:flex; gap:4px; margin-bottom:14px;">
 <button class={"link-btn " + (tab() === "secret" ? "active" : "")} onClick={() => setTab("secret")}> 시크릿</button>
 <button class={"link-btn " + (tab() === "mcp" ? "active" : "")} onClick={() => setTab("mcp")}> MCP 서버</button>
 <button class={"link-btn " + (tab() === "tool" ? "active" : "")} onClick={() => setTab("tool")}> 도구 카탈로그</button>
 <button class={"link-btn " + (tab() === "audit" ? "active" : "")} onClick={() => setTab("audit")}> 감사 로그</button>
 </nav>

 <Show when={tab() === "secret"}>
 <section class="card-section">
 <h3> 시크릿 (Secret)</h3>
 <p class="placeholder-note">
 기존 VaultView 통합. 사양 §3.1 — API 키·봇 토큰·DB 자격·webhook 등. `vault://&lt;path&gt;` 핸들로 다른 카드에서 참조만.
 </p>
 <VaultView />
 </section>
 </Show>

 <Show when={tab() === "mcp"}>
 <McpSection />
 </Show>

 <Show when={tab() === "tool"}>
 <ToolCatalogSection />
 </Show>

 <Show when={tab() === "audit"}>
 <VaultAuditSection />
 </Show>
 </div>
);
}

function McpSection() {
 const [list, { refetch}] = createResource(fetchMcpServers);
 const [name, setName] = createSignal("");
 const [transport, setTransport] = createSignal("stdio");
 const [command, setCommand] = createSignal("");
 const [url, setUrl] = createSignal("");
 async function add() {
 if (!name()) return;
 try { await invoke("vault_mcp_server_add", { name: name(), transport: transport(), command: command(), url: url(), scope: "user"}); setName(""); setCommand(""); setUrl(""); await refetch();} catch (e) { alert(String(e));}
}
 return (
 <section class="card-section">
 <h3>내가 쓰는 MCP 서버 (consume) — 사양 §3.2</h3>
 <p style="font-size:11px; color:var(--text-3); margin-bottom:6px;">
 OpenXgram 에이전트가 호출하는 외부 MCP 서버 (filesystem / brave-search 등 stdio·http).
 ⚠️ 반대 방향 ("내가 노출하는 MCP 도구" — Claude Desktop 등이 호출) 는 별도: <code>xgram mcp-serve</code> systemd unit.
 </p>
 <div style="display:flex; gap:4px; margin-bottom:6px;">
 <input value={name()} onInput={(e) => setName(e.currentTarget.value)} placeholder="이름 (filesystem)" style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <select value={transport()} onChange={(e) => setTransport(e.currentTarget.value)} style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="stdio">stdio</option>
 <option value="http">http</option>
 </select>
 </div>
 <Show when={transport() === "stdio"}>
 <input value={command()} onInput={(e) => setCommand(e.currentTarget.value)} placeholder="command (npx @x/mcp-fs)" style="width:100%; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; margin-bottom:4px;" />
 </Show>
 <Show when={transport() === "http"}>
 <input value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder="URL (http://localhost:9000)" style="width:100%; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; margin-bottom:4px;" />
 </Show>
 <button class="link-btn" onClick={add}>+ 등록</button>
 <h4 style="margin:8px 0 4px; font-size:12px;">등록된 서버 ({(list() ?? []).length})</h4>
 <For each={list() ?? []}>{(s: any) => (
 <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);">
 <strong>{s.name}</strong> · {s.transport} · scope:{s.scope} · {s.health_status || "unknown"}
 <div style="color:var(--text-3); font-size:11px;">{s.command || s.url || "—"}</div>
 </div>
)}</For>
 </section>
);
}

function VaultAuditSection() {
 const [audit] = createResource<any[]>(async () => { try { return await invoke<any[]>("identity_audit");} catch { return [];}});
 return (
 <section class="card-section">
 <h3> Vault 감사 로그 — 사양 §3.4 (M-11 영구)</h3>
 <p style="font-size:11px; color:var(--text-3);">시크릿 접근·등록·로테이션 영구 기록.</p>
 <For each={(audit() ?? []).slice(0, 50)}>{(a: any) => (
 <div style="font-size:11px; padding:4px 0; border-bottom:1px solid var(--border);">
 <span style="color:var(--text-3);">{a.created_at}</span> · <strong>{a.event_type}</strong>
 {a.target && <span style="margin-left:6px;"> → {a.target}</span>}
 </div>
)}</For>
 <Show when={(audit() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">감사 로그 없음.</p>
 </Show>
 </section>
);
}

function ToolCatalogSection() {
 const [list, { refetch}] = createResource(fetchToolCatalog);
 async function setPolicy(tool: string, policy: string) {
 try { await invoke("vault_tool_acl_set", { tool_name: tool, default_policy: policy, description: null}); await refetch();} catch (e) { alert(String(e));}
}
 return (
 <section class="card-section">
 <h3> 도구 카탈로그 — 사양 §3.3 (default-deny)</h3>
 <For each={list() ?? []}>{(t: any) => (
 <div style="display:flex; justify-content:space-between; align-items:center; padding:6px 0; border-bottom:1px solid var(--border);">
 <div style="flex:1;">
 <strong style="font-size:13px;">{t.tool_name}</strong>
 <span style="color:var(--text-3); font-size:11px; margin-left:6px;">{t.description}</span>
 <span style="color:var(--text-3); font-size:10px; margin-left:6px;">[{t.source}]</span>
 </div>
 <select value={t.default_policy} onChange={(e) => setPolicy(t.tool_name, e.currentTarget.value)}
 style="padding:3px 6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-size:11px;">
 <option value="auto">auto</option>
 <option value="confirm">confirm</option>
 <option value="mfa">mfa</option>
 <option value="block">block</option>
 </select>
 </div>
)}</For>
 </section>
);
}
