import { createSignal, createResource, createMemo, For, Show } from "solid-js";
import { invoke } from "../api/client";

// 위키 탭 — 카카오톡 네이티브 재디자인. 정본: _mockups/kakao-mockup.html (#wikiOvl · .wsearch · .witem).
// "Karpathy 방식 · 기록과 기억 + 자기개선". 오버레이가 아닌 인라인 풀하이트 패널 (AgentsTab/TalkTab 패턴).
// 백엔드 contract 재사용(신규 명령 발명 X):
//   wiki_pages_list      → WikiPage[]  (기록·기억 보드)
//   memory_patterns_list → Pattern[]   (자기개선 · 학습된 패턴)
//   memory_mistakes_list → Mistake[]   (자기개선 · 실수→규칙)
// MemoryTab(stub)에 의미검색 명령이 없으므로 검색바는 로드된 목록에 대한 클라이언트 필터.

interface WikiPage {
  id: string;
  title: string;
  page_type: string;
  updated_at?: number;
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

// page_type → 위키 칩 색 (정본 .wk-* 매핑). 미지정 종류는 insight 톤.
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
  // updated_at 는 epoch(초 또는 ms) 가능 → 자릿수로 추정.
  const ms = ts > 1e12 ? ts : ts * 1000;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

export function WikiTab() {
  const [pages] = createResource<WikiPage[]>(() => invoke("wiki_pages_list"));
  const [patterns] = createResource<Pattern[]>(() => invoke("memory_patterns_list"));
  const [mistakes] = createResource<Mistake[]>(() => invoke("memory_mistakes_list"));
  const [q, setQ] = createSignal("");

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
    <div class="kk-wiki">
      <div class="kk-wiki-head">
        <h2>📚 LLM 위키</h2>
        <span class="sub">Karpathy 방식 · 기록과 기억 + 자기개선</span>
      </div>

      <div class="kk-wiki-body">
        <input
          class="kk-wiki-search"
          placeholder="🔍 위키 검색 (지난 결정·인사이트·지시 찾기)"
          value={q()}
          onInput={(e) => setQ(e.currentTarget.value)}
        />

        {/* 기록 · 기억 (wiki_pages_list) */}
        <div class="kk-wiki-sec">기록 · 기억</div>
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
                    <div class="kk-wiki-item">
                      <div class="wt">{p.title || "(제목 없음)"}</div>
                      <div class="wm">
                        <span class={`kk-wiki-kind ${c.cls}`}>{c.label}</span>
                        <Show when={fmtTs(p.updated_at)}><span>{fmtTs(p.updated_at)}</span></Show>
                      </div>
                    </div>
                  );
                }}
              </For>
            </Show>
          </Show>
        </Show>

        {/* 🧬 자기개선 (memory_patterns_list + memory_mistakes_list) */}
        <div class="kk-wiki-sec evo">🧬 자기개선 (패턴 · 실수→규칙)</div>
        <Show when={!patterns.loading && !mistakes.loading} fallback={<div class="kk-wiki-empty">불러오는 중…</div>}>
          <Show when={!patterns.error && !mistakes.error} fallback={<div class="kk-wiki-empty err">⚠ 자기개선 데이터를 불러오지 못했습니다.</div>}>
            <Show when={anyEvo()} fallback={<div class="kk-wiki-empty">{needle() ? "검색 결과가 없습니다." : "아직 학습된 패턴·실수가 없습니다."}</div>}>
              <For each={filteredMistakes()}>
                {(m) => (
                  <div class="kk-wiki-item">
                    <div class="wt">
                      고친 실수: <b>{m.title || "(제목 없음)"}</b>
                      <Show when={m.description}><span class="wd"> · {m.description}</span></Show>
                    </div>
                    <div class="wm">
                      <span class="kk-wiki-kind wk-evo">실수→규칙</span>
                      <Show when={m.discovery_method}><span>{m.discovery_method}</span></Show>
                      <Show when={m.created_at}><span>{m.created_at}</span></Show>
                      <span>{m.resolved ? "해결됨" : "관찰 중"}</span>
                    </div>
                  </div>
                )}
              </For>
              <For each={filteredPatterns()}>
                {(p) => (
                  <div class="kk-wiki-item">
                    <div class="wt">
                      학습된 패턴: <b>{p.pattern_type || "—"}</b>
                      <Show when={p.description}><span class="wd"> · {p.description}</span></Show>
                    </div>
                    <div class="wm">
                      <span class="kk-wiki-kind wk-evo">패턴</span>
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
    </div>
  );
}
