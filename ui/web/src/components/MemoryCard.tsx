import { createResource, createSignal, For, Show} from "solid-js";
import { MemoryTab} from "./MemoryTab";
import { Breadcrumb} from "./Breadcrumb";
import { invoke} from "@/api/client";

interface WikiPage { id: string; title: string; page_type: string; updated_at: number;}
interface SearchHit { kind: string; ref_id: string; title: string; body: string; rank: number;}
interface SearchResult { query: string; hits: SearchHit[]; total: number;}
async function fetchWikiPages(): Promise<WikiPage[]> { try { return await invoke<WikiPage[]>("wiki_pages_list");} catch { return [];}}

// UI-MEMORY-SPEC v1.1 §3~§7 — 기억 카드 (PRD §0 #2: 기억·학습).
// 좌측: 카테고리·태그·최근·새 페이지·패턴 보드·실수 보드·휴지통
// 중앙: 5 모드 (위키 페이지 / 편집 / 이력 / 그래프 / 검색)
// 우측: 메타·연결·작업
// MVP: 검색 + 위키 리스트 + 보드 placeholder (기존 MemoryTab 재사용).

type Tab = "wiki" | "l0" | "search" | "pattern" | "mistake" | "trash" | "io";

function WebhookTokenSection() {
 const [data, { refetch}] = createResource<any>(async () => { try { return await invoke("memory_webhook_token");} catch { return null;}});
 async function rotate() {
 if (data()?.exists && !confirm("새 token 발급 시 이전 URL 무효화. 진행?")) return;
 try { await invoke("memory_webhook_rotate"); await refetch();} catch (e) { alert("실패: " + e);}
 }
 return (
 <section class="card-section">
 <h3>Webhook URL — LLM 직접 push (Bearer 없이)</h3>
 <p style="font-size:12px; color:var(--text-3);">
 외부 LLM (Claude Desktop, Cursor, ChatGPT 등) 이 OpenXgram 메모리에 직접 POST 할 수 있는 URL. URL 자체가 secret 이므로 노출 주의.
 </p>
 <Show when={data()?.exists} fallback={
 <button class="link-btn" onClick={rotate}>+ Webhook URL 발급</button>
 }>
 <div style="background:var(--surface-2); padding:10px; border-radius:4px; margin:6px 0;">
 <div style="font-family:monospace; font-size:11px; word-break:break-all;">{data()?.webhook_url}</div>
 </div>
 <button class="link-btn" onClick={() => { navigator.clipboard.writeText(data()?.webhook_url ?? ""); alert("URL 복사됨");}}>URL 복사</button>
 <button class="link-btn" onClick={rotate} style="margin-left:6px;">새로 발급 (rotate)</button>
 <pre style="margin-top:8px; padding:8px; background:var(--surface-2); border-radius:4px; font-size:11px;">{`# 사용 예 (curl):
curl -X POST "${data()?.webhook_url}" \\
  -H "Content-Type: application/json" \\
  -d '{
    "openxgram_import_version": 1,
    "source_app": "ChatGPT",
    "session_title": "예시 대화",
    "items": [
      {"type":"message","sender":"user","body":"안녕","timestamp":"2026-05-22T10:00:00Z"},
      {"type":"wiki_fact","page_id":"test-fact","title":"테스트","page_type":"concept","content":"# 테스트\\n본문"}
    ]
  }'`}</pre>
 </Show>
 </section>
 );
}

