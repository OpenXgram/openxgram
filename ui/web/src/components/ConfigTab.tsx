import { createSignal, createResource, For, Show } from "solid-js";
import {
  getBearer,
  getDaemonUrl,
  invoke,
  setBearer,
  setDaemonUrl,
} from "../api/client";

// 설정 탭 — 카카오톡 정본 목업(_mockups/kakao-mockup.html) 충실 이식.
// 정본 #settingsOvl 의 .board / .bh("⚙️ 설정 · 계정·머신·연동") / .bb / .wsec / .qbtn 마크업·CSS 를
// 그대로(verbatim) 포팅하고, 샘플 텍스트만 라이브 데이터로 치환. 오버레이 chrome(.ovl/.bx) 은 탭 본문이라 제거.
// 정본 본문이 sparse(.qbtn 행) 하므로 계정·머신은 정본 공유 클래스 .apvgrid/.apvcard/.cfgrow(워크플로 보드와 동일 출처) 로 표현.
//
// 백엔드 contract 재사용(신규 명령 발명 X):
//   status           → { initialized, alias, address }
//   identity_info    → { alias, did, machine, hostname, hd_path, auto_lock_minutes, session_token_ttl_minutes }
//   identity_settings (POST, body.auto_lock_minutes)  ← 편집 가능 필드(자동 잠금)
//   machines_list    → { machines:[{hostname,tailscale_ip,is_local,online,source}], machine_count }
//                       물리 머신만(worker agent 제외). local + tailscale online peer.
//   version_info     → { ... } (버전 표기)
//   daemon URL/token ← localStorage (getDaemonUrl/setDaemonUrl/getBearer/setBearer) — web GUI 전용 설정.

interface StatusDto {
  initialized?: boolean;
  alias?: string;
  address?: string;
}

interface IdentityInfo {
  alias?: string;
  did?: string;
  machine?: string;
  hostname?: string;
  hd_path?: string;
  auto_lock_minutes?: number;
  session_token_ttl_minutes?: number;
}

// machines_list 라우트 row — 물리 머신만(worker agent 제외).
interface MachineRow {
  hostname?: string;
  tailscale_ip?: string | null;
  is_local?: boolean;
  online?: boolean;
  source?: string | null;
}

interface MachinesDto {
  machines?: MachineRow[];
  machine_count?: number;
}

