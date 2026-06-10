import { createSignal, createResource, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./market.css";

// Phase 6 — 🌐 OpenAgentX 마켓 / 지갑 / 수익 / 외부작업.
// 정본 디자인: _mockups/kakao-mockup.html
//   #marketOvl  (L818-854) — 지갑 배지 + 카테고리 칩 + 2열 에이전트 카드 그리드
//   #budgetOvl  (L856-905) — 충전/잔액/에이전트별 예산/충전·사용 내역
//   #earningsOvl(L937-965) — 공개 에이전트 수익 + 외부 사용 이력
//   #extWorkOvl (L924-934) — 외부 요청 → 에이전트 작업 결과 상세
// 데이터 정직 원칙:
//   - 마켓/지갑/수익 백엔드 라우트 없음 (api/client.ts 확인: payment_* 만 존재).
//   - 일일 한도(限度)만 실제 payment_get_daily_limit / payment_set_daily_limit 로 배선.
//   - 잔액·시세·수익·리스팅 등 라이브 금액은 날조하지 않고 "준비 중(Phase 6 — 백엔드 미연결)"
//     placeholder 로 표기. 레이아웃은 정본 그대로 유지.

type SubView = "market" | "wallet" | "earnings" | "extwork";

// 카테고리 칩 — 정본 #marketOvl L826-829
const CATEGORIES = ["전체", "요약", "번역", "이미지", "리서치", "코딩", "SNS"] as const;

// 외부작업 상세 본문 — 정본 EXTWORK['review'] / ['sns'] (mockup JS)
interface ExtWork {
  t: string;
  req: string;
  body: string;
}
const EXTWORK: Record<string, ExtWork> = {
  review: {
    t: "코드리뷰 봇 · 외부 작업",
    req: "이 PR 리뷰해줘 → github.com/acme/web/pull/213 (로그인 폼 리팩터)",
    body:
      '<div class="toolcall"><span class="ok">✓</span> ⌗ fetch <span class="cmd">PR #213 diff (4 files, +212 -88)</span></div>' +
      "<p>리뷰 완료. <strong>버그 1 · 보안 1 · 스타일 2</strong> 발견.</p>" +
      '<pre class="code">🐞 <span class="g">auth.ts:42</span> async 누락 — await 없이 토큰 검증\n' +
      '🔒 <span class="g">login.tsx:func handleSubmit</span> 비밀번호 콘솔 로그 제거 필요\n' +
      "✦ 스타일: 미사용 import 2개</pre>" +
      "<p>수정 제안 패치를 코멘트로 남겼습니다.</p>",
  },
  sns: {
    t: "SNS 정리봇 · 외부 작업",
    req: "이 스레드 핵심만 3줄로 요약하고 해시태그 뽑아줘",
    body:
      '<div class="toolcall"><span class="ok">✓</span> ⌗ summarize <span class="cmd">thread (12 posts)</span></div>' +
      "<p>요약 완료. 핵심 3줄 + 해시태그 추출.</p>" +
      '<pre class="code">1. 신규 기능 출시 — 반응 긍정적\n' +
      "2. 가격 정책 문의 다수\n" +
      "3. 경쟁사 비교 언급 증가\n" +
      "#출시 #피드백 #가격</pre>",
  },
};

const fmtUsdc = (micro: number) => (micro / 1_000_000).toFixed(2);

async function fetchLimit(): Promise<number | null> {
  try {
    const raw = await invoke<unknown>("payment_get_daily_limit");
    const n = Number(raw);
    return Number.isFinite(n) ? n : null;
  } catch {
    // 데몬 미연결/미지원 — 날조하지 않고 null (UI 에서 '준비 중' 표기)
    return null;
  }
}

export function MarketTab() {
  const [view, setView] = createSignal<SubView>("market");
  const [cat, setCat] = createSignal<string>("전체");
  const [ext, setExt] = createSignal<{ work: ExtWork; meta: string } | null>(null);

  // 일일 한도 — 유일하게 실제 백엔드 배선되는 값
  const [limit, { refetch }] = createResource(fetchLimit);
  const [draft, setDraft] = createSignal<string>("");
  const [saving, setSaving] = createSignal(false);
  const [saveErr, setSaveErr] = createSignal<string | null>(null);

  const onSaveLimit = async () => {
    const num = Number(draft());
    if (!Number.isFinite(num) || num < 0) return;
    setSaving(true);
    setSaveErr(null);
    try {
      await invoke("payment_set_daily_limit", { microUsdc: Math.floor(num * 1_000_000) });
      setDraft("");
      void refetch();
    } catch (e) {
      setSaveErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const openExt = (type: string, meta: string) => {
    const w = EXTWORK[type];
    if (!w) return;
    setExt({ work: w, meta });
    setView("extwork");
  };

  return (
    <div class="kk-market">
      {/* ─────────── 마켓 (정본 #marketOvl) ─────────── */}
      <Show when={view() === "market"}>
        <div class="mk-head">
          <h2>🌐 OpenAgentX 마켓</h2>
          <span class="sub">공개 에이전트 검색·사용 · 쓴 만큼 지갑에서 차감</span>
          <button type="button" class="wallet" onClick={() => setView("wallet")}>
            👛 지갑 · 예산
          </button>
        </div>

        <div class="mk-note">
          마켓플레이스·지갑 백엔드는 <b>준비 중 (Phase 6 — 백엔드 미연결)</b>. 아래 공개 에이전트
          목록과 잔액·수익 수치는 정본 레이아웃 예시이며 라이브 데이터가 아닙니다. 실제로 연결된 값은
          지갑의 <b>일일 한도</b>(payment API) 한 곳뿐입니다.
        </div>

        <div class="wsearch">🔍 필요한 기능 검색 — 예: "유튜브 요약", "PDF 번역", "이미지 생성"</div>

        <button type="button" class="qbtn" onClick={() => setView("earnings")}>
          💰 내 공개 에이전트 · 수익 / 외부 작업 이력
        </button>

        <div class="mkcat">
          <For each={CATEGORIES}>
            {(c) => (
              <span class={`mc${cat() === c ? " on" : ""}`} onClick={() => setCat(c)}>
                {c}
              </span>
            )}
          </For>
        </div>

        <div class="mkgrid">
          {/* 정본 .mkcard ×4 — 라이브 리스팅 백엔드 없음(준비 중). 추가 버튼 비활성. */}
          <div class="mkcard">
            <div class="mkh">
              <div class="ava c-hermes">유</div>
              <div>
                <div class="mkn">유튜브 요약봇</div>
                <div class="mkby">by sangyeop · ⭐4.8 · 사용 2.1k</div>
              </div>
              <div class="mkprice">$0.05/회</div>
            </div>
            <div class="mkd">유튜브 링크 → 핵심 3줄 요약 + 타임스탬프 챕터.</div>
            <button type="button" class="mkuse" disabled title="준비 중 (Phase 6 — 백엔드 미연결)">
              ＋ 내 에이전트로 추가
            </button>
          </div>
          <div class="mkcard">
            <div class="mkh">
              <div class="ava c-gemini">번</div>
              <div>
                <div class="mkn">PDF 번역기</div>
                <div class="mkby">by mina · ⭐4.6 · 사용 980</div>
              </div>
              <div class="mkprice">$0.10/회</div>
            </div>
            <div class="mkd">PDF 통째로 한↔영 번역, 레이아웃 유지.</div>
            <button type="button" class="mkuse" disabled title="준비 중 (Phase 6 — 백엔드 미연결)">
              ＋ 내 에이전트로 추가
            </button>
          </div>
          <div class="mkcard">
            <div class="mkh">
              <div class="ava c-codex">이</div>
              <div>
                <div class="mkn">이미지 생성봇</div>
                <div class="mkby">by devkim · ⭐4.9 · 사용 5.3k</div>
              </div>
              <div class="mkprice">$0.20/장</div>
            </div>
            <div class="mkd">텍스트 프롬프트로 고해상도 이미지 생성.</div>
            <button type="button" class="mkuse" disabled title="준비 중 (Phase 6 — 백엔드 미연결)">
              ＋ 내 에이전트로 추가
            </button>
          </div>
          <div class="mkcard">
            <div class="mkh">
              <div class="ava c-claude">코</div>
              <div>
                <div class="mkn">코드리뷰 봇</div>
                <div class="mkby">by starian · ⭐4.7 · 사용 1.4k</div>
              </div>
              <div class="mkprice">$0.50/회</div>
            </div>
            <div class="mkd">PR 링크 → 버그·보안·스타일 리뷰 코멘트.</div>
            <button type="button" class="mkuse" disabled title="준비 중 (Phase 6 — 백엔드 미연결)">
              ＋ 내 에이전트로 추가
            </button>
          </div>
        </div>
      </Show>

      {/* ─────────── 지갑·예산 (정본 #budgetOvl) ─────────── */}
      <Show when={view() === "wallet"}>
        <button type="button" class="mk-back" onClick={() => setView("market")}>
          ← 마켓으로
        </button>
        <div class="board">
          <div class="bh">
            <h2>👛 지갑 · 예산</h2>
            <span class="sub">공개 에이전트 사용 비용 관리</span>
            <button type="button" class="bx" onClick={() => setView("market")}>
              ✕
            </button>
          </div>
          <div class="bb">
            {/* 잔액·이번 달 사용 — 백엔드 없음(준비 중). 한도만 실제 payment API. */}
            <div class="budtop">
              <div>
                <div class="big">준비 중</div>
                <div class="cap">잔액 · 백엔드 미연결 (Phase 6)</div>
              </div>
              <div>
                <div class="big" style="font-size:17px;color:#c9760e;">준비 중</div>
                <div class="cap">이번 달 사용</div>
              </div>
              <div style="margin-left:auto;display:flex;gap:8px;align-items:center;">
                <button type="button" class="budbtn" disabled title="준비 중 (Phase 6 — 백엔드 미연결)">
                  충전
                </button>
              </div>
            </div>

            {/* 일일 한도 — 실제 payment_get/set_daily_limit 배선 */}
            <div class="wsec">일일 한도 — 실제 연결 (payment API)</div>
            <div class="budrow">
              <div class="ava c-primary">👑</div>
              <div>
                <div class="bn">데몬 일일 한도</div>
                <div class="cap" style="font-size:11px;color:#9aa1ad;">USDC · 자동 차감 상한</div>
              </div>
              <div class="budamt">
                <Show
                  when={!limit.loading}
                  fallback={<div class="u">불러오는 중…</div>}
                >
                  <Show
                    when={limit() != null}
                    fallback={<div class="u" style="color:#9aa1ad;">준비 중</div>}
                  >
                    <div class="u">${fmtUsdc(limit() as number)}</div>
                    <div class="l">/ 일</div>
                  </Show>
                </Show>
              </div>
            </div>
            <div style="display:flex;gap:6px;margin:10px 0 4px;align-items:center;">
              <input
                type="number"
                step="0.01"
                min="0"
                value={draft()}
                placeholder="새 일일 한도 (USDC)"
                onInput={(e) => setDraft(e.currentTarget.value)}
                style="flex:1;border:1px solid var(--kk-line);border-radius:8px;padding:8px 10px;font-size:12.5px;"
              />
              <button
                type="button"
                class="budbtn alt"
                disabled={!draft() || saving()}
                onClick={() => void onSaveLimit()}
              >
                {saving() ? "저장 중…" : "한도 변경"}
              </button>
            </div>
            <Show when={saveErr()}>
              <div style="font-size:11.5px;color:#d64545;margin-bottom:8px;">한도 변경 실패: {saveErr()}</div>
            </Show>

            <div style="font-size:11.5px;color:#9aa1ad;margin-top:14px;">
              ⚠️ 에이전트가 한도 도달 시 동작: <b>자동 정지</b> · <b>알림만</b> — 에이전트별 선택은
              <b> 준비 중 (Phase 6)</b>. 현재는 데몬 일일 한도(payment API)만 적용됩니다.
            </div>

            {/* 충전·사용 내역 — 백엔드 없음 */}
            <div class="wsec" style="margin-top:20px;">충전 · 사용 내역</div>
            <div class="mk-note">
              충전/사용 거래내역은 <b>준비 중 (Phase 6 — 백엔드 미연결)</b>. 결제 데몬 연결 후 이 자리에
              실제 내역이 표시됩니다.
            </div>
          </div>
        </div>
      </Show>

      {/* ─────────── 수익 (정본 #earningsOvl) ─────────── */}
      <Show when={view() === "earnings"}>
        <button type="button" class="mk-back" onClick={() => setView("market")}>
          ← 마켓으로
        </button>
        <div class="board">
          <div class="bh">
            <h2>💰 공개 에이전트 · 수익</h2>
            <span class="sub">외부 사용 이력 + 수익 내역</span>
            <button type="button" class="bx" onClick={() => setView("market")}>
              ✕
            </button>
          </div>
          <div class="bb">
            <div class="budtop">
              <div>
                <div class="big" style="color:#1f9d4d;">준비 중</div>
                <div class="cap">이번 달 수익 · 백엔드 미연결</div>
              </div>
              <div>
                <div class="big" style="font-size:17px;">준비 중</div>
                <div class="cap">누적 수익</div>
              </div>
              <div style="margin-left:auto;">
                <button type="button" class="budbtn" disabled title="준비 중 (Phase 6 — 백엔드 미연결)">
                  정산하기 (USDC)
                </button>
              </div>
            </div>

            <div class="mk-note">
              수익·정산 백엔드는 <b>준비 중 (Phase 6 — 백엔드 미연결)</b>. 아래 항목은 정본 레이아웃
              예시이며 라이브 수익이 아닙니다.
            </div>

            <div class="wsec">공개한 에이전트 · 2</div>
            <div class="earn-item">
              <div class="earn-top">
                <div class="ava c-hermes">S</div>
                <div>
                  <div class="en">SNS 정리봇</div>
                  <div class="erate">⭐ 4.8 · $0.05/회</div>
                </div>
                <div class="er">+$6.20 이번 달</div>
              </div>
              <div class="earn-stat">
                외부 사용 <b>124회</b> (이번 달 18회) · 누적 수익 <b>$62.00</b>
              </div>
            </div>
            <div class="earn-item">
              <div class="earn-top">
                <div class="ava c-claude">코</div>
                <div>
                  <div class="en">코드리뷰 봇</div>
                  <div class="erate">⭐ 4.7 · $0.50/회</div>
                </div>
                <div class="er">+$18.60 이번 달</div>
              </div>
              <div class="earn-stat">
                외부 사용 <b>1.4k회</b> (이번 달 37회) · 누적 수익 <b>$250.40</b>
              </div>
            </div>

            <div class="wsec" style="margin-top:18px;">
              최근 외부 사용 이력{" "}
              <span style="font-weight:500;color:#b6bcc6;">(클릭 → 작업 내용)</span>
            </div>
            <div class="txn" style="cursor:pointer;" onClick={() => openExt("sns", "익명#a91 · 방금 · +$0.05")}>
              <div class="ti in">↗</div>
              <div>
                <div class="tn">SNS 요약</div>
                <div class="ts"><span class="tu">익명#a91</span> · 방금</div>
              </div>
              <div class="ta plus">+$0.05</div>
            </div>
            <div class="txn" style="cursor:pointer;" onClick={() => openExt("review", "익명#f80 · 12분 전 · +$0.50")}>
              <div class="ti in">↗</div>
              <div>
                <div class="tn">코드리뷰 (PR #213)</div>
                <div class="ts"><span class="tu">익명#f80</span> · 12분 전</div>
              </div>
              <div class="ta plus">+$0.50</div>
            </div>
            <div class="txn" style="cursor:pointer;" onClick={() => openExt("sns", "익명#c12 · 26분 전 · +$0.05")}>
              <div class="ti in">↗</div>
              <div>
                <div class="tn">SNS 요약</div>
                <div class="ts"><span class="tu">익명#c12</span> · 26분 전</div>
              </div>
              <div class="ta plus">+$0.05</div>
            </div>
            <div class="txn" style="cursor:pointer;" onClick={() => openExt("review", "익명#2db · 1시간 전 · +$0.50")}>
              <div class="ti in">↗</div>
              <div>
                <div class="tn">코드리뷰 (PR #208)</div>
                <div class="ts"><span class="tu">익명#2db</span> · 1시간 전</div>
              </div>
              <div class="ta plus">+$0.50</div>
            </div>
            <div style="font-size:11px;color:#9aa1ad;margin-top:12px;">
              외부 사용자는 익명 처리됩니다. 데몬이 사용 1건마다 기록·정산. 요청·결과 전문은 위키에도
              저장. (정산·기록 백엔드는 준비 중)
            </div>
          </div>
        </div>
      </Show>

      {/* ─────────── 외부작업 상세 (정본 #extWorkOvl) ─────────── */}
      <Show when={view() === "extwork"}>
        <button type="button" class="mk-back" onClick={() => setView("earnings")}>
          ← 수익으로
        </button>
        <div class="board">
          <div class="bh">
            <h2>{ext()?.work.t ?? "외부 작업 상세"}</h2>
            <span class="sub">{ext()?.meta ?? "요청 → 에이전트 작업 결과"}</span>
            <button type="button" class="bx" onClick={() => setView("earnings")}>
              ✕
            </button>
          </div>
          <div class="bb">
            <div class="wsec">외부 요청</div>
            <div class="ew-req">{ext()?.work.req}</div>
            <div class="wsec">에이전트 작업 결과</div>
            {/* eslint-disable-next-line solid/no-innerhtml */}
            <div class="ew-body" innerHTML={ext()?.work.body ?? ""} />
          </div>
        </div>
      </Show>
    </div>
  );
}