function ImportExportSection() {
 const [scan, { refetch}] = createResource<any>(async () => { try { return await invoke("memory_import_scan_paths");} catch { return null;}});
 const [prompt] = createResource<any>(async () => { try { return await invoke("memory_import_prompt");} catch { return null;}});
 const [importBundle, setImportBundle] = createSignal("");
 const [importResult, setImportResult] = createSignal<string | null>(null);
 async function doDesktopImport(path: string) {
 if (!confirm(`데스크탑 앱 conversation 폴더에서 import 진행?\n경로: ${path}`)) return;
 try {
 const r = await invoke<any>("memory_import_desktop", { path});
 alert(`import 완료: ${r.messages_imported} 메시지`);
 refetch();
 } catch (e) { alert("import 실패: " + e);}
 }
 async function doMigrationImport() {
 if (!importBundle().trim()) return;
 try {
 const bundle = JSON.parse(importBundle());
 const r = await invoke<any>("memory_migration_import", { bundle});
 setImportResult(JSON.stringify(r, null, 2));
 } catch (e) { setImportResult("실패: " + e);}
 }
 return (
 <>
 <section class="card-section">
 <h3>가져오기 (Import) — 데스크탑 앱 conversation</h3>
 <p style="font-size:12px; color:var(--text-3);">
 Claude Desktop / Cursor / ChatGPT 등 외부 LLM 앱에서 OpenXgram 메모리로 import.
 자동 탐지된 경로에서 직접 import 또는 export 한 .json bundle 붙여넣기.
 </p>
 <button class="link-btn" onClick={() => refetch()} style="margin-bottom:8px;">↻ 경로 재스캔</button>
 <Show when={scan()?.candidates}>
 <table style="width:100%; font-size:12px;">
 <thead><tr style="border-bottom:1px solid var(--border);"><th>앱</th><th>경로</th><th>존재</th><th>파일</th><th></th></tr></thead>
 <tbody>
 <For each={scan()?.candidates}>{(c: any) => (
 <tr style="border-bottom:1px dashed var(--border);">
 <td style="padding:4px;"><strong>{c.name}</strong></td>
 <td style="padding:4px; font-family:monospace; font-size:11px; word-break:break-all;">{c.path}</td>
 <td style="padding:4px; color:{c.exists ? '#4caf50' : '#666'};">{c.exists ? "✓" : "—"}</td>
 <td style="padding:4px;">{c.file_count}</td>
 <td style="padding:4px;">
 <Show when={c.exists && c.file_count > 0}>
 <button class="link-btn" onClick={() => doDesktopImport(c.path)}>import</button>
 </Show>
 </td>
 </tr>
 )}</For>
 </tbody>
 </table>
 </Show>
 </section>

 <section class="card-section">
 <h3>가져오기 (Import) — JSON bundle 붙여넣기</h3>
 <textarea
 value={importBundle()}
 onInput={(e) => setImportBundle(e.currentTarget.value)}
 placeholder='OpenXgram migration bundle JSON 붙여넣기 (export 받은 파일 내용)&#10;{"session": {...}, "messages": [...]}'
 rows={6}
 style="width:100%; padding:8px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-family:monospace; font-size:12px;"
 />
 <button class="link-btn" onClick={doMigrationImport} style="margin-top:6px;">import 실행</button>
 <Show when={importResult()}>
 <pre style="margin-top:8px; padding:8px; background:var(--surface-2); border-radius:4px; font-size:11px;">{importResult()}</pre>
 </Show>
 </section>

 <WebhookTokenSection />

 <section class="card-section">
 <h3>가져오기 프롬프트 (LLM 통해 다른 데이터 변환)</h3>
 <p style="font-size:12px; color:var(--text-3);">아래 프롬프트를 외부 LLM (ChatGPT, Claude) 에 던지고, 가진 대화 로그를 OpenXgram 형식으로 변환 받으세요.</p>
 <Show when={prompt()?.prompt}>
 <pre style="background:var(--surface-2); padding:10px; border-radius:4px; font-size:11px; max-height:300px; overflow:auto; white-space:pre-wrap;">{prompt()?.prompt}</pre>
 <button class="link-btn" onClick={() => navigator.clipboard.writeText(prompt()?.prompt ?? "")}>프롬프트 복사</button>
 </Show>
 </section>

 <section class="card-section">
 <h3>내보내기 (Export) — 위치</h3>
 <p style="font-size:12px; color:var(--text-3);">
 <strong>세션 단위 export</strong>: 메신저 사이드패널 → 세션 선택 → 우측 패널 "개요" 탭 → "export" 버튼<br />
 <strong>위키 페이지 단위 export</strong>: 메모리 카드 → 위키 페이지 목록 → 페이지 클릭 → 우상단 "export .md" 버튼<br />
 <strong>전체 백업</strong>: 운영·생존 카드 → 백업·복원 → "지금 백업" (tar.gz)
 </p>
 </section>
 </>
 );
}

