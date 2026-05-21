// UI-OPS-SPEC v1.0 — ⚙️ 운영·생존 (PRD §0 #7) — 깊은 구현.

import { createResource, For, Show } from "solid-js";
import { invoke } from "@/api/client";
import { Breadcrumb } from "./Breadcrumb";

async function fetchHealth(): Promise<any> { try { return await invoke("ops_health"); } catch { return null; } }
async function fetchQ(): Promise<any> { try { return await invoke("cross_machine_queue"); } catch { return null; } }
async function fetchDiag(): Promise<any> { try { return await invoke("ops_diagnostic"); } catch (e) { return { error: String(e) }; } }
async function fetchMachines(): Promise<any> { try { return await invoke("ops_machines"); } catch (e) { return { error: String(e) }; } }
async function fetchBackup(): Promise<any> { try { return await invoke("ops_backup_status"); } catch (e) { return { error: String(e) }; } }
async function fetchUpdate(): Promise<any> { try { return await invoke("ops_update_check"); } catch (e) { return { error: String(e) }; } }

export function OpsCard(props: { onBack: () => void }) {
  const [health] = createResource(fetchHealth);
  const [q] = createResource(fetchQ);
  const [diag, { refetch: refDiag }] = createResource(fetchDiag);
  const [mach, { refetch: refMach }] = createResource(fetchMachines);
  const [backup, { refetch: refBackup }] = createResource(fetchBackup);
  const [upd, { refetch: refUpd }] = createResource(fetchUpdate);
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
        <h3>🩺 자가 진단 <button class="link-btn" onClick={() => refDiag()}>↻</button></h3>
        <Show when={diag()}>
          <div class="card-section-row"><span class="label">요약</span><span class="value">{diag()?.summary || diag()?.error}</span></div>
          <div class="card-section-row"><span class="label">DB OK</span><span class="value">{diag()?.db?.ok ? "✓" : "✗"} · sessions={diag()?.db?.sessions} · messages={diag()?.db?.messages} · migration v{diag()?.db?.migration_version}</span></div>
          <div class="card-section-row"><span class="label">keystore master</span><span class="value">{diag()?.keystore?.master_exists ? "✓ 존재" : "✗ 없음"}</span></div>
          <div class="card-section-row"><span class="label">data_dir</span><span class="value mono">{diag()?.disk?.data_dir}</span></div>
          <div class="card-section-row"><span class="label">services</span><span class="value mono" style="font-size:11px;">{JSON.stringify(diag()?.services)}</span></div>
        </Show>
      </section>

      <section class="card-section">
        <h3>💻 머신 (Tailscale peer + DID) <button class="link-btn" onClick={() => refMach()}>↻</button></h3>
        <Show when={mach()}>
          <div class="card-section-row"><span class="label">현재 머신</span><span class="value">{mach()?.local_machine?.alias} ({mach()?.local_machine?.hostname})</span></div>
          <div class="card-section-row"><span class="label">Tailscale IP</span><span class="value mono">{mach()?.local_machine?.tailscale_ip || "—"}</span></div>
          <div class="card-section-row"><span class="label">등록 peer</span><span class="value">{mach()?.peer_count} 명</span></div>
          <For each={mach()?.registered_peers ?? []}>
            {(p: any) => (
              <div style="font-size:11px; padding:4px 0; border-bottom:1px solid var(--border);">
                <strong>{p.alias}</strong> ({p.role}) · <code>{p.address?.slice(0, 12)}...</code> · last={p.last_seen}
              </div>
            )}
          </For>
          <Show when={Object.keys(mach()?.tailscale_peers ?? {}).length > 0}>
            <div style="font-size:11px; color:var(--text-3); margin-top:6px;">Tailscale peer: {Object.keys(mach()?.tailscale_peers ?? {}).length} 명</div>
          </Show>
        </Show>
      </section>

      <section class="card-section">
        <h3>🖥️ Daemon 상태</h3>
        <Show when={health()}>
          <div class="card-section-row"><span class="label">release</span><span class="value">{health()?.version?.release}</span></div>
          <div class="card-section-row"><span class="label">daemon</span><span class="value">{health()?.version?.daemon}</span></div>
          <div class="card-section-row"><span class="label">사양</span><span class="value">{health()?.version?.spec_doc}</span></div>
          <div class="card-section-row"><span class="label">PRD</span><span class="value">{health()?.version?.prd_doc}</span></div>
          <div class="card-section-row"><span class="label">GUI 호스팅</span><span class="value">{health()?.gui_hosting}</span></div>
          <div class="card-section-row"><span class="label">자동 업데이트</span><span class="value">{health()?.auto_update_channel}</span></div>
        </Show>
      </section>

      <section class="card-section">
        <h3>🔄 자동 업데이트 <button class="link-btn" onClick={() => refUpd()}>↻</button></h3>
        <Show when={upd()}>
          <div class="card-section-row"><span class="label">현재 release</span><span class="value mono">{upd()?.current?.release}</span></div>
          <div class="card-section-row"><span class="label">최신 release</span><span class="value mono">{upd()?.latest_tag || "조회 실패"}</span></div>
          <div class="card-section-row"><span class="label">최신 여부</span><span class="value">{upd()?.up_to_date ? "✓ 최신" : "⚠ 업데이트 가능"}</span></div>
          <div class="card-section-row"><span class="label">채널</span><span class="value">{upd()?.channel}</span></div>
          <Show when={!upd()?.up_to_date && upd()?.update_url}>
            <a href={upd()?.update_url} target="_blank" style="color:#06c; font-size:12px;">→ 업데이트 다운로드</a>
          </Show>
        </Show>
      </section>

      <section class="card-section">
        <h3>💾 백업·복원 <button class="link-btn" onClick={() => refBackup()}>↻</button></h3>
        <Show when={backup()}>
          <div class="card-section-row"><span class="label">백업 dir</span><span class="value mono">{backup()?.backup_dir}</span></div>
          <div class="card-section-row"><span class="label">백업 파일</span><span class="value">{backup()?.count} 개</span></div>
          <div class="card-section-row"><span class="label">마지막</span><span class="value">{backup()?.last_at || "—"}</span></div>
          <p style="font-size:11px; color:var(--text-3);">{backup()?.note}</p>
          <For each={backup()?.backup_files ?? []}>
            {(f: any) => <div style="font-size:11px;">📦 {f.name}</div>}
          </For>
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
    </div>
  );
}
