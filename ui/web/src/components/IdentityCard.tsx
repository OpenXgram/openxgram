import { createResource, For, Show } from "solid-js";
import { invoke } from "@/api/client";

// UI-IDENTITY-SPEC v1.0 §3 — 🔑 신원 카드 (PRD §0 #5: 자기주권 신원).
// 4 구역: 잠금 상태 / DID / 마스터 지갑 / 외부 DID allowlist + 고급 메뉴.

interface WhoamiDto {
  alias?: string;
  address?: string;
  hostname?: string;
  did?: string;
}
interface StatusDto {
  initialized: boolean;
  alias?: string;
  address?: string;
}

async function fetchStatus(): Promise<StatusDto | null> {
  try { return await invoke<StatusDto>("status"); } catch { return null; }
}

export function IdentityCard(props: { onBack: () => void }) {
  const [s] = createResource(fetchStatus);

  function lockNow() {
    try {
      localStorage.removeItem("xgram_session_token");
      location.reload();
    } catch {}
  }

  const did = () => {
    const a = s()?.address;
    return a ? `did:openxgram:${a}` : "(로딩 중…)";
  };

  return (
    <div class="card-page">
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">🔑</span>
        <h1>신원</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #5 — 자기주권 신원</div>
      <div class="card-page-oneline">
        secp256k1 키스토어 · DID · BIP39/BIP44 HD · 마스터 지갑 · single-user lock · 외부 DID allowlist · 인증 감사
      </div>

      <section class="card-section">
        <h3>🛡️ 잠금 상태</h3>
        <div class="card-section-row">
          <span class="label">상태</span>
          <span class="value">잠금 해제됨 (session_token 활성)</span>
        </div>
        <div class="card-section-row">
          <span class="label">자동 잠금</span>
          <span class="value">M-2 — 사용자 설정 (기본 30분, daemon 재시작 시 무효)</span>
        </div>
        <button class="link-btn" onClick={lockNow}>🔒 지금 잠금</button>
        <p class="placeholder-note">M-2 자동 잠금 시간 편집 UI · M-8 비밀번호 실패 잠금 (5회 → 1분 backoff) — Phase 2</p>
      </section>

      <section class="card-section">
        <h3>🆔 내 신분 (DID)</h3>
        <div class="card-section-row">
          <span class="label">alias</span>
          <span class="value">{s()?.alias || "(로딩 중…)"}</span>
        </div>
        <div class="card-section-row">
          <span class="label">DID</span>
          <span class="value mono">{did()}</span>
        </div>
        <div class="card-section-row">
          <span class="label">address</span>
          <span class="value mono">{s()?.address || "—"}</span>
        </div>
        <p class="placeholder-note">머신별 서브 DID (zalman·gcp·...) · QR 코드 · did:key export — Phase 2</p>
      </section>

      <section class="card-section">
        <h3>💰 마스터 지갑</h3>
        <div class="card-section-row">
          <span class="label">총 잔액</span>
          <span class="value">— USDC (백엔드 API 미구현)</span>
        </div>
        <div class="card-section-row">
          <span class="label">할당됨</span>
          <span class="value">— ($0 / 0 에이전트)</span>
        </div>
        <div class="card-section-row">
          <span class="label">사용가능</span>
          <span class="value">—</span>
        </div>
        <p class="placeholder-note">
          M-5 자동 분배 정책 (기본 $5/에이전트) · V-8 입금 QR · V-9 출금 — Phase 2.
          백엔드 `GET /v1/gui/identity/wallet/master` 신설 필요.
        </p>
      </section>

      <section class="card-section">
        <h3>🛡️ 외부 호출 허용 목록 (M-4 V-7)</h3>
        <div class="card-section-row">
          <span class="label">기본 정책</span>
          <span class="value">default-deny (안티패턴 #6 준수)</span>
        </div>
        <p class="placeholder-note">
          M-4 3-가지 추가 경로 (마켓 게이트웨이 자동 / 개별 DID 추가 / 요청 큐) · V-7 즉시 적용 — Phase 2.
          백엔드 `GET·POST /v1/gui/identity/allowlist` 필요.
        </p>
      </section>

      <section class="card-section">
        <h3>⚙️ 고급 메뉴</h3>
        <button class="link-btn">🔐 BIP39 백업 단어 보기 (M-3)</button>
        <button class="link-btn">🔄 비밀번호 변경</button>
        <button class="link-btn">🆔 키 교체 / 새 DID 발급 (M-15)</button>
        <button class="link-btn">📋 인증 감사 로그 (M-7)</button>
        <button class="link-btn">🖥️ 머신 등록 (M-14)</button>
        <button class="link-btn">📥 키스토어 복구</button>
        <p class="placeholder-note">
          각 기능은 Phase 2. 사양 = UI-IDENTITY-SPEC v1.0 §4~§8.
        </p>
      </section>
    </div>
  );
}
