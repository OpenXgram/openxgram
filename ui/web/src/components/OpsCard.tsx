// UI-OPS-SPEC v1.0 — ⚙️ 운영·생존 (PRD §0 #7).

import { createResource, Show } from "solid-js";
import { invoke } from "@/api/client";
import { Breadcrumb } from "./Breadcrumb";

async function fetchHealth(): Promise<any> { try { return await invoke("ops_health"); } catch { return null; } }
async function fetchQ(): Promise<any> { try { return await invoke("cross_machine_queue"); } catch { return null; } }

export function OpsCard(props: { onBack: () => void }) {
  const [health] = createResource(fetchHealth);
  const [q] = createResource(fetchQ);
  return (
    <div class="card-page">
      <Breadcrumb cardName="⚙️ 운영·생존" onReturn={props.onBack} />
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
        <h3>🖥️ Daemon 상태 (ops/health)</h3>
        <Show when={health()}>
          <div class="card-section-row"><span class="label">release</span><span class="value">{health()?.version?.release}</span></div>
          <div class="card-section-row"><span class="label">daemon</span><span class="value">{health()?.version?.daemon}</span></div>
          <div class="card-section-row"><span class="label">사양</span><span class="value">{health()?.version?.spec_doc}</span></div>
          <div class="card-section-row"><span class="label">PRD</span><span class="value">{health()?.version?.prd_doc}</span></div>
          <div class="card-section-row"><span class="label">머신 alias</span><span class="value">{health()?.machine?.alias}</span></div>
          <div class="card-section-row"><span class="label">Tailscale IP</span><span class="value mono">{health()?.machine?.tailscale_ip}</span></div>
          <div class="card-section-row"><span class="label">GUI 호스팅</span><span class="value">{health()?.gui_hosting}</span></div>
          <div class="card-section-row"><span class="label">백업 last</span><span class="value">{health()?.backup?.last_at || "—"}</span></div>
          <div class="card-section-row"><span class="label">자동 업데이트</span><span class="value">{health()?.auto_update_channel}</span></div>
          <div class="card-section-row"><span class="label">DB OK</span><span class="value">{health()?.self_check?.db_ok ? "✓" : "✗"}</span></div>
        </Show>
      </section>
      <section class="card-section">
        <h3>🌐 Cross-machine 큐 (S8 + V6)</h3>
        <Show when={q()}>
          <div class="card-section-row"><span class="label">backend</span><span class="value">{q()?.backend}</span></div>
          <div class="card-section-row"><span class="label">queue path</span><span class="value mono">{q()?.queue_path}</span></div>
          <div class="card-section-row"><span class="label">보관</span><span class="value">{q()?.max_retention_days} 일</span></div>
          <div class="card-section-row"><span class="label">backoff</span><span class="value">{q()?.retry_backoff}</span></div>
          <div class="card-section-row"><span class="label">dedup</span><span class="value">{q()?.dedup_strategy}</span></div>
          <div class="card-section-row"><span class="label">pending</span><span class="value">{q()?.pending}</span></div>
        </Show>
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
