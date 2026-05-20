import { createResource, createSignal, For, Show } from "solid-js";
import { MemoryTab } from "./MemoryTab";
import { Breadcrumb } from "./Breadcrumb";
import { invoke } from "@/api/client";

interface WikiPage { id: string; title: string; page_type: string; updated_at: number; }
interface SearchHit { kind: string; ref_id: string; title: string; body: string; rank: number; }
interface SearchResult { query: string; hits: SearchHit[]; total: number; }
async function fetchWikiPages(): Promise<WikiPage[]> { try { return await invoke<WikiPage[]>("wiki_pages_list"); } catch { return []; } }

// UI-MEMORY-SPEC v1.1 §3~§7 — 🧠 기억 카드 (PRD §0 #2: 기억·학습).
// 좌측: 카테고리·태그·최근·새 페이지·패턴 보드·실수 보드·휴지통
// 중앙: 5 모드 (위키 페이지 / 편집 / 이력 / 그래프 / 검색)
// 우측: 메타·연결·작업
// MVP: 검색 + 위키 리스트 + 보드 placeholder (기존 MemoryTab 재사용).

type Tab = "wiki" | "search" | "pattern" | "mistake" | "trash";

export function MemoryCard(props: { onBack: () => void }) {
  const [tab, setTab] = createSignal<Tab>("wiki");

  return (
    <div class="card-page">
      <Breadcrumb cardName="🧠 기억" onReturn={props.onBack} />
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">🧠</span>
        <h1>기억</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #2 — 기억·학습</div>
      <div class="card-page-oneline">
        Karpathy 위키 + L0~L4 5-layer · 패턴/실수 보드 · 검색 (FTS5 + sqlite-vec hybrid)
      </div>

      <nav style="display:flex; gap:4px; margin-bottom:14px;">
        <button class={"link-btn " + (tab() === "wiki" ? "active" : "")} onClick={() => setTab("wiki")}>📄 위키 페이지</button>
        <button class={"link-btn " + (tab() === "search" ? "active" : "")} onClick={() => setTab("search")}>🔍 검색</button>
        <button class={"link-btn " + (tab() === "pattern" ? "active" : "")} onClick={() => setTab("pattern")}>📈 패턴 보드</button>
        <button class={"link-btn " + (tab() === "mistake" ? "active" : "")} onClick={() => setTab("mistake")}>⚠️ 실수 보드</button>
        <button class={"link-btn " + (tab() === "trash" ? "active" : "")} onClick={() => setTab("trash")}>🗑️ 휴지통</button>
      </nav>

      <Show when={tab() === "wiki"}>
        <WikiSection />
      </Show>

      <Show when={tab() === "search"}>
        <SearchSection />
      </Show>

      <Show when={tab() === "pattern"}>
        <section class="card-section">
          <h3>📈 패턴 보드 — 사양 §6 (M-5)</h3>
          <p class="placeholder-note">
            🤖 AI 발견 패턴 (confidence 점수) + 👤 사용자 추가 패턴 (검증 X — V-5).
            예: "사용자는 오전 9시에 업무 시작", "OpenAgentX 결제는 항상 사용자 승인 받음".
            백엔드 L3 Pattern 테이블 신설 필요.
          </p>
        </section>
      </Show>

      <Show when={tab() === "mistake"}>
        <section class="card-section">
          <h3>⚠️ AI 실수 기록 — 사양 §7 (M-13 V-9)</h3>
          <p class="placeholder-note">
            발견 방식 3가지: 👤 사용자 편집 diff / 🤖 LLM 충돌 감지 / 👤 사용자 명시 등록.
            AI 객관화·자기 성장 추적. 백엔드 Mistake 테이블 + API 신설 필요.
          </p>
        </section>
      </Show>

      <Show when={tab() === "trash"}>
        <section class="card-section">
          <h3>🗑️ 휴지통 — 사양 §9 (M-12 V-4)</h3>
          <p class="placeholder-note">
            30일 후 자동 영구 삭제 (1일 전 알림). 복원 / 지금 영구 삭제 액션.
          </p>
        </section>
      </Show>
    </div>
  );
}

function WikiSection() {
  const [pages, { refetch }] = createResource(fetchWikiPages);
  const [title, setTitle] = createSignal("");
  const [content, setContent] = createSignal("");
  const [ptype, setPtype] = createSignal("concept");
  const [busy, setBusy] = createSignal(false);
  async function save() {
    if (!title()) return;
    setBusy(true);
    try {
      const id = title().toLowerCase().replace(/[^a-z0-9가-힣]+/g, "-").replace(/^-|-$/g, "") || `page-${Date.now()}`;
      await invoke("wiki_page_upsert", { id, title: title(), page_type: ptype(), content: content() });
      setTitle("");
      setContent("");
      await refetch();
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <section class="card-section">
        <h3>📄 위키 페이지 — 사양 §3~§4 (L2, M-1·M-3·M-11)</h3>
        <div style="display:flex; gap:6px; margin-bottom:6px;">
          <input value={title()} onInput={(e) => setTitle(e.currentTarget.value)} placeholder="제목"
            style="flex:1; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
          <select value={ptype()} onChange={(e) => setPtype(e.currentTarget.value)}
            style="padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
            <option value="entity">entity</option><option value="concept">concept</option>
            <option value="comparison">comparison</option><option value="other">other</option>
          </select>
          <button class="link-btn" onClick={save} disabled={busy()}>저장</button>
        </div>
        <textarea value={content()} onInput={(e) => setContent(e.currentTarget.value)}
          placeholder="마크다운 본문 (M-3 — 저장은 항상 마크다운)"
          rows={6}
          style="width:100%; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;"
        />
      </section>
      <section class="card-section">
        <h3>최근 페이지 ({pages()?.length ?? 0})</h3>
        <For each={pages() ?? []}>
          {(p) => (
            <div style="font-size:12px; padding:4px 0; border-bottom:1px solid var(--border);">
              <strong>{p.title}</strong>
              <span style="color:var(--text-3); margin-left:8px;">{p.page_type} · {new Date(p.updated_at * 1000).toLocaleString()}</span>
            </div>
          )}
        </For>
      </section>
      <section class="card-section">
        <h3>기존 MemoryTab (L2 통합 view)</h3>
        <MemoryTab />
      </section>
    </>
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
      const res = await invoke<SearchResult>("global_search", { q: q(), limit: 30 });
      setR(res);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section class="card-section">
      <h3>🔍 검색 — 사양 §11 (V-10 FTS5 + RRF, sqlite-vec 시멘틱은 Phase 2)</h3>
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
