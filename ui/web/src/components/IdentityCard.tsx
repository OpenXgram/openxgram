import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";
import { Breadcrumb} from "./Breadcrumb";

// UI-IDENTITY-SPEC v1.0 §3 — 신원 카드 (PRD §0 #5: 자기주권 신원).
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
 try { return await invoke<StatusDto>("status");} catch { return null;}
}
async function fetchInfo(): Promise<any> { try { return await invoke("identity_info");} catch { return null;}}
async function fetchAudit(): Promise<any[]> { try { return await invoke("identity_audit");} catch { return [];}}
async function fetchAllowlist(): Promise<any> { try { return await invoke("identity_allowlist");} catch { return null;}}
async function fetchSubDids(): Promise<any[]> { try { return await invoke("identity_sub_dids");} catch { return [];}}
async function fetchLockout(): Promise<any> { try { return await invoke("identity_lockout_status");} catch { return null;}}
async function fetchSuspicious(): Promise<any[]> { try { return await invoke("identity_suspicious_dids");} catch { return [];}}

export function IdentityCard(props: { onBack: () => void}) {
 const [s] = createResource(fetchStatus);
 const [info] = createResource(fetchInfo);
 const [audit] = createResource(fetchAudit);
 const [allowlist, { refetch: refetchAllow}] = createResource(fetchAllowlist);
 const [subDids, { refetch: refetchSub}] = createResource(fetchSubDids);
 const [lockout] = createResource(fetchLockout);
 const [suspicious, { refetch: refetchSusp}] = createResource(fetchSuspicious);
 const [bip39, setBip39] = createSignal<string[] | null>(null);
 const [newDid, setNewDid] = createSignal("");
 const [newMachine, setNewMachine] = createSignal("");
 const [autoLockEdit, setAutoLockEdit] = createSignal<number | null>(null);
 async function saveAutoLock() {
 const v = autoLockEdit();
 if (v == null || v < 1 || v > 1440) { alert("1~1440 분 사이"); return;}
 try {
 await invoke("identity_settings", { auto_lock_minutes: v});
 setAutoLockEdit(null);
 alert("저장됨. 새로고침하면 반영됩니다.");
 } catch (e) { alert(String(e));}
 }
 async function dismissSusp(id: number) {
 try { await invoke("identity_suspicious_dismiss", { id}); await refetchSusp();} catch (e) { alert(String(e));}
 }
 async function showBip39() {
 try { const r: any = await invoke("identity_bip39", {}); setBip39(r.words); setTimeout(() => setBip39(null), 30000);} catch (e) { alert(String(e));}
}
 async function addSubDid() {
 if (!newMachine()) return;
 try { await invoke("identity_sub_did_new", { machine: newMachine()}); setNewMachine(""); await refetchSub();} catch (e) { alert(String(e));}
}
 async function revokeSub(id: string) {
 if (!confirm(`${id} revoke? 영구 (M-15)`)) return;
 try { await invoke("identity_sub_did_revoke", { id}); await refetchSub();} catch (e) { alert(String(e));}
}
 async function addAllow() {
 if (!newDid()) return;
 try { await invoke("identity_allowlist_add", { external_did: newDid(), note: ""}); setNewDid(""); await refetchAllow();} catch {}
}

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
 <Breadcrumb cardName=" 신원" onReturn={props.onBack} />
 <button class="card-page-back" onClick={props.onBack}>← 홈</button>
 <div class="card-page-head">
 <span class="icon"></span>
 <h1>신원</h1>
 </div>
 <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #5 — 자기주권 신원</div>
 <div class="card-page-oneline">
 secp256k1 키스토어 · DID · BIP39/BIP44 HD · 마스터 지갑 · single-user lock · 외부 DID allowlist · 인증 감사
 </div>

 <section class="card-section">
 <h3> 잠금 상태</h3>
 <div class="card-section-row">
 <span class="label">상태</span>
 <span class="value">잠금 해제됨 (session_token 활성)</span>
 </div>
 <div class="card-section-row">
 <span class="label">자동 잠금</span>
 <span class="value">M-2 — 사용자 설정 (기본 30분, daemon 재시작 시 무효)</span>
 </div>
 <button class="link-btn" onClick={lockNow}> 지금 잠금</button>
 <p class="placeholder-note">M-2 자동 잠금 시간 편집 UI · M-8 비밀번호 실패 잠금 (5회 → 1분 backoff) — Phase 2</p>
 </section>

 <section class="card-section">
 <h3> 내 신분 (DID)</h3>
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
 <h3> 마스터 지갑</h3>
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
 <h3> 외부 호출 허용 목록 (M-4 V-7)</h3>
 <div class="card-section-row">
 <span class="label">기본 정책</span>
 <span class="value">{allowlist()?.policy ?? "default-deny (N9)"}</span>
 </div>
 <div class="card-section-row">
 <span class="label">마켓 게이트웨이 자동 신뢰</span>
 <span class="value">{allowlist()?.marketplace_gateway_auto_trusted ? "" : ""}</span>
 </div>
 <div class="card-section-row">
 <span class="label">세션 override (V9)</span>
 <span class="value">{allowlist()?.session_override ? "허용" : "불가 (마스터 1개)"}</span>
 </div>
 <Show when={(allowlist()?.entries ?? []).length === 0} fallback={null}>
 <p style="font-size:12px; color:var(--text-3);">등록된 외부 DID 없음.</p>
 </Show>
 <For each={allowlist()?.entries ?? []}>{(e: any) => (
 <div class="card-section-row"><span class="label">{e.external_did}</span><span class="value">{e.note}</span></div>
)}</For>
 <div style="display:flex; gap:6px; margin-top:8px;">
 <input value={newDid()} onInput={(e) => setNewDid(e.currentTarget.value)} placeholder="did:openxgram:0x..."
 style="flex:1; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <button class="link-btn" onClick={addAllow}>+ 추가</button>
 </div>
 </section>

 <section class="card-section">
 <h3> 인증 감사 로그 (M-7 영구)</h3>
 <Show when={(audit() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">감사 로그 없음.</p>
 </Show>
 <For each={(audit() ?? []).slice(0, 20)}>{(e: any) => (
 <div style="font-size:11px; padding:4px 0; border-bottom:1px solid var(--border);">
 <span style="color:var(--text-3);">{e.created_at}</span> · <strong>{e.event_type}</strong>
 </div>
)}</For>
 </section>

 <section class="card-section">
 <h3> BIP39 백업 단어 (M-3 V-3)</h3>
 <button class="link-btn" onClick={showBip39}> 보기 (30초 후 자동 숨김 — 스크린샷 금지)</button>
 <Show when={bip39()}>
 <div style="background:var(--surface-2); padding:10px; border-radius:4px; margin-top:8px; font-family:monospace;">
 {bip39()!.join(" ")}
 </div>
 <p style="color:#f88; font-size:11px;"> 적었음 확인 후 닫으세요. 30초 후 자동 숨김.</p>
 </Show>
 </section>

 <section class="card-section">
 <h3> 머신 sub-DID (M-9 V-12)</h3>
 <div style="display:flex; gap:6px; margin-bottom:8px;">
 <input value={newMachine()} onInput={(e) => setNewMachine(e.currentTarget.value)} placeholder="머신 alias (zalman / macmini / gcp)"
 style="flex:1; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;" />
 <button class="link-btn" onClick={addSubDid}>+ sub-DID 발급</button>
 </div>
 <For each={subDids() ?? []}>{(s: any) => (
 <div style="display:flex; justify-content:space-between; padding:6px 0; border-bottom:1px solid var(--border); font-size:11px;">
 <div>
 <strong>{s.machine}</strong>
 <div class="mono" style="color:var(--text-3);">{s.id}</div>
 </div>
 <Show when={s.status === "Active"} fallback={<span style="color:var(--text-3);">revoked</span>}>
 <button class="link-btn" onClick={() => revokeSub(s.id)}> revoke</button>
 </Show>
 </div>
)}</For>
 </section>

 <section class="card-section">
 <h3>자동 잠금 시간 (M-2)</h3>
 <div class="card-section-row">
 <span class="label">현재</span>
 <span class="value">{info()?.auto_lock_minutes ?? "?"} 분</span>
 </div>
 <div style="display:flex; gap:6px; align-items:center; margin-top:8px;">
 <input
 type="number"
 min="1"
 max="1440"
 value={autoLockEdit() ?? info()?.auto_lock_minutes ?? 30}
 onInput={(e) => setAutoLockEdit(parseInt(e.currentTarget.value))}
 style="width:100px; padding:6px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;"
 />
 <span style="color:var(--text-3); font-size:12px;">분 (1~1440)</span>
 <button class="link-btn" onClick={saveAutoLock}>저장</button>
 </div>
 <p style="font-size:11px; color:var(--text-3); margin-top:6px;">
 DB identity_settings 에 영구 저장. daemon 재시작 후에도 유지.
 </p>
 </section>

 <section class="card-section">
 <h3>해킹 의심 새 DID (M-10)</h3>
 <Show when={(suspicious() ?? []).length === 0}>
 <p style="font-size:12px; color:var(--text-3);">의심 DID 없음. 외부에서 새 DID 가 들어올 때 자동 적재됩니다.</p>
 </Show>
 <For each={suspicious() ?? []}>{(d: any) => (
 <div style="display:flex; justify-content:space-between; align-items:center; padding:6px 0; border-bottom:1px solid var(--border); font-size:11px;">
 <div>
 <div class="mono" style="color:var(--text-1);">{d.external_did}</div>
 <div style="color:var(--text-3);">{d.reason} · {d.first_seen}</div>
 </div>
 <button class="link-btn" onClick={() => dismissSusp(d.id)}>무시</button>
 </div>
 )}</For>
 </section>

 <section class="card-section">
 <h3>백업 안내 (M-6)</h3>
 <p style="font-size:12px; color:var(--text-3);">
 <strong>3 단계 백업 권장</strong>:
 </p>
 <ol style="font-size:12px; color:var(--text-3); padding-left:18px;">
 <li>BIP39 12 단어 (위 섹션) — 종이에 적어서 금고/안전한 곳</li>
 <li>keystore 파일 (<code>~/.openxgram/keystore/master.json</code>) — 외장 SSD 또는 암호화 클라우드</li>
 <li>SQLite DB (<code>~/.openxgram/db.sqlite</code>) — 위키·메모리·감사 로그 복구용</li>
 </ol>
 <p style="font-size:11px; color:#f88; margin-top:4px;">
 자동 백업 cron: <code>xgram backup --to s3://...</code> (Phase 2). 현재는 수동.
 </p>
 </section>

 <section class="card-section">
 <h3>새 머신 등록 (M-14)</h3>
 <p style="font-size:12px; color:var(--text-3);">
 신규 머신(예: 노트북·서버) 에 OpenXgram install 시:
 </p>
 <ol style="font-size:12px; color:var(--text-3); padding-left:18px;">
 <li>새 머신에서 <code>curl -s openxgram.org/install.sh | sh</code></li>
 <li><code>xgram init --restore-from-seed</code> + BIP39 12 단어 입력</li>
 <li>위 sub-DID 섹션에서 머신 alias 등록 (라이브 동기화 시작)</li>
 </ol>
 <p style="font-size:11px; color:var(--text-3);">QR 코드 페어링은 M-12 섹션 참고.</p>
 </section>

 <section class="card-section">
 <h3> DID QR 공유 (M-12)</h3>
 <p style="font-size:12px; color:var(--text-3);">
 내 DID 를 QR 로 공유 (휴대폰 → 카메라 스캔 → 새 머신 등록).
 QR 생성: <code>qrencode -t ANSI256 "did:openxgram:{s()?.address || '...'}"</code> (CLI)
 또는 외부 QR 생성기에 위 DID 입력.
 </p>
 <a href={`https://api.qrserver.com/v1/create-qr-code/?size=200x200&data=did:openxgram:${s()?.address || ''}`} target="_blank" style="color:#06c; font-size:11px;">→ QR 코드 생성 (외부)</a>
 </section>

 <section class="card-section">
 <h3> 비밀번호 복구 (M-13)</h3>
 <p style="font-size:12px; color:var(--text-3);">
 BIP39 12 단어 → 새 비밀번호 설정: <code>xgram keystore restore --from-seed</code> CLI (Phase 2: GUI 마법사).
 </p>
 </section>

 <section class="card-section">
 <h3> 비밀번호 실패 lockout (M-8)</h3>
 <Show when={lockout()}>
 <Row label="최근 1시간 실패" value={String(lockout()?.recent_failures_1h ?? 0)} />
 <Row label="lockout 임계" value={`${lockout()?.lockout_threshold}회`} />
 <Row label="backoff 전략" value={lockout()?.backoff_strategy} />
 <p style="font-size:11px; color:var(--text-3);">{lockout()?.policy}</p>
 </Show>
 </section>

 <section class="card-section">
 <h3> 기술 파라미터 (info endpoint)</h3>
 <Show when={info()}>
 <div class="card-section-row"><span class="label">Argon2id</span><span class="value">m={info()?.argon2?.m} · t={info()?.argon2?.t} · p={info()?.argon2?.p}</span></div>
 <div class="card-section-row"><span class="label">auto_lock</span><span class="value">{info()?.auto_lock_minutes} 분 (M-2)</span></div>
 <div class="card-section-row"><span class="label">session TTL</span><span class="value">{info()?.session_token_ttl_minutes} 분 (V-4)</span></div>
 <div class="card-section-row"><span class="label">HD path</span><span class="value mono">{info()?.hd_path}</span></div>
 </Show>
 </section>
 </div>
);
}
