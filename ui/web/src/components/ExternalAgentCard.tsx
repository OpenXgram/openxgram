// UI-EXTERNAL-AGENT-SPEC v1.0 — 외부 에이전트·결제 (PRD §0 #3) — 깊은 구현.

import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";
import { Breadcrumb} from "./Breadcrumb";

async function fetchDirectory(): Promise<any> { try { return await invoke("external_directory");} catch { return null;}}
async function fetchOutbound(): Promise<any[]> { try { return await invoke("external_outbound_calls");} catch { return [];}}
async function fetchInbound(): Promise<any[]> { try { return await invoke("external_inbound_pending");} catch { return [];}}
async function fetchListings(): Promise<any[]> { try { return await invoke("external_my_listings");} catch { return [];}}
async function fetchReputation(): Promise<any[]> { try { return await invoke("external_reputation");} catch { return [];}}
async function fetchProtocols(): Promise<any> { try { return await invoke("external_protocols");} catch { return null;}}

type Tab = "directory" | "outbound" | "inbound" | "listing" | "reputation" | "protocols";

export function ExternalAgentCard(props: { onBack: () => void}) {
 const [tab, setTab] = createSignal<Tab>("directory");
 const [dir] = createResource(fetchDirectory);
 const [outbound] = createResource(fetchOutbound);
 const [inbound, { refetch: refIn}] = createResource(fetchInbound);
 const [listings, { refetch: refL}] = createResource(fetchListings);
 const [rep] = createResource(fetchReputation);
 const [proto, { refetch: refP}] = createResource(fetchProtocols);

 return (
 <div class="card-page">
 <Breadcrumb cardName=" 외부 에이전트" onReturn={props.onBack} />
 <button class="card-page-back" onClick={props.onBack}>← 홈</button>
 <div class="card-page-head">
 <span class="icon"></span>
 <h1>외부 에이전트</h1>
 </div>
 <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #3 — 외부 에이전트·결제</div>
 <div class="card-page-oneline">
 다른 AI 시스템 (OpenAgentX·A2A·ANP·x402·Virtuals ACP) 과의 거래 게이트웨이.
 </div>

 <nav style="display:flex; gap:4px; margin-bottom:14px; flex-wrap:wrap;">
 <button class={"link-btn " + (tab() === "directory" ? "active" : "")} onClick={() => setTab("directory")}> 디렉토리</button>
 <button class={"link-btn " + (tab() === "outbound" ? "active" : "")} onClick={() => setTab("outbound")}> 아웃바운드</button>
 <button class={"link-btn " + (tab() === "inbound" ? "active" : "")} onClick={() => setTab("inbound")}> 인바운드</button>
 <button class={"link-btn " + (tab() === "listing" ? "active" : "")} onClick={() => setTab("listing")}> 내 listing</button>
 <button class={"link-btn " + (tab() === "reputation" ? "active" : "")} onClick={() => setTab("reputation")}>⭐ 평판</button>
 <button class={"link-btn " + (tab() === "protocols" ? "active" : "")} onClick={() => setTab("protocols")}> 프로토콜</button>
 </nav>

 <Show when={tab() === "directory"}>
 <section class="card-section">
 <h3> 외부 디렉토리 (M-2 통합 검색)</h3>
 <Show when={dir()}>
 <div class="card-section-row"><span class="label">활성 프로토콜</span><span class="value">{(dir()?.protocols ?? []).join(" · ") || "—"}</span></div>
 <div class="card-section-row"><span class="label">등록 에이전트</span><span class="value">{(dir()?.external_agents ?? []).length}</span></div>
 <div class="card-section-row"><span class="label">마지막 동기화</span><span class="value">{dir()?.last_sync_at || "미동기화"}</span></div>
 </Show>
 <For each={dir()?.external_agents ?? []}>
 {(a: any) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <strong>{a.name}</strong> · {a.protocol} · ⭐{a.rating || "—"} · {a.price || "?"} USDC
 <div style="color:var(--text-3); font-size:11px;">{a.description?.slice(0, 80)}</div>
 </div>
)}
 </For>
 </section>
 </Show>

 <Show when={tab() === "outbound"}>
 <section class="card-section">
 <h3> 아웃바운드 호출 이력 (M-4)</h3>
 <For each={outbound() ?? []}>
 {(c: any) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <strong>{c.to_agent}</strong> · {c.status} · {c.amount} USDC · {c.completed_at}
 {c.rating && <span> · ⭐{c.rating}</span>}
 </div>
)}
 </For>
 <Show when={(outbound() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">아웃바운드 호출 없음. 디렉토리 → 선택 → 계약 → 결제 → 작업.</p>
 </Show>
 </section>
 </Show>

 <Show when={tab() === "inbound"}>
 <section class="card-section">
 <h3> 인바운드 승인 큐 (M-5)<button class="link-btn" onClick={() => refIn()}>↻</button></h3>
 <For each={inbound() ?? []}>
 {(c: any) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <strong>{c.from_agent}</strong> · {c.request_summary} · {c.offered_price} USDC
 <button class="link-btn" style="margin-left:6px;" onClick={async () => { try { await invoke("external_inbound_approve", { id: c.id}); refIn();} catch (e) { alert(String(e));}}}> 승인</button>
 <button class="link-btn" onClick={async () => { try { await invoke("external_inbound_reject", { id: c.id}); refIn();} catch (e) { alert(String(e));}}}> 거절</button>
 </div>
)}
 </For>
 <Show when={(inbound() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">대기 중 인바운드 없음.</p>
 </Show>
 </section>
 </Show>

 <Show when={tab() === "listing"}>
 <ListingSection listings={listings()} refetch={refL} />
 </Show>

 <Show when={tab() === "reputation"}>
 <section class="card-section">
 <h3>⭐ 외부 에이전트 평판 (M-11 M-13)</h3>
 <For each={rep() ?? []}>
 {(r: any) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <strong>{r.external_agent}</strong> · ⭐{r.avg_rating} ({r.review_count} 리뷰) {r.blacklisted ? "" : ""}
 </div>
)}
 </For>
 <Show when={(rep() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">평판 데이터 없음 (M-12: 최소 10회 거래 후 신뢰 점수).</p>
 </Show>
 </section>
 </Show>

 <Show when={tab() === "protocols"}>
 <section class="card-section">
 <h3> 프로토콜 설정 (M-1)<button class="link-btn" onClick={() => refP()}>↻</button></h3>
 <For each={proto()?.protocols ?? ["OpenAgentX", "x402", "A2A", "ANP", "Virtuals ACP"]}>
 {(p: string) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border); display:flex; justify-content:space-between;">
 <span>{p}</span>
 <span>{proto()?.[p.toLowerCase().replace(/ /g, "_")] ? " 활성" : "○ 비활성"}</span>
 </div>
)}
 </For>
 </section>
 </Show>
 </div>
);
}

