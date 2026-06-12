import { createSignal, createResource, For, Show } from "solid-js";
import { invoke } from "../api/client";
import "./market.css";

// Phase 6 — 🌐 OpenAgentX 마켓 / 지갑 / 수익 / 외부작업.
// 정본 디자인: _mockups/kakao-mockup.html
//   #marketOvl  (L818-854) — 지갑 배지 + 카테고리 칩 + 2열 에이전트 카드 그리드
//   #budgetOvl  (L856-905) — 충전/잔액/에이전트별 예산/충전·사용 내역
//   #earningsOvl(L937-965) — 공개 에이전트 수익 + 외부 사용 이력
//   #extWorkOvl (L924-934) — 외부 요청 → 에이전트 작업 결과 상세
// 데이터 정직 원칙 (마켓 (c)갈래 배선 완료):
//   - 잔액·충전·거래내역·수익은 실제 백엔드(wallets_list / wallet_ledger / wallet_topup)로 배선.
//     잔액 = sub_wallets balance 합, 거래내역·수익 = wallet_ledger(내부 원장) 실데이터.
//   - 구매(차감)는 MCP purchase_service(내부 ledger gateway)가 sub_wallets 잔액을 실제 차감.
//   - 일일 한도(payment_*_daily_limit) 도 실제 배선 유지.
//   - 마켓 리스팅(공개 에이전트 카드)은 디렉토리(OpenAgentX) 미배포 시 정본 레이아웃 예시로 유지.

type SubView = "market" | "wallet" | "earnings" | "extwork";

interface LedgerEntry {
  id: string;
  agent_id: string;
  kind: string; // 'topup' | 'purchase' | 'earn'
  amount_micro: number;
  chain?: string | null;
  counterparty?: string | null;
  memo?: string | null;
  created_at: string;
}
interface LedgerDto {
  entries: LedgerEntry[];
  total_topup_micro: number;
  total_purchase_micro: number;
  total_earned_micro: number;
}
interface SubWallet {
  agent_id: string;
  derived_address: string;
  balance_micro: number;
  spent_micro: number;
  earned_micro: number;
  status: string;
}
interface MasterWallet {
  address?: string | null;
  free_micro: number;
}
interface WalletsDto {
  master: MasterWallet;
  sub_wallets: SubWallet[];
  next_hd_index: number;
}

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

// 마켓 (c)갈래 — 지갑(잔액) 실데이터. 백엔드 미연결 시 null (UI '준비 중').
async function fetchWallets(): Promise<WalletsDto | null> {
  try {
    return await invoke<WalletsDto>("wallets_list");
  } catch {
    return null;
  }
}

// 마켓 (c)갈래 — 거래 원장 + 집계 실데이터.
async function fetchLedger(): Promise<LedgerDto | null> {
  try {
    return await invoke<LedgerDto>("wallet_ledger", { limit: 50 });
  } catch {
    return null;
  }
}

// 마켓 (d)갈래 — free-tier 무료 할당량 상태 (전역 기본 기준 잔여/사용량).
interface FreeTierStatus {
  agent_id: string;
  free_per_day: number;
  used_today: number;
  remaining: number;
  has_override: boolean;
}
async function fetchFreeTier(): Promise<FreeTierStatus | null> {
  try {
    // agentId 생략 → 전역 기본("*") 기준 잔여/사용량.
    return await invoke<FreeTierStatus>("free_tier_status");
  } catch {
    return null;
  }
}

