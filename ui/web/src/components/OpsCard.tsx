// UI-OPS-SPEC v1.0 — ⚙️ 운영·생존 (PRD §0 #7).
// 사양 문서 작성 예정. UI-CARDS-IDENTITY v1.1 §2.7 책임 기반 placeholder.

export function OpsCard(props: { onBack: () => void }) {
  return (
    <div class="card-page">
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">⚙️</span>
        <h1>운영·생존</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #7 — 운영·생존</div>
      <div class="card-page-oneline">
        OpenXgram 자체의 호스팅 · daemon · 머신 · 백업 · 업데이트 · 자가 진단
      </div>

      <section class="card-section">
        <h3>🖥️ Daemon 상태</h3>
        <p class="placeholder-note">버전 · pid · uptime · 메모리 · 마지막 재시작 사유.</p>
      </section>

      <section class="card-section">
        <h3>🌐 GUI 호스팅</h3>
        <p class="placeholder-note">Tailscale Funnel default · Cloudflare · 자체 nginx. 결정 11 (PRD §9).</p>
      </section>

      <section class="card-section">
        <h3>💻 머신 (4-tuple address 의 machine part)</h3>
        <p class="placeholder-note">현재 머신 ID · 클러스터 동기화 · WireGuard 메시 (Phase 2).</p>
      </section>

      <section class="card-section">
        <h3>💾 백업·복원</h3>
        <p class="placeholder-note">암호화 백업 (BIP39 마스터 키 필요) · GitHub Gist / 사용자 GCP / 외장 디스크.</p>
      </section>

      <section class="card-section">
        <h3>🔄 자동 업데이트</h3>
        <p class="placeholder-note">stable / beta 채널 · 보안 패치 자동 / 메이저 사용자 승인.</p>
      </section>

      <section class="card-section">
        <h3>🩺 자가 진단</h3>
        <p class="placeholder-note">DB 무결성 · keystore 잠금 상태 · 디스크 · CPU · 의존 서비스 (Tailscale·MCP) 헬스체크.</p>
      </section>

      <p class="placeholder-note" style="margin-top:16px;">
        <strong>사양 문서 = UI-OPS-SPEC-v1.0.md (작성 예정)</strong>. 본 화면은 정체성·책임 placeholder.
      </p>
    </div>
  );
}
