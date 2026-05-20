import { createSignal, Show } from "solid-js";
import { MemoryTab } from "./MemoryTab";

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
        <section class="card-section">
          <h3>📄 위키 페이지 — 사양 §3~§4 (L2)</h3>
          <p class="placeholder-note">
            카테고리 트리 (최대 5단) + 태그 + 최근 + 새 페이지 알림.
            5 모드: 위키 / 편집 (마크다운+위지윅) / 이력 / 그래프 (Cytoscape.js) / 검색.
            기존 MemoryTab 통합 (L2 memories) — 위키 페이지 CRUD 별도 신설 필요 (Phase 2).
          </p>
          <MemoryTab />
        </section>
      </Show>

      <Show when={tab() === "search"}>
        <section class="card-section">
          <h3>🔍 검색 — 사양 §11 (V-10)</h3>
          <input
            type="text"
            placeholder="검색어 입력… (예: OpenAgentX 결제 정책)"
            style="width:100%; padding:8px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px; margin-bottom:8px;"
          />
          <p class="placeholder-note">
            FTS5 (키워드) + sqlite-vec (시멘틱) hybrid + RRF (Reciprocal Rank Fusion).
            L0 raw 메시지 + L1 episode + L2 위키 + L3 패턴 + L4 trait + Mistake + Comments 통합 검색.
            백엔드 `GET /v1/gui/memory/search?q=` 신설 필요.
          </p>
        </section>
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