function FiveLayerStats() {
 const [stats] = createResource(async () => {
 try { return await invoke<any>("memory_stats");} catch { return null;}
 });
 const cards: Array<[string, string, string]> = [
 ["L0", "L0_raw_messages", "raw 메시지"],
 ["L1", "L1_episodes", "episodes"],
 ["L2", "L2_wiki_pages", "위키 페이지"],
 ["L3", "L3_patterns", "패턴"],
 ["L4", "L4_traits", "특성"],
 ];
 return (
 <section style="display:grid; grid-template-columns:repeat(auto-fit, minmax(140px, 1fr)); gap:8px; margin:8px 0 14px;">
 <For each={cards}>{([lvl, key, label]) => {
 const layer = () => stats()?.layers?.[key];
 return (
 <div style="background:var(--surface-2); border:1px solid var(--border); border-radius:6px; padding:10px;">
 <div style="font-size:11px; color:var(--text-3);">{lvl} · {label}</div>
 <div style="font-size:22px; font-weight:bold; margin-top:4px;">{layer()?.count ?? "—"}</div>
 <Show when={layer()?.last_at}>
 <div style="font-size:10px; color:var(--text-3);">최근 {String(layer()?.last_at).slice(0,16)}</div>
 </Show>
 </div>
 );
 }}</For>
 </section>
 );
}