function ListingSection(props: { listings: any[] | undefined; refetch: () => void}) {
 const [agentId, setAgentId] = createSignal("");
 const [price, setPrice] = createSignal(1.0);
 const [marketplace, setMarketplace] = createSignal("OpenAgentX");
 async function add() {
 if (!agentId()) return;
 try { await invoke("external_listing_add", { agent_id: agentId(), marketplace: marketplace(), price_usdc: price()}); setAgentId(""); props.refetch();} catch (e) { alert(String(e));}
}
 return (
 <section class="card-section">
 <h3> 내 마켓 listing (M-6 M-7 M-8 M-9) — Cognac 수익원</h3>
 <div style="display:flex; gap:4px; margin-bottom:6px; flex-wrap:wrap;">
 <input value={agentId()} onInput={(e) => setAgentId(e.currentTarget.value)} placeholder="agent_id" style="flex:2; padding:4px;" />
 <select value={marketplace()} onChange={(e) => setMarketplace(e.currentTarget.value)} style="padding:4px;">
 <option>OpenAgentX</option>
 <option>x402</option>
 <option>A2A</option>
 </select>
 <input type="number" step="0.1" value={price()} onInput={(e) => setPrice(parseFloat(e.currentTarget.value) || 1)} style="width:80px; padding:4px;" />
 <button class="link-btn" onClick={add}>+ 등록</button>
 </div>
 <For each={props.listings ?? []}>
 {(l: any) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <strong>{l.agent_id}</strong> @ {l.marketplace} · {l.price_usdc} USDC ({l.pricing_model || "per-call"})
 <div style="color:var(--text-3); font-size:11px;">{l.description?.slice(0, 80)}</div>
 </div>
)}
 </For>
 <Show when={(props.listings ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">등록된 listing 없음. 위에서 agent_id + 가격 입력 + 등록.</p>
 </Show>
 </section>
);
}