export function ConfigTab() {
  const [status] = createResource<StatusDto | null>(async () => {
    try { return await invoke<StatusDto>("status"); } catch { return null; }
  });
  const [info] = createResource<IdentityInfo | null>(async () => {
    try { return await invoke<IdentityInfo>("identity_info"); } catch { return null; }
  });
  const [machines] = createResource<MachinesDto | null>(async () => {
    try { return await invoke<MachinesDto>("machines_list"); } catch { return null; }
  });
  const [version] = createResource<Record<string, unknown> | null>(async () => {
    try { return await invoke<Record<string, unknown>>("version_info"); } catch { return null; }
  });

  // 데몬 연결 (web GUI 전용 — URL + Bearer). SettingsTab DaemonSection 과 동일 contract.
  const [url, setUrl] = createSignal(getDaemonUrl());
  const [token, setToken] = createSignal(getBearer() ?? "");
  const [connMsg, setConnMsg] = createSignal("");

  // 자동 잠금 (identity_settings 로 영구 저장되는 편집 가능 필드).
  const [autoLock, setAutoLock] = createSignal<number | null>(null);
  const [lockMsg, setLockMsg] = createSignal("");
  // info() 로드되면 초기값 주입 (사용자가 아직 안 건드렸을 때만).
  const lockValue = () => {
    const v = autoLock();
    if (v !== null) return v;
    return info()?.auto_lock_minutes ?? 0;
  };

  function saveConn() {
    setDaemonUrl(url());
    setBearer(token());
    setConnMsg("저장됨 — 다음 요청부터 적용됩니다.");
  }

  async function testConn() {
    setDaemonUrl(url());
    setBearer(token());
    setConnMsg("연결 확인 중…");
    try {
      const r = await invoke<unknown>("status");
      setConnMsg(`연결 OK · ${JSON.stringify(r).slice(0, 90)}`);
    } catch (e) {
      setConnMsg(`연결 실패: ${(e as Error).message}`);
    }
  }

  async function saveAutoLock() {
    const v = lockValue();
    setLockMsg("저장 중…");
    try {
      await invoke("identity_settings", { auto_lock_minutes: v });
      setLockMsg(`자동 잠금 ${v === 0 ? "끔" : `${v}분`} 저장됨.`);
    } catch (e) {
      setLockMsg(`저장 실패: ${(e as Error).message}`);
    }
  }

  // 물리 머신 목록 — 로컬 머신 먼저, 그다음 tailscale peer.
  const machineRows = () => {
    const list = machines()?.machines ?? [];
    return [...list].sort((a, b) => (b.is_local ? 1 : 0) - (a.is_local ? 1 : 0));
  };
  // 로컬 머신은 online 필드가 없음 — 항상 온라인으로 간주.
  const isMachineOnline = (m: MachineRow) => m.is_local === true || m.online === true;

  const versionLabel = () => {
    const v = version();
    if (!v) return "";
    return (v.release as string) || (v.daemon as string) || (v.version as string) || JSON.stringify(v).slice(0, 60);
  };

  return (
    // 정본 .ovl > .board 구조를 탭 본문(.kk-set)으로 인라인화. .board 의 .bh / .bb 그대로.
    <div class="kk-set">
      <div class="board">
        <div class="bh">
          <h2>⚙️ 설정</h2>
          <span class="sub">계정 · 머신 · 연동</span>
          <Show when={versionLabel()}><span class="ver">버전 {versionLabel()}</span></Show>
        </div>
        <div class="bb">
          {/* 계정 · 신원 (정본 .wsec 헤더 + .apvgrid/.apvcard 카드) */}
          <div class="wsec">계정 · 신원</div>
          <Show when={!status.loading || !info.loading} fallback={<div class="kk-set-empty">불러오는 중…</div>}>
            <div class="apvgrid">
              <div class="apvcard"><div class="k">별칭(alias)</div><div class="v">{info()?.alias || status()?.alias || "—"}</div></div>
              <div class="apvcard"><div class="k">주소(address)</div><div class="v">{status()?.address || "—"}</div></div>
              <div class="apvcard"><div class="k">DID</div><div class="v">{info()?.did || "—"}</div></div>
              <div class="apvcard"><div class="k">머신</div><div class="v">{info()?.machine || info()?.hostname || "—"}</div></div>
              <div class="apvcard"><div class="k">HD 경로</div><div class="v">{info()?.hd_path || "—"}</div></div>
              <div class="apvcard"><div class="k">상태</div><div class="v">{status()?.initialized ? "초기화됨" : "미초기화"}</div></div>
            </div>
          </Show>

          {/* 자동 잠금 (편집 가능 — identity_settings 로 영구 저장) */}
          <div class="wsec" style="margin-top:18px;">자동 잠금</div>
          <p class="kk-set-hint">유휴 시 자동으로 잠그기까지의 분(min). 0 = 끔. 세션 토큰 TTL: {info()?.session_token_ttl_minutes ?? "—"}분.</p>
          <div class="kk-set-row">
            <input
              class="kk-set-num"
              type="number"
              min="0"
              value={lockValue()}
              onInput={(e) => setAutoLock(parseInt(e.currentTarget.value, 10) || 0)}
            />
            <span class="kk-set-unit">분</span>
            <button class="kk-set-btn" onClick={() => void saveAutoLock()}>저장</button>
          </div>
          <Show when={lockMsg()}><div class="kk-set-msg">{lockMsg()}</div></Show>

          {/* 연결된 머신 — 물리 머신만(worker agent 제외, machines_list). 정본 .wsec + .cfgrow 행. */}
          <div class="wsec" style="margin-top:18px;">연결된 머신 <span class="auto">(물리 머신 · 자동 탐지)</span></div>
          <Show when={!machines.loading} fallback={<div class="kk-set-empty">불러오는 중…</div>}>
            <Show
              when={machineRows().length > 0}
              fallback={<div class="kk-set-empty">연결된 머신 정보가 없습니다. (데몬 machines_list 응답 비어있음)</div>}
            >
              <For each={machineRows()}>
                {(m) => (
                  <div class="cfgrow">
                    <span class="cfi">💻</span>
                    <div>
                      <div class="cfp">
                        <span class={`mdot${isMachineOnline(m) ? " on" : ""}`} />
                        {m.hostname || "—"}
                        <Show when={m.is_local}><span class="mbadge">이 머신</span></Show>
                      </div>
                      <div class="cfc">
                        <Show when={m.tailscale_ip} fallback="Tailscale IP 미설정">
                          {m.tailscale_ip}
                        </Show>
                        <Show when={m.source}> · {m.source}</Show>
                      </div>
                    </div>
                    <span class="cfx">{isMachineOnline(m) ? "온라인" : "오프라인"}</span>
                  </div>
                )}
              </For>
            </Show>
          </Show>

          {/* 데몬 연결 (web GUI 전용) */}
          <div class="wsec" style="margin-top:18px;">데몬 연결 <span class="auto">(이 브라우저)</span></div>
          <p class="kk-set-hint">웹 GUI 가 접속할 데몬 주소와 인증 토큰. 이 브라우저에만 저장됩니다.</p>
          <label class="kk-set-label">데몬 URL</label>
          <input class="kk-set-text" type="text" value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder="/v1/gui 또는 http://localhost:47302/v1/gui" />
          <label class="kk-set-label">인증 토큰 (Bearer)</label>
          <input class="kk-set-text" type="password" value={token()} onInput={(e) => setToken(e.currentTarget.value)} placeholder="mcp-token" />
          <div class="kk-set-row">
            <button class="kk-set-btn" onClick={saveConn}>저장</button>
            <button class="kk-set-btn alt" onClick={() => void testConn()}>연결 테스트</button>
          </div>
          <Show when={connMsg()}><div class="kk-set-msg">{connMsg()}</div></Show>

          {/* 연동 안내 (정본 hint) */}
          <div class="wsec" style="margin-top:18px;">연동</div>
          <div class="kk-set-note">
            외부 채널 연동(디스코드·텔레그램 등)은 <b>에이전트 탭</b>에서 에이전트별로,
            워크플로우 채널은 <b>흐름 탭</b>에서 설정합니다.
          </div>
        </div>
      </div>
    </div>
  );
}