function L0Section() {
 const [q, setQ] = createSignal("");
 const [items, { refetch}] = createResource<any[]>(async () => {
 try {
 const path = q().trim() ? "memory_l0_list" : "memory_l0_list";
 return await invoke<any[]>(path, q().trim() ? { q: q().trim(), limit: 100} : { limit: 100});
 } catch { return [];}
 });
 return (
 <section class="card-section">
 <h3>L0 raw 메시지 — UI-MEMORY-SPEC §2.3</h3>
 <div style="display:flex; gap:6px; margin-bottom:10px;">
 <input
 placeholder="검색 (body LIKE)"
 value={q()}
 onInput={(e) => setQ(e.currentTarget.value)}
 onKeyDown={(e) => { if (e.key === "Enter") refetch();}}
 style="flex:1; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;"
 />
 <button class="link-btn" onClick={() => refetch()}>검색</button>
 </div>
 <Show when={(items() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">L0 메시지 없음.</p>
 </Show>
 <For each={items() ?? []}>{(m: any) => (
 <div style="padding:8px; border-bottom:1px solid var(--border); font-size:12px;">
 <div style="display:flex; justify-content:space-between; font-size:11px; color:var(--text-3);">
 <span><strong>{m.sender}</strong> · session: {m.session_id?.slice(0,30)}</span>
 <span>{String(m.timestamp).slice(0,19)}</span>
 </div>
 <div style="margin-top:4px; white-space:pre-wrap; word-break:break-word;">{m.body?.slice(0, 400)}{m.body?.length > 400 ? "…" : ""}</div>
 </div>
 )}</For>
 </section>
 );
}

export function MemoryCard(props: { onBack: () => void}) {
 const [tab, setTab] = createSignal<Tab>("wiki");

 return (
 <div class="card-page">
 <Breadcrumb cardName=" 기억" onReturn={props.onBack} />
 <div class="card-page-head">
 <span class="icon"></span>
 <h1>기억</h1>
 </div>
 <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #2 — 기억·학습</div>
 <div class="card-page-oneline">
 Karpathy 위키 + L0~L4 5-layer · 패턴/실수 보드 · 검색 (FTS5 + sqlite-vec hybrid)
 </div>

 <FiveLayerStats />

 <nav style="display:flex; gap:4px; margin-bottom:14px; flex-wrap:wrap;">
 <button class={"link-btn " + (tab() === "wiki" ? "active" : "")} onClick={() => setTab("wiki")}>L2 위키 페이지</button>
 <button class={"link-btn " + (tab() === "l0" ? "active" : "")} onClick={() => setTab("l0" as Tab)}>L0 raw 메시지</button>
 <button class={"link-btn " + (tab() === "search" ? "active" : "")} onClick={() => setTab("search")}>검색</button>
 <button class={"link-btn " + (tab() === "pattern" ? "active" : "")} onClick={() => setTab("pattern")}>L3 패턴 보드</button>
 <button class={"link-btn " + (tab() === "mistake" ? "active" : "")} onClick={() => setTab("mistake")}>실수 보드</button>
 <button class={"link-btn " + (tab() === "trash" ? "active" : "")} onClick={() => setTab("trash")}>휴지통</button>
 <button class={"link-btn " + (tab() === "io" ? "active" : "")} onClick={() => setTab("io" as Tab)}>가져오기·내보내기</button>
 </nav>
 <Show when={tab() === "io"}>
 <ImportExportSection />
 </Show>
 <Show when={tab() === "l0"}>
 <L0Section />
 </Show>

 <Show when={tab() === "wiki"}>
 <WikiSection />
 </Show>

 <Show when={tab() === "search"}>
 <SearchSection />
 </Show>

 <Show when={tab() === "pattern"}>
 <PatternSection />
 </Show>

 <Show when={tab() === "mistake"}>
 <MistakeSection />
 </Show>

 <Show when={tab() === "trash"}>
 <TrashSection />
 </Show>
 </div>
);
}

function renderMarkdown(md: string): string {
 // 단순 markdown — heading / bold / italic / code / link / list
 return md
 .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
 .replace(/^### (.+)$/gm, "<h3>$1</h3>")
 .replace(/^## (.+)$/gm, "<h2>$1</h2>")
 .replace(/^# (.+)$/gm, "<h1>$1</h1>")
 .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
 .replace(/\*(.+?)\*/g, "<em>$1</em>")
 .replace(/`(.+?)`/g, "<code>$1</code>")
 .replace(/^- (.+)$/gm, "<li>$1</li>")
 .replace(/\n\n/g, "<br/><br/>")
 .replace(/\[(.+?)\]\((.+?)\)/g, '<a href="$2" target="_blank">$1</a>');
}

function WikiSection() {
 // 위키 layout — 좌측 카테고리·검색·페이지 트리, 중앙 본문, 우측 메타·작업.
 const [pages, { refetch}] = createResource(fetchWikiPages);
 const [filter, setFilter] = createSignal("");
 const [filterType, setFilterType] = createSignal<string>("all");
 const [selectedId, setSelectedId] = createSignal<string | null>(null);
 const [viewing, setViewing] = createSignal<any | null>(null);
 const [viewMode, setViewMode] = createSignal<"render" | "raw" | "edit">("render");
 const [editBody, setEditBody] = createSignal("");
 const [editTitle, setEditTitle] = createSignal("");
 const [showNewForm, setShowNewForm] = createSignal(false);
 const [newTitle, setNewTitle] = createSignal("");
 const [newType, setNewType] = createSignal("concept");
 const [busy, setBusy] = createSignal(false);

 async function open(id: string) {
 setSelectedId(id);
 try {
 const p = await invoke<any>("wiki_page_get", { id});
 setViewing(p);
 setEditBody(p?.body ?? p?.content ?? "");
 setEditTitle(p?.title ?? "");
 setViewMode("render");
 } catch (e) { alert("로드 실패: " + e);}
 }
 async function saveEdit() {
 if (!viewing()) return;
 setBusy(true);
 try {
 await invoke("wiki_page_upsert", {
 id: viewing().id, title: editTitle(), page_type: viewing().page_type, content: editBody()
 });
 await refetch();
 await open(viewing().id);
 setViewMode("render");
 } finally { setBusy(false);}
 }
 async function createNew() {
 if (!newTitle().trim()) return;
 setBusy(true);
 try {
 const id = newTitle().toLowerCase().replace(/[^a-z0-9가-힣]+/g, "-").replace(/^-|-$/g, "") || `page-${Date.now()}`;
 await invoke("wiki_page_upsert", { id, title: newTitle(), page_type: newType(), content: `# ${newTitle()}\n\n`});
 setNewTitle("");
 setShowNewForm(false);
 await refetch();
 await open(id);
 } finally { setBusy(false);}
 }

 // 카테고리 = page_type 별 grouping + 검색 필터
 const filteredPages = () => {
 const all = pages() ?? [];
 const q = filter().trim().toLowerCase();
 return all.filter(p =>
 (filterType() === "all" || p.page_type === filterType()) &&
 (!q || p.title.toLowerCase().includes(q) || p.id.toLowerCase().includes(q))
 );
 };
 const grouped = () => {
 const m = new Map<string, any[]>();
 for (const p of filteredPages()) {
 const k = p.page_type || "other";
 if (!m.has(k)) m.set(k, []);
 m.get(k)!.push(p);
 }
 return Array.from(m.entries()).sort((a,b) => a[0].localeCompare(b[0]));
 };

 return (
 <section class="card-section" style="padding:0;">
 <div style="display:grid; grid-template-columns:260px 1fr 240px; gap:0; min-height:600px; border:1px solid var(--border); border-radius:6px; overflow:hidden;">
 {/* 좌측 — 검색 + 카테고리 트리 */}
 <aside style="background:var(--surface-2); padding:10px; border-right:1px solid var(--border); overflow-y:auto;">
 <input
 placeholder="🔍 위키 검색"
 value={filter()}
 onInput={(e) => setFilter(e.currentTarget.value)}
 style="width:100%; padding:6px; background:var(--surface-1); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-size:12px; box-sizing:border-box;"
 />
 <div style="display:flex; gap:4px; margin-top:6px; flex-wrap:wrap;">
 <For each={["all", "concept", "entity", "comparison", "other"]}>{(t) => (
 <button class={"link-btn " + (filterType() === t ? "active" : "")}
 onClick={() => setFilterType(t)}
 style="font-size:10px; padding:3px 6px;">{t}</button>
 )}</For>
 </div>
 <button class="link-btn" onClick={() => setShowNewForm(!showNewForm())}
 style="margin:10px 0 4px; font-size:12px; width:100%;">
 + 새 페이지
 </button>
 <Show when={showNewForm()}>
 <div style="background:var(--surface-1); padding:6px; border-radius:4px;">
 <input value={newTitle()} onInput={(e) => setNewTitle(e.currentTarget.value)}
 placeholder="제목" style="width:100%; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:3px; font-size:11px; box-sizing:border-box;" />
 <select value={newType()} onChange={(e) => setNewType(e.currentTarget.value)}
 style="width:100%; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:3px; font-size:11px; margin-top:4px; box-sizing:border-box;">
 <option value="concept">concept</option>
 <option value="entity">entity</option>
 <option value="comparison">comparison</option>
 <option value="other">other</option>
 </select>
 <button class="link-btn" onClick={createNew} disabled={busy()}
 style="width:100%; margin-top:4px; font-size:11px;">생성</button>
 </div>
 </Show>
 <hr style="margin:10px 0; border-color:var(--border); opacity:0.3;" />
 <For each={grouped()}>{([type, list]) => (
 <div style="margin-bottom:10px;">
 <div style="font-size:10px; color:var(--text-3); padding:3px 0; text-transform:uppercase; letter-spacing:0.5px;">
 {type} ({list.length})
 </div>
 <For each={list}>{(p) => (
 <div onClick={() => open(p.id)}
 style={`padding:4px 6px; font-size:12px; cursor:pointer; border-radius:3px; ${selectedId() === p.id ? "background:#06c; color:white;" : ""}`}>
 {p.title}
 </div>
 )}</For>
 </div>
 )}</For>
 <Show when={(pages() ?? []).length === 0}>
 <p style="font-size:11px; color:var(--text-3); padding:8px 0;">위키 페이지 없음. 새 페이지 버튼으로 생성.</p>
 </Show>
 </aside>

 {/* 중앙 — 본문 */}
 <main style="padding:16px 20px; overflow-y:auto; min-width:0;">
 <Show when={viewing()} fallback={
 <div style="display:flex; flex-direction:column; align-items:center; justify-content:center; height:100%; color:var(--text-3);">
 <p style="font-size:14px;">좌측에서 위키 페이지 선택</p>
 <p style="font-size:11px;">또는 + 새 페이지 클릭</p>
 </div>
 }>
 <div style="display:flex; justify-content:space-between; align-items:flex-start; margin-bottom:6px; gap:8px; flex-wrap:wrap;">
 <div style="min-width:0; flex:1;">
 <Show when={viewMode() === "edit"} fallback={<h2 style="margin:0;">{viewing()?.title}</h2>}>
 <input value={editTitle()} onInput={(e) => setEditTitle(e.currentTarget.value)}
 style="width:100%; padding:6px; font-size:18px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; box-sizing:border-box;" />
 </Show>
 </div>
 <div style="display:flex; gap:4px; flex-wrap:wrap;">
 <button class={"link-btn " + (viewMode() === "render" ? "active" : "")}
 onClick={() => setViewMode("render")}>렌더</button>
 <button class={"link-btn " + (viewMode() === "raw" ? "active" : "")}
 onClick={() => setViewMode("raw")}>raw</button>
 <button class={"link-btn " + (viewMode() === "edit" ? "active" : "")}
 onClick={() => { setEditBody(viewing()?.body ?? viewing()?.content ?? ""); setEditTitle(viewing()?.title ?? ""); setViewMode("edit");}}>편집</button>
 <Show when={viewMode() === "edit"}>
 <button class="link-btn" onClick={saveEdit} disabled={busy()}
 style="background:#06c; color:white;">{busy() ? "저장 중…" : "저장"}</button>
 </Show>
 </div>
 </div>
 <Show when={viewMode() === "render"}>
 <article style="line-height:1.7; font-size:14px;" innerHTML={renderMarkdown(viewing()?.body ?? viewing()?.content ?? "(빈 페이지)")} />
 </Show>
 <Show when={viewMode() === "raw"}>
 <pre style="background:var(--surface-2); padding:14px; border-radius:4px; white-space:pre-wrap; word-break:break-word; font-size:12px; line-height:1.6;">{viewing()?.body ?? viewing()?.content ?? ""}</pre>
 </Show>
 <Show when={viewMode() === "edit"}>
 <textarea value={editBody()} onInput={(e) => setEditBody(e.currentTarget.value)}
 rows={25}
 style="width:100%; padding:10px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; font-family:monospace; font-size:13px; line-height:1.6; box-sizing:border-box;" />
 </Show>
 </Show>
 </main>

 {/* 우측 — 메타 + 작업 */}
 <aside style="background:var(--surface-2); padding:12px; border-left:1px solid var(--border); overflow-y:auto; font-size:11px;">
 <Show when={viewing()} fallback={
 <p style="color:var(--text-3);">페이지 선택 시 메타 정보·작업 노출</p>
 }>
 <strong style="font-size:12px;">메타</strong>
 <div style="margin-top:6px;"><span style="color:var(--text-3);">id:</span> <code style="font-size:10px;">{viewing()?.id}</code></div>
 <div><span style="color:var(--text-3);">type:</span> {viewing()?.page_type}</div>
 <div><span style="color:var(--text-3);">updated:</span> {new Date((viewing()?.updated_at ?? 0) * 1000).toLocaleString()}</div>
 <hr style="margin:10px 0; border-color:var(--border); opacity:0.3;" />
 <strong style="font-size:12px;">작업</strong>
 <div style="display:flex; flex-direction:column; gap:4px; margin-top:6px;">
 <a class="link-btn" href={`/v1/gui/memory/export/wiki/${viewing()?.id}`}
 download={`${viewing()?.title || 'page'}.md`}
 style="text-decoration:none; text-align:center;">📥 export .md</a>
 <button class="link-btn" onClick={async () => {
 try { const h = await invoke<any[]>("wiki_history", { id: viewing()?.id}); alert(`이력 ${h.length}건:\n${h.slice(0,10).map(e=>e.event_type+' @ '+e.at).join('\n')}`);} catch (e) { alert(String(e));}
 }}>📜 변경 이력</button>
 <button class="link-btn" onClick={async () => {
 try { await invoke("wiki_lock", { id: viewing()?.id, locked_by: "user", reason: "사용자 표시"}); alert("🔒 잠금 완료");} catch (e) { alert(String(e));}
 }}>🔒 잠금</button>
 <button class="link-btn" onClick={async () => {
 try { const s = await invoke<any>("wiki_share", { id: viewing()?.id, mode: "secret", noindex: true}); alert(`🔗 공유 URL:\n${s.url}\n(noindex=${s.noindex})`);} catch (e) { alert(String(e));}
 }}>🔗 공유</button>
 <button class="link-btn" onClick={async () => {
 if (!confirm(`"${viewing()?.title}" 휴지통으로?`)) return;
 try { await invoke("wiki_delete", { id: viewing()?.id}); setViewing(null); setSelectedId(null); await refetch();} catch (e) { alert(String(e));}
 }} style="color:#f88;">🗑 휴지통</button>
 </div>
 <hr style="margin:10px 0; border-color:var(--border); opacity:0.3;" />
 <strong style="font-size:12px;">관련</strong>
 <p style="color:var(--text-3); margin-top:4px;">백업 안내·새 페이지 알림 등은 좌측 사이드바 하단 또는 운영 카드 → 백업 메뉴.</p>
 </Show>
 </aside>
 </div>
 <details style="margin-top:14px;">
 <summary style="cursor:pointer; color:var(--text-3); font-size:12px;">고급 (백업·알림·기존 통합 view)</summary>
 <div style="padding:10px;">
 <h4 style="font-size:12px;">새 페이지 알림 (M-6)</h4>
 <NewAlertsView />
 <h4 style="font-size:12px; margin-top:14px;">기존 MemoryTab</h4>
 <MemoryTab />
 </div>
 </details>
 </section>
 );
}

/* 옛 WikiSection 의 orphan JSX 제거됨 — 새 WikiSection 이 위에 정의됨 */
function _NoopRef_unused() { return null;}
/* prettier-ignore */

function NewAlertsView() {
 const [list] = createResource<any[]>(async () => { try { return await invoke<any[]>("wiki_new_alerts");} catch { return [];}});
 return (
 <>
 <Show when={(list() ?? []).length === 0}>
 <div style="font-size:12px; color:var(--text-3);">새 페이지 알림 없음.</div>
 </Show>
 <For each={list() ?? []}>{(a) => (
 <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);">
 <strong>{a.title}</strong>
 <span style="color:var(--text-3); margin-left:6px;">{a.created_at}</span>
 </div>
)}</For>
 </>
);
}

function PatternSection() {
 const [list, { refetch}] = createResource<any[]>(async () => { try { return await invoke<any[]>("memory_patterns_list");} catch { return [];}});
 const [desc, setDesc] = createSignal("");
 const [type, setType] = createSignal("behavior");
 async function add() {
 if (!desc()) return;
 try { await invoke("memory_pattern_add", { pattern_type: type(), description: desc(), source: "user", confidence: 1.0}); setDesc(""); await refetch();} catch {}
}
 return (
 <section class="card-section">
 <h3> 패턴 보드 — 사양 §6 (M-5 V-5)</h3>
 <div style="display:flex; gap:4px; margin-bottom:6px;">
 <select value={type()} onChange={(e) => setType(e.currentTarget.value)}
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="behavior">behavior</option>
 <option value="utterance">utterance</option>
 <option value="preference">preference</option>
 </select>
 <input value={desc()} onInput={(e) => setDesc(e.currentTarget.value)} placeholder="예: 사용자는 오전 9시에 업무 시작"
 style="flex:1; padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <button class="link-btn" onClick={add}>+ 추가</button>
 </div>
 <For each={list() ?? []}>{(p: any) => (
 <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);">
 <strong>[{p.pattern_type}]</strong> {p.description}
 <span style="color:var(--text-3); margin-left:6px;">{p.source} · conf {p.confidence.toFixed(2)}</span>
 </div>
)}</For>
 </section>
);
}

function MistakeSection() {
 const [list, { refetch}] = createResource<any[]>(async () => { try { return await invoke<any[]>("memory_mistakes_list");} catch { return [];}});
 const [title, setTitle] = createSignal("");
 const [body, setBody] = createSignal("");
 const [method, setMethod] = createSignal("user_explicit");
 async function add() {
 if (!title()) return;
 try { await invoke("memory_mistake_add", { title: title(), description: body(), discovery_method: method()}); setTitle(""); setBody(""); await refetch();} catch {}
}
 return (
 <section class="card-section">
 <h3> AI 실수 기록 — 사양 §7 (M-13 V-9)</h3>
 <div style="display:flex; flex-direction:column; gap:4px; margin-bottom:6px;">
 <input value={title()} onInput={(e) => setTitle(e.currentTarget.value)} placeholder="실수 제목"
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <textarea value={body()} onInput={(e) => setBody(e.currentTarget.value)} placeholder="설명" rows={2}
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <div style="display:flex; gap:4px;">
 <select value={method()} onChange={(e) => setMethod(e.currentTarget.value)}
 style="padding:4px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 <option value="user_edit_diff">user_edit_diff</option>
 <option value="llm_conflict">llm_conflict</option>
 <option value="user_explicit">user_explicit</option>
 </select>
 <button class="link-btn" onClick={add}>+ 추가</button>
 </div>
 </div>
 <For each={list() ?? []}>{(m: any) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <strong>{m.title}</strong> {m.resolved ? "" : ""}
 <div style="color:var(--text-3); font-size:11px;">[{m.discovery_method}] {m.created_at}</div>
 <div>{m.description}</div>
 </div>
)}</For>
 </section>
);
}

function TrashSection() {
 const [list, { refetch}] = createResource<any[]>(async () => { try { return await invoke<any[]>("wiki_trash_list");} catch { return [];}});
 async function restore(id: string) {
 try { await invoke("wiki_trash_restore", { id}); await refetch();} catch {}
}
 return (
 <section class="card-section">
 <h3> 휴지통 — 사양 §9 (M-12 V-4 — 30일 후 자동 영구 삭제)</h3>
 <Show when={(list() ?? []).length === 0}>
 <div style="font-size:12px; color:var(--text-3);">휴지통 비어 있음.</div>
 </Show>
 <For each={list() ?? []}>{(t: any) => (
 <div style="display:flex; justify-content:space-between; font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <div>
 <strong>{t.title}</strong>
 <div style="color:var(--text-3); font-size:11px;">{t.page_type} · 삭제 {t.deleted_at} · 영구 삭제 {t.purge_at}</div>
 </div>
 <button class="link-btn" onClick={() => restore(t.id)}>↩ 복원</button>
 </div>
)}</For>
 </section>
);
}

function SearchSection() {
 const [q, setQ] = createSignal("");
 const [r, setR] = createSignal<SearchResult | null>(null);
 const [busy, setBusy] = createSignal(false);
 async function run() {
 if (!q().trim()) return;
 setBusy(true);
 try {
 const res = await invoke<SearchResult>("global_search", { q: q(), limit: 30});
 setR(res);
} finally {
 setBusy(false);
}
}
 return (
 <section class="card-section">
 <h3> 검색 — 사양 §11 (V-10 FTS5 + RRF, sqlite-vec 시멘틱은 Phase 2)</h3>
 <div style="display:flex; gap:6px; margin-bottom:8px;">
 <input value={q()} onInput={(e) => setQ(e.currentTarget.value)}
 onKeyDown={(e) => e.key === "Enter" && run()}
 placeholder="L0~L4 통합 검색 (메시지·위키·패턴·실수)"
 style="flex:1; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <button class="link-btn" onClick={run} disabled={busy()}>검색</button>
 </div>
 <Show when={r()}>
 <p style="font-size:11px; color:var(--text-3);">{r()!.total} 건</p>
 <For each={r()!.hits}>
 {(h) => (
 <div style="font-size:12px; padding:6px 0; border-bottom:1px solid var(--border);">
 <div style="color:var(--text-3); font-size:10px;">[{h.kind}] {h.ref_id} · rank {h.rank.toFixed(2)}</div>
 <strong>{h.title || "(제목 없음)"}</strong>
 <div>{h.body.slice(0, 200)}</div>
 </div>
)}
 </For>
 </Show>
 </section>
);
}
