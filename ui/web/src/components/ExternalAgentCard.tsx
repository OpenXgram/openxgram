// UI-EXTERNAL-AGENT-SPEC v1.0 — 🌐 외부 에이전트·결제 (PRD §0 #3).
// 사양 작성 예정. UI-CARDS-IDENTITY v1.1 §2.3 책임 기반 placeholder.

import { Breadcrumb } from "./Breadcrumb";

export function ExternalAgentCard(props: { onBack: () => void }) {
  return (
    <div class="card-page">
      <Breadcrumb cardName="🌐 외부 에이전트" onReturn={props.onBack} />
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">🌐</span>
        <h1>외부 에이전트</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #3 — 외부 에이전트·결제</div>
      <div class="card-page-oneline">
        다른 AI 시스템 (OpenAgentX·A2A·ANP·x402·Virtuals ACP) 과의 거래 게이트웨이. 대화는 메신저, 거래·계약·정산은 여기.
      </div>

      <section class="card-section">
        <h3>📚 외부 디렉토리</h3>
        <p class="placeholder-note">OpenAgentX 마켓 · ANP discovery · A2A registry 통합. 시멘틱·평점·가격으로 외부 에이전트 찾기.</p>
      </section>

      <section class="card-section">
        <h3>📤 아웃바운드 호출 이력</h3>
        <p class="placeholder-note">내가 누구한테 보냈는지 · 결과 · 평점 · 계약 단위 (메시지 단위와 다른 차원).</p>
      </section>

      <section class="card-section">
        <h3>📥 인바운드 승인 큐</h3>
        <p class="placeholder-note">외부 AI 가 내 에이전트 호출 시. 사용자 승인 → 대화 자체는 💬 메신저로 인계.</p>
      </section>

      <section class="card-section">
        <h3>🏪 내 마켓 listing</h3>
        <p class="placeholder-note">내 에이전트를 OpenAgentX 마켓에 등록 (Cognac 수익). 가격 · 소개 · 통계.</p>
      </section>

      <section class="card-section">
        <h3>⚙️ 프로토콜 설정</h3>
        <p class="placeholder-note">A2A · ANP · x402 · Virtuals ACP enable/disable. 토큰은 🗝️ Vault.</p>
      </section>

      <section class="card-section">
        <h3>⭐ 평판·track record</h3>
        <p class="placeholder-note">외부 에이전트별 평점 · 블랙리스트. 외부 DID allowlist(보안) 는 🔑 신원 카드.</p>
      </section>

      <p class="placeholder-note" style="margin-top:16px;">
        <strong>사양 문서 = UI-EXTERNAL-AGENT-SPEC-v1.0.md (작성 예정)</strong>. 본 화면은 정체성·책임 placeholder.
      </p>
    </div>
  );
}
