import { createSignal, createResource, createMemo, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./wiki-extra.css";

// 위키 탭 — 카카오톡 정본 목업(_mockups/kakao-mockup.html) 충실 이식.
// 정본 #wikiOvl 의 .board / .bh / .bb / .wsearch / .wsec / .witem / .wt2 / .wm / .wkind 마크업·CSS 를
// 그대로(verbatim) 포팅하고, 샘플 텍스트만 라이브 데이터로 치환. 오버레이 chrome(.ovl/.bx) 은 탭 본문이라 제거.
// 백엔드 contract 재사용(신규 명령 발명 X):
//   wiki_pages_list      → WikiPage[]  (기록·기억)
//   memory_patterns_list → Pattern[]   (자기개선 · 학습된 패턴)
//   memory_mistakes_list → Mistake[]   (자기개선 · 실수→규칙)
// 의미검색 명령이 따로 없으므로 .wsearch 는 로드된 목록에 대한 클라이언트 필터.

interface WikiPage {
  id: string;
  title: string;
  page_type: string;
  updated_at?: number;
}

// wiki_body_get / wiki_body_put 응답 DTO (daemon_gui WikiBodyDto / put 결과).
interface WikiBody {
  slug: string;
  title: string;
  body: string;
  updated_at?: number;
}

interface WikiPutResult {
  ok: boolean;
  slug: string;
  content_hash: string;
}

// daemon 라우트는 `/v1/gui/wiki/{ptype}/{slug}` 이고 PageId::new(ptype, slug) 로
// `{ptype}/{slug}` 재구성한다. wiki_pages_list 의 id 는 정규화된 PageId(`entity/foo`) 이므로
// page_type 을 prefix 로 떼어 순수 slug 만 추출한다. (slug 자체엔 `/` 가 없음 — PageId::new 검증.)
function resolveSlug(page: WikiPage): { ptype: string; slug: string } {
  const ptype = page.page_type || "";
  const id = page.id || "";
  if (ptype && id.startsWith(`${ptype}/`)) {
    return { ptype, slug: id.slice(ptype.length + 1) };
  }
  // fallback — id 가 `{type}/{slug}` 형태면 첫 `/` 로 분리.
  const idx = id.indexOf("/");
  if (idx > 0) {
    return { ptype: ptype || id.slice(0, idx), slug: id.slice(idx + 1) };
  }
  // 마지막 보루 — slug 만 있을 때 (page_type 으로 라우팅).
  return { ptype, slug: id };
}

interface Pattern {
  pattern_type?: string;
  description?: string;
  source?: string;
  confidence?: number;
}

interface Mistake {
  title?: string;
  description?: string;
  discovery_method?: string;
  created_at?: string;
  resolved?: boolean;
}

// page_type → 위키 칩(정본 .wk-* 매핑). 미지정 종류는 insight 톤.
const KIND_CHIP: Record<string, { cls: string; label: string }> = {
  insight: { cls: "wk-insight", label: "인사이트" },
  concept: { cls: "wk-insight", label: "개념" },
  decision: { cls: "wk-decision", label: "결정" },
  order: { cls: "wk-order", label: "지시" },
  entity: { cls: "wk-order", label: "엔티티" },
  comparison: { cls: "wk-evo", label: "비교" },
  other: { cls: "wk-insight", label: "기록" },
};

function kindChip(t?: string): { cls: string; label: string } {
  return (t && KIND_CHIP[t]) || { cls: "wk-insight", label: t || "기록" };
}

function fmtTs(ts?: number): string {
  if (!ts) return "";
  // updated_at 은 epoch(초 또는 ms) 가능 → 자릿수로 추정.
  const ms = ts > 1e12 ? ts : ts * 1000;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

export function WikiTab() {
  const [pages, { refetch: refetchPages }] = createResource<WikiPage[]>(() => invoke("wiki_pages_list"));
  const [patterns] = createResource<Pattern[]>(() => invoke("memory_patterns_list"));
  const [mistakes] = createResource<Mistake[]>(() => invoke("memory_mistakes_list"));
  const [q, setQ] = createSignal("");

  // --- 상세/편집 뷰 상태 ---
  // open: 현재 열린 페이지 (null 이면 목록). loading/error 는 본문 fetch.
  const [open, setOpen] = createSignal<WikiPage | null>(null);
  const [bodyDto, setBodyDto] = createSignal<WikiBody | null>(null);
  // LLM 위키 — 이 페이지의 나가는 링크 + backlinks(들어오는 링크).
  const [wlinks] = createResource(
    () => open()?.id,
    async (id) => {
      try { return await invoke<{ outgoing: { title: string; id: string | null }[]; backlinks: { id: string; title: string }[] }>("wiki_backlinks", { id }); }
      catch { return { outgoing: [], backlinks: [] }; }
    },
  );
  const openById = (pid: string) => { const p = (pages() ?? []).find((x) => x.id === pid); if (p) void openPage(p); };
  const [bodyLoading, setBodyLoading] = createSignal(false);
  const [bodyError, setBodyError] = createSignal<string | null>(null);
  const [editing, setEditing] = createSignal(false);
  const [editTitle, setEditTitle] = createSignal("");
  const [editBody, setEditBody] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [saveMsg, setSaveMsg] = createSignal<{ kind: "ok" | "err"; text: string } | null>(null);

  // 카드 클릭 → 상세 열기 + 본문 로드.
  async function openPage(p: WikiPage) {
    setOpen(p);
    setEditing(false);
    setSaveMsg(null);
    setBodyDto(null);
    setBodyError(null);
    setBodyLoading(true);
    const { ptype, slug } = resolveSlug(p);
    try {
      const dto = await invoke<WikiBody>("wiki_body_get", { ptype, slug });
      setBodyDto(dto);
    } catch (e) {
      setBodyError((e as Error).message || "본문을 불러오지 못했습니다.");
    } finally {
      setBodyLoading(false);
    }
  }

  function closeDetail() {
    setOpen(null);
    setBodyDto(null);
    setBodyError(null);
    setEditing(false);
    setSaveMsg(null);
  }

  // 읽기 → 편집 전환 (현재 본문을 textarea 초기값으로).
  function startEdit() {
    const dto = bodyDto();
    const p = open();
    setEditTitle((dto?.title ?? p?.title ?? "").trim());
    setEditBody(dto?.body ?? "");
    setSaveMsg(null);
    setEditing(true);
  }

  function cancelEdit() {
    setEditing(false);
    setSaveMsg(null);
  }

  // 저장 → wiki_body_put → 성공 시 본문 재로드 + 목록 갱신(updated_at).
  async function saveEdit() {
    const p = open();
    if (!p) return;
    const title = editTitle().trim();
    const body = editBody();
    if (!title) {
      setSaveMsg({ kind: "err", text: "제목을 입력하세요." });
      return;
    }
    const { ptype, slug } = resolveSlug(p);
    setSaving(true);
    setSaveMsg(null);
    try {
      const res = await invoke<WikiPutResult>("wiki_body_put", { ptype, slug, title, body });
      if (!res || res.ok !== true) {
        throw new Error("저장 응답이 올바르지 않습니다.");
      }
      // 저장 정본을 다시 읽어 표시 (H1 추출/정규화 반영).
      try {
        const dto = await invoke<WikiBody>("wiki_body_get", { ptype, slug });
        setBodyDto(dto);
      } catch {
        // 본문 재로드 실패해도 저장은 성공 — 입력값으로 임시 표시.
        setBodyDto({ slug: res.slug, title, body });
      }
      setEditing(false);
      setSaveMsg({ kind: "ok", text: "저장되었습니다." });
      void refetchPages();
    } catch (e) {
      setSaveMsg({ kind: "err", text: `저장 실패 — ${(e as Error).message}` });
    } finally {
      setSaving(false);
    }
  }

  const needle = () => q().trim().toLowerCase();

  const filteredPages = createMemo(() => {
    const n = needle();
    const list = pages() ?? [];
    if (!n) return list;
    return list.filter(
      (p) => (p.title || "").toLowerCase().includes(n) || (p.page_type || "").toLowerCase().includes(n),
    );
  });

  const filteredPatterns = createMemo(() => {
    const n = needle();
    const list = patterns() ?? [];
    if (!n) return list;
    return list.filter(
      (p) => (p.description || "").toLowerCase().includes(n) || (p.pattern_type || "").toLowerCase().includes(n),
    );
  });

  const filteredMistakes = createMemo(() => {
    const n = needle();
    const list = mistakes() ?? [];
    if (!n) return list;
    return list.filter(
      (m) => (m.title || "").toLowerCase().includes(n) || (m.description || "").toLowerCase().includes(n),
    );
  });

  const anyEvo = () => filteredPatterns().length > 0 || filteredMistakes().length > 0;

  return (
    // 정본 .ovl > .board 구조를 탭 본문(.kk-wiki)으로 인라인화. .board 의 .bh(헤더)/.bb(스크롤 본문) 그대로.
    <div class="kk-wiki">
      <div class="board">
        <div class="bh">
          <h2>📚 LLM 위키</h2>
          <span class="sub">Karpathy 방식 · 기록과 기억 + 자기개선</span>
        </div>
        <Show when={open()} fallback={
        <div class="bb">
          <input
            class="wsearch"
            placeholder="🔍 위키 검색 (의미 검색 · 지난 결정·인사이트·지시 찾기)"
            value={q()}
            onInput={(e) => setQ(e.currentTarget.value)}
          />

          {/* 기록 · 기억 (wiki_pages_list) */}
          <div class="wsec">기록 · 기억</div>
          <Show when={!pages.loading} fallback={<div class="kk-wiki-empty">불러오는 중…</div>}>
            <Show when={!pages.error} fallback={<div class="kk-wiki-empty err">⚠ 위키를 불러오지 못했습니다. 데몬 연결을 확인하세요.</div>}>
              <Show
                when={filteredPages().length > 0}
                fallback={
                  <div class="kk-wiki-empty">
                    {needle() ? "검색 결과가 없습니다." : "아직 기록된 위키 페이지가 없습니다."}
                  </div>
                }
              >
                <For each={filteredPages()}>
                  {(p) => {
                    const c = kindChip(p.page_type);
                    return (
                      <div
                        class="witem wclick"
                        role="button"
                        tabindex="0"
                        title="클릭하여 본문 보기·편집"
                        onClick={() => void openPage(p)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            void openPage(p);
                          }
                        }}
                      >
                        <div class="wt2">{p.title || "(제목 없음)"}</div>
                        <div class="wm">
                          <span class={`wkind ${c.cls}`}>{c.label}</span>
                          <Show when={fmtTs(p.updated_at)}><span>{fmtTs(p.updated_at)}</span></Show>
                          <span style="margin-left:auto; color:#b6bcc6;">본문 보기 ›</span>
                        </div>
                      </div>
                    );
                  }}
                </For>
              </Show>
            </Show>
          </Show>

          {/* 🧬 자기개선 (memory_mistakes_list + memory_patterns_list) */}
          <div class="wsec" style="margin-top:18px;">🧬 자기개선 (패턴 · 실수→규칙)</div>
          <Show when={!patterns.loading && !mistakes.loading} fallback={<div class="kk-wiki-empty">불러오는 중…</div>}>
            <Show when={!patterns.error && !mistakes.error} fallback={<div class="kk-wiki-empty err">⚠ 자기개선 데이터를 불러오지 못했습니다.</div>}>
              <Show when={anyEvo()} fallback={<div class="kk-wiki-empty">{needle() ? "검색 결과가 없습니다." : "아직 학습된 패턴·실수가 없습니다."}</div>}>
                <For each={filteredMistakes()}>
                  {(m) => (
                    <div class="witem">
                      <div class="wt2">
                        고친 실수: <b>{m.title || "(제목 없음)"}</b>
                        <Show when={m.description}><span class="wd"> · {m.description}</span></Show>
                      </div>
                      <div class="wm">
                        <span class="wkind wk-evo">실수→규칙</span>
                        <Show when={m.discovery_method}><span>{m.discovery_method}</span></Show>
                        <Show when={m.created_at}><span>{m.created_at}</span></Show>
                        <span>{m.resolved ? "해결됨" : "관찰 중"}</span>
                      </div>
                    </div>
                  )}
                </For>
                <For each={filteredPatterns()}>
                  {(p) => (
                    <div class="witem">
                      <div class="wt2">
                        학습된 패턴: <b>{p.pattern_type || "—"}</b>
                        <Show when={p.description}><span class="wd"> · {p.description}</span></Show>
                      </div>
                      <div class="wm">
                        <span class="wkind wk-evo">패턴</span>
                        <Show when={p.source}><span>{p.source}</span></Show>
                        <Show when={typeof p.confidence === "number"}>
                          <span>신뢰도 {(p.confidence as number).toFixed(2)}</span>
                        </Show>
                      </div>
                    </div>
                  )}
                </For>
              </Show>
            </Show>
          </Show>
        </div>
        }>
          {/* --- 상세/편집 뷰 (페이지 카드 클릭 시) — open() 이 페이지일 때 .bb 대체. --- */}
          <div class="bb">
            <div class="wdetail-bar">
              <button class="wback" type="button" onClick={closeDetail}>← 목록</button>
              <span class="wslug">{resolveSlug(open() as WikiPage).ptype}/{resolveSlug(open() as WikiPage).slug}</span>
              <div class="wdetail-actions">
                <Show when={!editing()}>
                  <button class="wbtn" type="button" disabled={bodyLoading() || !!bodyError()} onClick={startEdit}>
                    ✏ 편집
                  </button>
                </Show>
                <Show when={editing()}>
                  <button class="wbtn ghost" type="button" disabled={saving()} onClick={cancelEdit}>취소</button>
                  <button class="wbtn" type="button" disabled={saving()} onClick={() => void saveEdit()}>
                    {saving() ? "저장 중…" : "💾 저장"}
                  </button>
                </Show>
              </div>
            </div>

            <Show when={!bodyLoading()} fallback={<div class="wstatus busy">본문을 불러오는 중…</div>}>
              <Show
                when={!bodyError()}
                fallback={<div class="wstatus err">⚠ {bodyError()}</div>}
              >
                {/* 읽기 모드 */}
                <Show when={!editing()}>
                  <div class="wdetail-title">
                    {(bodyDto()?.title || (open() as WikiPage).title || "(제목 없음)").trim()}
                  </div>
                  <Show
                    when={(bodyDto()?.body || "").trim().length > 0}
                    fallback={<div class="kk-wiki-empty">본문이 비어 있습니다. ‘편집’으로 내용을 추가하세요.</div>}
                  >
                    <pre class="wbody">{bodyDto()?.body}</pre>
                  </Show>
                  {/* LLM 위키 — 링크/backlinks 패널 */}
                  <Show when={((wlinks()?.outgoing?.length || 0) + (wlinks()?.backlinks?.length || 0)) > 0}>
                    <div style="margin-top:14px; border-top:1px solid var(--border); padding-top:10px;">
                      <Show when={(wlinks()?.outgoing?.length || 0) > 0}>
                        <div style="font-size:12.5px; color:var(--text-3); margin-bottom:4px;">🔗 이 페이지가 가리키는 곳</div>
                        <div style="display:flex; flex-wrap:wrap; gap:6px; margin-bottom:8px;">
                          <For each={wlinks()!.outgoing}>{(l) => <span onClick={() => l.id && openById(l.id)} style={`cursor:${l.id ? "pointer" : "default"}; padding:3px 8px; border-radius:6px; font-size:12px; background:var(--bg-soft); border:1px solid var(--border); color:${l.id ? "var(--accent)" : "var(--text-3)"};`}>[[{l.title}]]{!l.id && " (미생성)"}</span>}</For>
                        </div>
                      </Show>
                      <Show when={(wlinks()?.backlinks?.length || 0) > 0}>
                        <div style="font-size:12.5px; color:var(--text-3); margin-bottom:4px;">↩ 이 페이지를 가리키는 곳 (backlinks)</div>
                        <div style="display:flex; flex-wrap:wrap; gap:6px;">
                          <For each={wlinks()!.backlinks}>{(b) => <span onClick={() => openById(b.id)} style="cursor:pointer; padding:3px 8px; border-radius:6px; font-size:12px; background:var(--bg-soft); border:1px solid var(--border); color:var(--accent);">{b.title}</span>}</For>
                        </div>
                      </Show>
                    </div>
                  </Show>
                </Show>

                {/* 편집 모드 */}
                <Show when={editing()}>
                  <input
                    class="wedit-title"
                    placeholder="제목"
                    value={editTitle()}
                    onInput={(e) => setEditTitle(e.currentTarget.value)}
                  />
                  <textarea
                    class="wedit-body"
                    placeholder="마크다운 본문…"
                    value={editBody()}
                    onInput={(e) => setEditBody(e.currentTarget.value)}
                  />
                </Show>

                <Show when={saveMsg()}>
                  <div class={`wstatus ${saveMsg()!.kind}`}>{saveMsg()!.text}</div>
                </Show>
              </Show>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
