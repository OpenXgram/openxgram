# nashsu/llm_wiki — Research Report (for OpenXgram wiki rebuild)

> Researched 2026-06-12 from the actual repo https://github.com/nashsu/llm_wiki
> (verified: real project, 11.2k stars, 1.4k forks, 569 commits, active).

---

## 1. What it actually IS

**LLM Wiki** is a **cross-platform desktop app (Tauri v2 + React 19)** that turns
your documents into an **organized, interlinked, self-maintaining knowledge base**.

Core thesis — the headline differentiator:

> Instead of traditional RAG (retrieve-and-answer from scratch on every query),
> the LLM **incrementally builds and maintains a persistent wiki** from your
> sources. **Knowledge is compiled once and kept current, not re-derived per query.**

It is a concrete implementation of **Karpathy's "LLM Wiki" pattern**
(gist 442a6bf...) with significant engineering enhancements. The wiki is just a
folder of markdown files (Obsidian-compatible vault) — not a database blob.

### Architecture it KEPT from Karpathy
- **Three-layer**: Raw Sources (immutable) → Wiki (LLM-generated) → Schema (rules/config)
- **Three core operations**: Ingest, Query, Lint
- `index.md` (catalog / LLM nav entry), `log.md` (chronological op record, parseable)
- `[[wikilink]]` cross-references + **YAML frontmatter** on every page
- "Human curates, LLM maintains" role division

### File layout (the data model — important, copy-able)
```
my-wiki/
├── purpose.md       # goals, key questions, research scope
├── schema.md        # structure rules, page types
├── raw/sources/     # uploaded docs (immutable) + raw/assets/
├── wiki/
│   ├── index.md     # content catalog
│   ├── log.md       # operation history
│   ├── overview.md  # global summary (auto-updated)
│   ├── entities/    # people, orgs, products
│   ├── concepts/    # theories, methods, techniques
│   ├── sources/     # per-source summaries
│   ├── queries/     # saved chat answers + research
│   ├── synthesis/   # cross-source analysis
│   └── comparisons/ # side-by-side
└── .llm-wiki/       # app config, chat history, review items
```

### Tech stack
Tauri v2 (Rust) · React19+TS+Vite · Milkdown editor · **sigma.js+graphology+ForceAtlas2** (graph) · search = tokenized + graph relevance + **optional LanceDB vector** · LLM via streaming fetch (OpenAI/Anthropic/Google/Ollama) · web search Tavily/SerpApi/SearXNG.

---

## 2. KEY features/patterns (what makes it a *good* LLM wiki)

| # | Pattern | What it does |
|---|---------|--------------|
| 1 | **Two-step CoT ingest** | Step1 *Analysis* (entities, concepts, connections, contradictions, structure recs) → Step2 *Generation* (entity/concept pages w/ cross-refs, updates index/log/overview, review items). Split = much better quality than single-pass. |
| 2 | **Typed pages** | entity / concept / source / query / synthesis / comparison — gives structure, not a flat note pile. |
| 3 | **4-signal knowledge graph** | edge weight = Direct `[[link]]` ×3.0 + **Source overlap** (shared frontmatter `sources[]`) ×4.0 + **Adamic-Adar** (shared neighbors) ×1.5 + Type affinity ×1.0. This is the heart of "interlinked." |
| 4 | **Louvain community detection** | auto-clusters pages by link topology; cohesion scoring flags weak clusters. |
| 5 | **Graph Insights** | surprising connections (cross-community/cross-type edges), **knowledge gaps** (isolated pages deg≤1, sparse communities, bridge nodes) → one-click Deep Research. |
| 6 | **Multi-phase retrieval** | tokenized search (+title bonus) → optional vector (LanceDB) → **2-hop graph expansion with decay** → token-budget allocation (60% pages/20% chat/...) → numbered context with **`[1][2]` citations**. |
| 7 | **Source traceability** | every page's frontmatter lists `sources[]`; answers cite page numbers. |
| 8 | **SHA256 incremental cache** | hash sources, skip unchanged → save tokens. |
| 9 | **Persistent ingest queue** | serial, crash-recoverable, auto-retry. |
| 10 | **Async review system** | LLM flags items for human judgment with *constrained* action types (Create Page / Deep Research / Skip) — prevents hallucinated actions. |
| 11 | **Deep Research** | LLM detects gaps → web search → synthesizes a new wiki page w/ cross-refs → auto-ingests. |
| 12 | **Local HTTP API + MCP server** | `127.0.0.1:19828` JSON API + MCP for hybrid search / file read / graph traversal — agents can use the wiki. |

---

## 3. License — ⚠️ BLOCKER

**GNU GPL v3.0.** This is **incompatible with OpenXgram's MIT**.

→ We **cannot copy code** from llm_wiki. We **can** freely adopt the *ideas /
patterns / data model* (concepts, file layout, the 4-signal formula, the
two-step ingest flow are not copyrightable) and **reimplement clean-room** in
Rust/SQLite/SolidJS. The underlying **Karpathy gist pattern is the real
source of truth** and is what we should cite. Do not vendor any of their source.

---