// 온체인 결제 지갑 — keystore master 주소 + Base 체인 ETH/USDC 실잔액 (가짜 값 없음).
interface PaymentWallet {
  address: string | null;
  chain: string;
  rpc_url: string | null;
  eth_balance: string | null; // wei (string)
  usdc_balance: string | null; // micro (6 decimals, string)
  onchain_enabled: boolean;
  error: string | null;
}
async function fetchPaymentWallet(): Promise<PaymentWallet | null> {
  try {
    return await invoke<PaymentWallet>("payment_wallet");
  } catch {
    // 데몬 미연결/미지원 — 날조하지 않고 null (UI '준비 중' 표기)
    return null;
  }
}
// wei(18 decimals) string → 사람이 읽는 ETH. null/실패는 그대로 null 표기.
function fmtEth(wei: string | null): string {
  if (wei == null) return "—";
  try {
    const v = BigInt(wei);
    const whole = v / 1_000_000_000_000_000_000n;
    const frac = (v % 1_000_000_000_000_000_000n).toString().padStart(18, "0").slice(0, 6);
    return `${whole.toString()}.${frac}`;
  } catch {
    return "—";
  }
}
// USDC micro(6 decimals) string → 사람이 읽는 USDC.
function fmtUsdcStr(micro: string | null): string {
  if (micro == null) return "—";
  try {
    const v = BigInt(micro);
    const whole = v / 1_000_000n;
    const frac = (v % 1_000_000n).toString().padStart(6, "0").slice(0, 2);
    return `${whole.toString()}.${frac}`;
  } catch {
    return "—";
  }
}
function shortAddr(a: string | null): string {
  if (!a) return "주소 없음";
  return a.length > 12 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a;
}

// 총 잔액 = 서브 지갑 balance 합 + 마스터 free.
function totalBalanceMicro(w: WalletsDto | null | undefined): number {
  if (!w) return 0;
  const sub = (w.sub_wallets ?? []).reduce((a, s) => a + (s.balance_micro || 0), 0);
  return sub + (w.master?.free_micro || 0);
}