## 4. Adoptable patterns mapped to OUR stack (Rust + SQLite + SolidJS + existing wiki_pages/embeddings/L0-L4)

| Pattern | Effort in our stack | What it adds over our current basic wiki |
|---|---|---|
| **Typed pages** (entity/concept/source/synthesis) | **Easy** — add `page_type` + `frontmatter` (sources[], links[]) columns to `wiki_pages`; enum in MCP `write_wiki_page` | Turns flat page list into structured KB. Foundation for everything else. |
| **`[[wikilink]]` + backlinks** | **Easy** — parse `[[..]]` on upsert into a `wiki_links(src_id, dst_title)` table; backlink query | Interlinking — the #1 thing missing. Cheap, huge value. |
| **Two-step CoT ingest** (analyze→generate) | **Medium** — new MCP tool `ingest_source` that does 2 LLM calls; reuse our LLM client | Auto-generates structured pages from raw docs vs hand-writing. The "self-building" magic. |
| **Source traceability + `[N]` citations** | **Medium** — `sources[]` in frontmatter; numbered-context assembly in `search_wiki` | Trust/verifiability; lets agents cite. |
| **4-signal graph** (direct/source-overlap/Adamic-Adar/type) | **Medium** — pure SQL/Rust over `wiki_links` + `sources[]`; no new dep | Relevance-ranked related pages; powers retrieval + viz. We already have embeddings for the vector signal. |
| **Multi-phase retrieval** (keyword→vector→2-hop graph→budget) | **Medium** — we already have embeddings; add keyword (SQLite FTS5) + graph hop + budget assembler | Far better than our current embeddings-only search. |
| **SHA256 incremental cache** | **Easy** — we already store `content_hash`; gate ingest on it | Token savings; trivial. |
| **Graph Insights** (isolated pages, gaps, bridges) | **Medium** — graph analytics queries; surface in WikiTab | Tells user *what to research next* — makes wiki feel alive. |
| **Louvain community detection** | **Hard** — needs a community-detection impl in Rust (no graphology); could defer or use simple connected-components first | Auto topic clusters; nice-to-have, not core. |
| **Graph visualization** (sigma.js) | **Medium** — SolidJS + a force-graph lib in WikiTab.tsx | Visual KB; high demo value but not core utility. |
| **Deep Research** (gap→web search→ingest) | **Hard** — needs web-search integration + orchestration | Powerful but heavy; phase 2+. |
| **Async review queue** (constrained actions) | **Medium** — `wiki_review_items` table + GUI panel | Human-in-loop quality gate; pairs with auto-ingest. |
| **Local HTTP/MCP exposure** | **Already have it** — we have MCP wiki tools + GUI endpoints | Just extend with new ops (graph traversal, ingest). |

### Where we're already ahead / aligned
- We already have **embeddings + MCP tools + GUI + L0-L4 memory** — llm_wiki's vector search is *optional/off-by-default*; ours is built-in. Our **L0-L4 memory layers** map naturally to their raw→wiki→synthesis layering.
- Our SQLite-with-body-on-disk is fine; we don't need their full Obsidian-vault-on-disk model (though exporting to `[[wikilink]]` markdown for Obsidian compat is a cheap bonus).

---

## 5. Prioritized TOP-3 to build first (weak → genuinely good)

These three convert our flat page store into a real *interlinked, self-building*
wiki — the exact gap the master called out.

### #1 — Wikilinks + backlinks + typed pages (Easy, foundational)
Add `page_type` + frontmatter (`sources[]`, parsed `[[links]]`) to `wiki_pages`;
new `wiki_links(src_id, dst_title)` table populated on every upsert; expose
backlinks in `read_wiki_page` and WikiTab. **This alone fixes "interlinked."**
Nothing else works without it.

### #2 — Two-step CoT ingest tool (Medium, the "self-building" core)
New MCP/GUI op `ingest_source(text|file)`: LLM call 1 = structured analysis
(entities, concepts, connections, contradictions); LLM call 2 = generate/update
typed wiki pages with `[[links]]` + `sources[]` frontmatter + update index/log/
overview. Gate on SHA256 (`content_hash` we already store). **This is what turns
"a notes table" into "a wiki that builds itself" — the headline value.**

### #3 — 4-signal graph relevance + multi-phase retrieval (Medium, the payoff)
Build the relevance engine over #1's link table + `sources[]` + our existing
embeddings (direct-link ×3, source-overlap ×4, Adamic-Adar ×1.5, type ×1).
Wire into `search_wiki`: FTS5 keyword + embedding vector + 2-hop graph expansion
+ token-budget numbered context with `[N]` citations. **This makes retrieval and
"related pages" dramatically better than today's embeddings-only search**, and
unlocks Graph Insights (gaps/bridges) later as a thin layer on top.

> Defer: Louvain clustering, sigma.js viz, Deep Research, async review — all
> valuable phase-2 items but they sit *on top of* #1–#3. Build the graph + ingest
> spine first.

**License reminder:** clean-room reimplement in Rust; cite Karpathy's gist as the
pattern source; copy zero code from the GPL-v3 repo.