const KIND_LABEL: Record<string, string> = { topup: "충전", purchase: "구매", earn: "수익" };
function relTime(iso: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return iso;
  const diff = Date.now() - t;
  const m = Math.floor(diff / 60000);
  if (m < 1) return "방금";
  if (m < 60) return `${m}분 전`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}시간 전`;
  return `${Math.floor(h / 24)}일 전`;
}

export function MarketTab() {
  const [view, setView] = createSignal<SubView>("market");
  const [cat, setCat] = createSignal<string>("전체");
  const [ext, setExt] = createSignal<{ work: ExtWork; meta: string } | null>(null);

  // 일일 한도 — 실제 백엔드 배선
  const [limit, { refetch }] = createResource(fetchLimit);
  const [draft, setDraft] = createSignal<string>("");
  const [saving, setSaving] = createSignal(false);
  const [saveErr, setSaveErr] = createSignal<string | null>(null);

  // 마켓 (c)갈래 — 지갑(잔액) + 거래원장 실데이터 resource.
  const [wallets, { refetch: refetchWallets }] = createResource(fetchWallets);
  const [ledger, { refetch: refetchLedger }] = createResource(fetchLedger);

  // 온체인 결제 지갑 — keystore master 주소 + Base 체인 ETH/USDC 실잔액.
  const [payWallet, { refetch: refetchPayWallet }] = createResource(fetchPaymentWallet);
  const [copied, setCopied] = createSignal(false);
  const onCopyAddr = async () => {
    const a = payWallet()?.address;
    if (!a) return;
    try {
      await navigator.clipboard.writeText(a);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* 클립보드 미지원 — 조용히 무시 (주소는 화면에 이미 노출됨) */
    }
  };

  // 마켓 (d)갈래 — free-tier 무료 할당량(전역 기본) 상태 + 설정.
  const [freeTier, { refetch: refetchFreeTier }] = createResource(fetchFreeTier);
  const [ftDraft, setFtDraft] = createSignal<string>("");
  const [ftSaving, setFtSaving] = createSignal(false);
  const [ftErr, setFtErr] = createSignal<string | null>(null);

  const onSaveFreeTier = async () => {
    const n = Number(ftDraft());
    if (!Number.isFinite(n) || n < 0) return;
    setFtSaving(true);
    setFtErr(null);
    try {
      // agent_id 생략 → 전역 기본("*") 설정.
      await invoke("free_tier_config_set", { free_per_day: Math.floor(n) });
      setFtDraft("");
      void refetchFreeTier();
    } catch (e) {
      setFtErr(e instanceof Error ? e.message : String(e));
    } finally {
      setFtSaving(false);
    }
  };

  // 충전(topup) — 마스터 → 첫 서브 지갑 즉시 이체. 실제 잔액 변화.
  const [topupAmt, setTopupAmt] = createSignal<string>("");
  const [topupBusy, setTopupBusy] = createSignal(false);
  const [topupErr, setTopupErr] = createSignal<string | null>(null);

  const firstAgentId = () => wallets()?.sub_wallets?.[0]?.agent_id ?? null;

  const onTopup = async () => {
    const usd = Number(topupAmt());
    if (!Number.isFinite(usd) || usd <= 0) return;
    const aid = firstAgentId();
    if (!aid) {
      setTopupErr("서브 지갑이 없습니다 — 지갑 생성 후 충전하세요.");
      return;
    }
    setTopupBusy(true);
    setTopupErr(null);
    try {
      await invoke("wallet_topup", { agent_id: aid, amount_micro: Math.floor(usd * 1_000_000) });
      setTopupAmt("");
      void refetchWallets();
      void refetchLedger();
    } catch (e) {
      setTopupErr(e instanceof Error ? e.message : String(e));
    } finally {
      setTopupBusy(false);
    }
  };

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
            {/* 💳 결제 지갑 & 온체인 잔액 — keystore master 주소 + Base 체인 실조회 (가짜 값 없음) */}
            <div class="wsec" style="display:flex;align-items:center;gap:8px;">
              💳 결제 지갑 &amp; 온체인 잔액
              <button
                type="button"
                class="budbtn"
                style="margin-left:auto;font-size:11px;padding:4px 10px;"
                onClick={() => void refetchPayWallet()}
                title="온체인 잔액 새로고침"
              >
                ⟳ 새로고침
              </button>
            </div>
            <Show
              when={payWallet() != null}
              fallback={
                <div style="font-size:12px;color:#9aa1ad;margin-bottom:10px;">
                  지갑 정보 준비 중 (데몬 미연결)
                </div>
              }
            >
              <div
                style="border:1px solid var(--kk-line);border-radius:10px;padding:10px 12px;margin-bottom:12px;background:#fafbfc;"
              >
                {/* 주소 + 복사 */}
                <div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;">
                  <span style="font-size:11px;color:#6b7280;">결제 지갑</span>
                  <code
                    style="font-size:12px;font-family:monospace;color:#111;"
                    title={payWallet()?.address ?? ""}
                  >
                    {shortAddr(payWallet()?.address ?? null)}
                  </code>
                  <Show when={payWallet()?.address}>
                    <button
                      type="button"
                      class="budbtn"
                      style="font-size:10.5px;padding:3px 8px;"
                      onClick={() => void onCopyAddr()}
                    >
                      {copied() ? "✓ 복사됨" : "복사"}
                    </button>
                  </Show>
                  <span
                    style={`font-size:10.5px;padding:2px 8px;border-radius:999px;margin-left:auto;${
                      payWallet()?.onchain_enabled
                        ? "background:#e6f4ea;color:#1f7a3d;"
                        : "background:#f1f1f3;color:#7a7f88;"
                    }`}
                    title="XGRAM_CHAIN_RPC 설정 여부"
                  >
                    {payWallet()?.onchain_enabled ? "● 온체인 활성" : "○ 내부 원장 모드"}
                  </span>
                </div>
                <div style="font-size:10.5px;color:#9aa1ad;margin-top:4px;">
                  체인 {payWallet()?.chain ?? "base"}
                  {payWallet()?.rpc_url ? ` · ${payWallet()?.rpc_url}` : ""}
                </div>
                {/* 온체인 잔액 — RPC 실패 시 '조회 실패' (가짜 0 금지) */}
                <div style="display:flex;gap:20px;margin-top:10px;">
                  <div>
                    <div style="font-size:16px;font-weight:600;color:#111;">
                      {payWallet()?.eth_balance != null
                        ? `${fmtEth(payWallet()?.eth_balance ?? null)} ETH`
                        : "조회 실패"}
                    </div>
                    <div class="cap">온체인 ETH (가스)</div>
                  </div>
                  <div>
                    <div style="font-size:16px;font-weight:600;color:#1f7a3d;">
                      {payWallet()?.usdc_balance != null
                        ? `$${fmtUsdcStr(payWallet()?.usdc_balance ?? null)} USDC`
                        : "조회 실패"}
                    </div>
                    <div class="cap">온체인 USDC (결제)</div>
                  </div>
                </div>
                <Show when={payWallet()?.error}>
                  <div style="font-size:11px;color:#d64545;margin-top:8px;">
                    온체인 조회 오류: {payWallet()?.error}
                  </div>
                </Show>
              </div>
            </Show>

            {/* 잔액·누적 구매 — 실제 wallets_list + wallet_ledger 배선 (내부 원장 예산) */}
            <div class="wsec">내부 예산 (원장) — 잔액 / 누적 구매</div>
            <div class="budtop">
              <div>
                <Show
                  when={wallets() != null}
                  fallback={<div class="big" style="color:#9aa1ad;">준비 중</div>}
                >
                  <div class="big">${fmtUsdc(totalBalanceMicro(wallets()))}</div>
                </Show>
                <div class="cap">잔액 (서브 지갑 합 + 마스터 free)</div>
              </div>
              <div>
                <Show
                  when={ledger() != null}
                  fallback={<div class="big" style="font-size:17px;color:#9aa1ad;">준비 중</div>}
                >
                  <div class="big" style="font-size:17px;color:#c9760e;">
                    ${fmtUsdc(ledger()?.total_purchase_micro ?? 0)}
                  </div>
                </Show>
                <div class="cap">누적 구매 (차감)</div>
              </div>
              <div style="margin-left:auto;display:flex;gap:6px;align-items:center;">
                <input
                  type="number"
                  step="0.01"
                  min="0"
                  value={topupAmt()}
                  placeholder="충전액 (USDC)"
                  onInput={(e) => setTopupAmt(e.currentTarget.value)}
                  style="width:110px;border:1px solid var(--kk-line);border-radius:8px;padding:7px 9px;font-size:12px;"
                />
                <button
                  type="button"
                  class="budbtn"
                  disabled={!topupAmt() || topupBusy() || !firstAgentId()}
                  title={firstAgentId() ? "마스터 → 서브 지갑 충전" : "서브 지갑 없음 — 먼저 지갑 생성"}
                  onClick={() => void onTopup()}
                >
                  {topupBusy() ? "충전 중…" : "충전"}
                </button>
              </div>
            </div>
            <Show when={topupErr()}>
              <div style="font-size:11.5px;color:#d64545;margin:2px 0 8px;">충전 실패: {topupErr()}</div>
            </Show>

            {/* 마켓 (d)갈래 — free-tier 무료 할당량 (실제 free_tier_status/config 배선) */}
            <div class="wsec">무료 할당량 — 실제 연결 (free-tier API)</div>
            <div class="budrow">
              <div class="ava c-primary">🎁</div>
              <div>
                <div class="bn">무료 잔여 (오늘)</div>
                <div class="cap" style="font-size:11px;color:#9aa1ad;">
                  전역 기본 · 무료 소진 시 지갑 차감
                </div>
              </div>
              <div class="budamt">
                <Show
                  when={!freeTier.loading}
                  fallback={<div class="u">불러오는 중…</div>}
                >
                  <Show
                    when={freeTier()}
                    fallback={<div class="u" style="color:#9aa1ad;">준비 중</div>}
                  >
                    <div class="u">
                      {freeTier()!.remaining} / {freeTier()!.free_per_day}
                    </div>
                    <div class="l">회 · 사용 {freeTier()!.used_today}</div>
                  </Show>
                </Show>
              </div>
            </div>
            <div style="display:flex;gap:6px;margin:10px 0 4px;align-items:center;">
              <input
                type="number"
                step="1"
                min="0"
                value={ftDraft()}
                placeholder="무료 횟수/일 (전역 기본)"
                onInput={(e) => setFtDraft(e.currentTarget.value)}
                style="flex:1;border:1px solid var(--kk-line);border-radius:8px;padding:8px 10px;font-size:12.5px;"
              />
              <button
                type="button"
                class="budbtn alt"
                disabled={ftDraft() === "" || ftSaving()}
                onClick={() => void onSaveFreeTier()}
              >
                {ftSaving() ? "저장 중…" : "무료 한도 변경"}
              </button>
            </div>
            <Show when={ftErr()}>
              <div style="font-size:11.5px;color:#d64545;margin-bottom:8px;">무료 한도 변경 실패: {ftErr()}</div>
            </Show>
            <div style="font-size:11px;color:#9aa1ad;margin:2px 0 14px;">
              무료 잔여가 있으면 구매가 과금 없이 통과되고, 소진되면 지갑에서 차감됩니다.
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

            {/* 충전·사용 내역 — 실제 wallet_ledger 배선 */}
            <div class="wsec" style="margin-top:20px;">충전 · 사용 내역</div>
            <Show
              when={ledger() != null}
              fallback={
                <div class="mk-note">
                  거래내역 백엔드 미연결 — daemon 연결 후 표시됩니다.
                </div>
              }
            >
              <Show
                when={(ledger()?.entries.length ?? 0) > 0}
                fallback={
                  <div class="mk-note">
                    아직 거래내역이 없습니다. 위에서 충전하거나 마켓에서 구매하면 여기에 기록됩니다.
                  </div>
                }
              >
                <For each={ledger()?.entries ?? []}>
                  {(e) => (
                    <div class="txn">
                      <div class={`ti ${e.amount_micro >= 0 ? "in" : "out"}`}>
                        {e.amount_micro >= 0 ? "↓" : "↑"}
                      </div>
                      <div>
                        <div class="tn">
                          {KIND_LABEL[e.kind] ?? e.kind}
                          {e.memo ? ` · ${e.memo}` : ""}
                        </div>
                        <div class="ts">
                          <span class="tu">{e.agent_id}</span> · {relTime(e.created_at)}
                        </div>
                      </div>
                      <div class={`ta ${e.amount_micro >= 0 ? "plus" : ""}`}>
                        {e.amount_micro >= 0 ? "+" : "-"}${fmtUsdc(Math.abs(e.amount_micro))}
                      </div>
                    </div>
                  )}
                </For>
              </Show>
            </Show>
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
                <Show
                  when={ledger() != null}
                  fallback={<div class="big" style="color:#9aa1ad;">준비 중</div>}
                >
                  <div class="big" style="color:#1f9d4d;">
                    ${fmtUsdc(ledger()?.total_earned_micro ?? 0)}
                  </div>
                </Show>
                <div class="cap">누적 수익 (내부 원장)</div>
              </div>
              <div>
                <Show
                  when={wallets() != null}
                  fallback={<div class="big" style="font-size:17px;color:#9aa1ad;">준비 중</div>}
                >
                  <div class="big" style="font-size:17px;">
                    ${fmtUsdc((wallets()?.sub_wallets ?? []).reduce((a, s) => a + (s.earned_micro || 0), 0))}
                  </div>
                </Show>
                <div class="cap">지갑 적립 합</div>
              </div>
              <div style="margin-left:auto;">
                <button
                  type="button"
                  class="budbtn"
                  disabled
                  title="정산(온체인 USDC 출금)은 funded wallet/RPC 배선 후 — 현재는 내부 원장 적립만"
                >
                  정산하기 (USDC)
                </button>
              </div>
            </div>

            <div class="mk-note">
              수익은 내부 지갑 원장(<b>wallet_ledger</b>)의 실데이터입니다. 외부 USDC 출금(정산)은
              funded wallet + chain RPC 배선 후 활성화됩니다 (현재는 내부 원장 적립까지).
            </div>

            {/* 실제 수익(earn) 원장 내역 */}
            <Show when={(ledger()?.entries.filter((e) => e.kind === "earn").length ?? 0) > 0}>
              <div class="wsec" style="margin-top:14px;">수익 적립 내역 (실데이터)</div>
              <For each={ledger()?.entries.filter((e) => e.kind === "earn") ?? []}>
                {(e) => (
                  <div class="txn">
                    <div class="ti in">↓</div>
                    <div>
                      <div class="tn">{e.memo ?? "외부 사용 수익"}</div>
                      <div class="ts">
                        <span class="tu">{e.counterparty ?? e.agent_id}</span> · {relTime(e.created_at)}
                      </div>
                    </div>
                    <div class="ta plus">+${fmtUsdc(Math.abs(e.amount_micro))}</div>
                  </div>
                )}
              </For>
            </Show>

            <div class="mk-note" style="margin-top:10px;">
              아래는 정본 레이아웃 예시(공개 에이전트 카드·외부 사용 데모)입니다. 위 수치·적립 내역만
              라이브 데이터입니다.
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
