import { createSignal, Show, For, onMount } from "solid-js";
import { invoke } from "@/api/client";

// Tailscale 장치 — GET /tailnet/devices → { devices: [{name, ip, online, self}] }.
// 백엔드 라우트가 아직 없을 수 있음(다른 에이전트가 추가 중) → 실패 시 graceful(수동 IP 입력 폴백).
interface TailnetDevice {
  name: string;
  ip: string;
  online: boolean;
  self: boolean;
}

// 친구(Friend) 추가 모달 — "원격 머신의 에이전트·외부"를 친구로 등록 (rc.320 agent-level opt-in 모델 B).
// 종류 2종:
//   🖥 머신     — 2단계: (1) Tailscale 장치를 골라 그 머신의 친구-가능 에이전트 로스터를 fetch
//                 (friends_remote_agents → 원격 GET /v1/gui/friends/roster), (2) 로스터에서 SPECIFIC
//                 에이전트 1개를 골라 친구로 등록. 친구 row = 머신이 아니라 고른 그 에이전트.
//                 단방향(one-directional) opt-in — 강제 양방향/머신 전체 친구 없음.
//                 기존 agents_register 재사용(classification="friend" + messenger_enabled=true →
//                 sub-keypair + peers 등록). project_path 는 비워(원격 = 로컬 파일트리 없음).
//   🌐 외부 A2A — 외부 에이전트. 별칭(alias) + AgentCard base URL.
//                 위임 시 a2a_send target=URL 로 쓰는 그 값. 백엔드에 영속 라우트가 없으므로
//                 로컬(localStorage)에 등록만 — TalkTab 이 친구 섹션에 합쳐 표시한다.

// 외부 A2A 친구 — localStorage 영속(백엔드 외부 등록 라우트 부재). TalkTab 도 같은 키를 읽는다.
export const EXTERNAL_FRIENDS_KEY = "oxg.friends.external.v1";

export interface ExternalFriend {
  alias: string;
  url: string; // AgentCard base URL — a2a_send target.
  kind: "external";
}

export function loadExternalFriends(): ExternalFriend[] {
  try {
    const raw = localStorage.getItem(EXTERNAL_FRIENDS_KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    return arr.filter((x) => x && typeof x.alias === "string" && typeof x.url === "string")
      .map((x) => ({ alias: x.alias, url: x.url, kind: "external" as const }));
  } catch {
    return [];
  }
}

function saveExternalFriends(list: ExternalFriend[]) {
  try {
    localStorage.setItem(EXTERNAL_FRIENDS_KEY, JSON.stringify(list));
  } catch {
    /* 저장 실패는 조용히 — 세션 한정 등록으로 폴백 */
  }
}

type FriendKind = "machine" | "external";

const KIND_OPTS: { v: FriendKind; icon: string; label: string; hint: string }[] = [
  { v: "machine", icon: "🖥", label: "머신", hint: "원격 머신의 에이전트를 골라 친구 추가" },
  { v: "external", icon: "🌐", label: "외부 A2A", hint: "외부 에이전트 (AgentCard URL)" },
];

// 원격 머신 로스터의 친구-가능 에이전트 (friends_remote_agents 응답의 agents[] 요소).
interface RemoteAgent {
  alias: string;
  ai_type?: string;
  role?: string;
}

export function AddFriendModal(props: {
  onClose: () => void;
  // 머신 친구는 alias, 외부 친구는 alias 반환(부모가 선택/새로고침에 사용).
  onCreated: (alias: string, kind: FriendKind) => void;
}) {
  const [kind, setKind] = createSignal<FriendKind>("machine");
  const [alias, setAlias] = createSignal("");
  const [addr, setAddr] = createSignal(""); // 머신: Tailscale IP/host · 외부: AgentCard base URL
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  // Tailscale 장치 자동 목록 — 머신 친구 추가 시 클릭 선택. 실패하면 수동 IP 입력만 노출(폴백).
  const [devices, setDevices] = createSignal<TailnetDevice[]>([]);
  const [devLoading, setDevLoading] = createSignal(false);
  const [devFailed, setDevFailed] = createSignal(false); // 라우트 없음/tailscale 부재 → 수동 입력 폴백
  const [selectedDev, setSelectedDev] = createSignal<string | null>(null); // 선택한 장치 ip(키)

  // rc.320 — 머신 친구 2단계: 장치 선택 후 그 머신의 친구-가능 에이전트 로스터.
  const [remoteAgents, setRemoteAgents] = createSignal<RemoteAgent[]>([]);
  const [remoteBase, setRemoteBase] = createSignal<string>(""); // 응답 base url (친구 row 의 machine)
  const [rosterLoading, setRosterLoading] = createSignal(false);
  const [rosterErr, setRosterErr] = createSignal<string | null>(null);
  const [pickedAgent, setPickedAgent] = createSignal<string | null>(null); // 고른 원격 에이전트 alias

  // rc.321 — 친구 단위 정책 (권한/격리/비용). 에이전트 선택 후 추가 전에 설정.
  const [permission, setPermission] = createSignal<"blocked" | "read" | "request" | "full">("request");
  const [isolated, setIsolated] = createSignal(false);
  const [costTracked, setCostTracked] = createSignal(true);

  onMount(() => { void loadDevices(); });

  async function loadDevices() {
    setDevLoading(true);
    setDevFailed(false);
    try {
      const r = await invoke<{ devices?: TailnetDevice[] }>("tailnet_devices", {});
      const list = Array.isArray(r?.devices) ? r.devices : [];
      setDevices(list);
      // 장치가 0개여도 라우트는 살아있는 것 — 실패로 보지 않음(수동 입력은 항상 가능).
    } catch {
      // 라우트 미배포 / tailscale 미설치 → graceful: 수동 IP 입력만.
      setDevices([]);
      setDevFailed(true);
    } finally {
      setDevLoading(false);
    }
  }

  // (1단계) 장치 선택 → 주소 prefill + 그 머신의 친구-가능 에이전트 로스터 fetch.
  async function pickDevice(d: TailnetDevice) {
    if (d.self) return; // 자기 머신은 선택 불가.
    setSelectedDev(d.ip);
    setAddr(d.ip);
    setErr(null);
    await fetchRoster(d.ip);
  }

  // host(Tailscale IP/host)의 친구-가능 에이전트 로스터를 fetch (friends_remote_agents).
  // 실패는 silent 하지 않고 rosterErr 에 명시 — 사용자가 수동 IP 재시도/다른 장치 선택 가능.
  async function fetchRoster(host: string) {
    const h = host.trim();
    if (!h) return;
    setRosterLoading(true);
    setRosterErr(null);
    setRemoteAgents([]);
    setRemoteBase("");
    setPickedAgent(null);
    try {
      const r = await invoke<{ ok?: boolean; base?: string; machine?: string; agents?: RemoteAgent[]; error?: string }>(
        "friends_remote_agents",
        { host: h },
      );
      if (!r?.ok) {
        setRosterErr(r?.error ?? "원격 머신에 도달하지 못했습니다.");
        return;
      }
      const list = Array.isArray(r.agents) ? r.agents.filter((a) => a && typeof a.alias === "string") : [];
      setRemoteAgents(list);
      setRemoteBase(typeof r.base === "string" && r.base ? r.base : h);
      if (list.length === 0) {
        setRosterErr("이 머신에 친구로 추가할 수 있는 에이전트가 없습니다.");
      }
    } catch (e) {
      // 라우트 미배포 / 원격 데몬 미도달 → 명시 에러(수동 IP 입력 후 재시도 가능).
      setRosterErr((e as Error)?.message ?? String(e));
    } finally {
      setRosterLoading(false);
    }
  }

  // (2단계) 로스터에서 에이전트 1개 선택 → 별칭 기본값 = 그 에이전트 alias.
  function pickAgent(a: RemoteAgent) {
    setPickedAgent(a.alias);
    setAlias(a.alias);
    setErr(null);
  }

  async function create() {
    setErr(null);
    if (kind() === "machine") {
      // 머신 친구 — 2단계 검증: 로스터에서 고른 SPECIFIC 에이전트가 있어야 한다.
      const chosen = remoteAgents().find((a) => a.alias === pickedAgent());
      if (!chosen) { setErr("원격 머신의 에이전트를 먼저 선택하세요."); return; }
      const name = (alias().trim() || chosen.alias);
      // 친구 row 의 machine = 응답 base url(없으면 장치 IP). 단방향 등록 — reciprocal announce 없음.
      const machineAddr = remoteBase() || addr().trim() || selectedDev() || "";
      if (!machineAddr) { setErr("원격 머신 주소를 확인하지 못했습니다. 장치를 다시 선택하세요."); return; }
      setBusy(true);
      try {
        await invoke("agents_register", {
          alias: name,
          role: chosen.role ?? "원격 에이전트",
          description: `원격 에이전트 ${chosen.alias} (${machineAddr})`,
          project_path: null,
          group_name: null,
          messenger_enabled: true,
          ai_type: chosen.ai_type ?? "claude",
          classification: "friend",
          execution_mode: "on_demand",
          machine: machineAddr,
          worktree: null,
          is_public: false,
          // rc.321 — 친구 정책 (권한/격리/비용).
          friend_permission: permission(),
          friend_isolated: isolated(),
          friend_cost_tracked: costTracked(),
        });
        props.onCreated(name, "machine");
      } catch (e) {
        setErr((e as Error)?.message ?? String(e));
      } finally {
        setBusy(false);
      }
      return;
    }

    // 외부 A2A 친구.
    const name = alias().trim();
    const address = addr().trim();
    if (!name) { setErr("별칭(대화명)을 입력하세요."); return; }
    if (!address) { setErr("AgentCard base URL 을 입력하세요."); return; }
    setBusy(true);
    try {
      // 외부 A2A 친구 — 로컬 등록(백엔드 외부 영속 라우트 부재). url = a2a_send target.
      const list = loadExternalFriends();
      const next = list.filter((f) => f.alias !== name);
      next.push({ alias: name, url: address, kind: "external" });
      saveExternalFriends(next);
      props.onCreated(name, "external");
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="modal" style="max-width:440px;">
        <h2>👥 친구 추가</h2>
        <p class="sub">원격 머신의 에이전트를 골라 친구 추가 · 또는 외부 A2A 에이전트 — A2A/peer 로 통신합니다.</p>

        <div class="fld">
          <label>종류</label>
          <div class="seg">
            <For each={KIND_OPTS}>
              {(k) => (
                <div class={`s${kind() === k.v ? " on" : ""}`} title={k.hint} onClick={() => { setKind(k.v); setErr(null); }}>
                  {k.icon} {k.label}
                </div>
              )}
            </For>
          </div>
          <div class="hint" style="margin-top:4px;">
            {KIND_OPTS.find((k) => k.v === kind())?.hint}
          </div>
        </div>

        {/* 🖥 머신 — 2단계 흐름.
            (1단계) Tailscale 장치 선택(또는 IP 수동 입력 후 조회) → 그 머신의 에이전트 로스터 fetch.
            (2단계) 로스터에서 에이전트 1개 선택 → 친구로 등록. */}
        <Show when={kind() === "machine"}>
          {/* 1단계 — 장치 선택 */}
          <div class="fld">
            <label>① Tailscale 장치 선택</label>
            <Show when={devLoading()}>
              <div class="hint" style="margin-top:2px;">장치 목록 불러오는 중…</div>
            </Show>
            <Show when={!devLoading() && devFailed()}>
              <div class="hint" style="margin-top:2px;">
                장치 목록을 가져올 수 없습니다 (tailscale 라우트 없음). 아래에 IP 를 직접 입력하고 [조회] 하세요.
              </div>
            </Show>
            <Show when={!devLoading() && !devFailed() && devices().length === 0}>
              <div class="hint" style="margin-top:2px;">탐지된 장치가 없습니다. 아래에 IP 를 직접 입력하고 [조회] 하세요.</div>
            </Show>
            <Show when={!devLoading() && devices().length > 0}>
              <div style="display:flex;flex-direction:column;gap:4px;max-height:160px;overflow-y:auto;margin-top:4px;">
                <For each={devices()}>
                  {(d) => (
                    <div
                      onClick={() => void pickDevice(d)}
                      title={d.self ? "이 머신 — 자기 자신은 친구로 추가 불가" : `${d.name} · ${d.ip}`}
                      style={`display:flex;align-items:center;gap:8px;padding:7px 10px;border-radius:8px;` +
                        `border:1px solid ${selectedDev() === d.ip ? "#2f5d3a" : "#2a2f3a"};` +
                        (d.self
                          ? "opacity:0.5;cursor:not-allowed;background:#15171c;"
                          : `cursor:pointer;background:${selectedDev() === d.ip ? "#1d2a20" : "#15171c"};`)}
                    >
                      <span style={`width:8px;height:8px;border-radius:50%;flex:none;background:${d.online ? "#3fb950" : "#4a4f5a"};`} />
                      <span style="font-size:13px;color:#cfe3d6;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">
                        {d.name}{d.self ? " (이 머신)" : ""}
                      </span>
                      <span style="font-size:11px;color:#6b7280;margin-left:auto;flex:none;">{d.ip}</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
            {/* 수동 IP 입력 + 조회 (장치 자동탐지 실패/누락 폴백) */}
            <div style="display:flex;gap:6px;margin-top:6px;">
              <input class="ctl" style="flex:1;" value={addr()}
                onInput={(e) => { setAddr(e.currentTarget.value); setSelectedDev(null); }}
                placeholder="100.x.x.x 또는 host (수동)" />
              <button class="btn-q" disabled={rosterLoading() || !addr().trim()}
                onClick={() => void fetchRoster(addr().trim())}>조회</button>
            </div>
          </div>

          {/* 2단계 — 에이전트 선택 */}
          <div class="fld">
            <label>② 원격 머신의 에이전트 선택</label>
            <Show when={rosterLoading()}>
              <div class="hint" style="margin-top:2px;">원격 에이전트 목록 불러오는 중…</div>
            </Show>
            <Show when={!rosterLoading() && rosterErr()}>
              <div style="color:#ff6b6b;font-size:12px;margin-top:2px;">⚠ {rosterErr()}</div>
            </Show>
            <Show when={!rosterLoading() && !rosterErr() && remoteAgents().length === 0}>
              <div class="hint" style="margin-top:2px;">먼저 위에서 장치를 선택하거나 IP 를 조회하세요.</div>
            </Show>
            <Show when={!rosterLoading() && remoteAgents().length > 0}>
              <div style="display:flex;flex-direction:column;gap:4px;max-height:180px;overflow-y:auto;margin-top:4px;">
                <For each={remoteAgents()}>
                  {(a) => (
                    <div
                      onClick={() => pickAgent(a)}
                      title={`${a.alias}${a.role ? " · " + a.role : ""}`}
                      style={`display:flex;align-items:center;gap:8px;padding:7px 10px;border-radius:8px;cursor:pointer;` +
                        `border:1px solid ${pickedAgent() === a.alias ? "#2f5d3a" : "#2a2f3a"};` +
                        `background:${pickedAgent() === a.alias ? "#1d2a20" : "#15171c"};`}
                    >
                      <span style="font-size:13px;color:#cfe3d6;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">
                        {a.alias}
                      </span>
                      <Show when={a.role}>
                        <span style="font-size:11px;color:#8b94a3;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">· {a.role}</span>
                      </Show>
                      <span style="font-size:11px;color:#6b7280;margin-left:auto;flex:none;">{a.ai_type ?? "claude"}</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>

          {/* ③ rc.321 — 친구 정책 (에이전트 선택 후 추가 전). 권한/격리/비용. */}
          <Show when={pickedAgent()}>
            <div class="fld">
              <label>③ 친구 정책</label>
              <div style="display:flex;flex-direction:column;gap:8px;margin-top:4px;">
                <div style="display:flex;align-items:center;gap:8px;">
                  <span style="font-size:12px;color:#8b94a3;width:64px;flex:none;">권한</span>
                  <select class="ctl" style="flex:1;" value={permission()}
                    onChange={(e) => setPermission(e.currentTarget.value as "blocked" | "read" | "request" | "full")}>
                    <option value="blocked">차단 (요청 거절)</option>
                    <option value="read">읽기 (상태/조회만)</option>
                    <option value="request">작업요청 (기본)</option>
                    <option value="full">전체</option>
                  </select>
                </div>
                <label style="display:flex;align-items:center;gap:8px;cursor:pointer;font-size:13px;color:#cfe3d6;">
                  <input type="checkbox" checked={isolated()} onChange={(e) => setIsolated(e.currentTarget.checked)} />
                  격리 — 친구 작업을 별도 디렉토리에서 실행 (메인 워크트리 보호)
                </label>
                <label style="display:flex;align-items:center;gap:8px;cursor:pointer;font-size:13px;color:#cfe3d6;">
                  <input type="checkbox" checked={costTracked()} onChange={(e) => setCostTracked(e.currentTarget.checked)} />
                  비용 기록 — 친구별 사용량 원장에 기록
                </label>
              </div>
            </div>
          </Show>
        </Show>

        {/* 외부 A2A — 별칭 + AgentCard URL */}
        <Show when={kind() === "external"}>
          <div class="fld">
            <label>별칭 (대화명)</label>
            <input class="ctl" value={alias()} onInput={(e) => setAlias(e.currentTarget.value)}
              placeholder="moneyprinter" />
          </div>
          <div class="fld">
            <label>AgentCard base URL</label>
            <input class="ctl" value={addr()} onInput={(e) => setAddr(e.currentTarget.value)}
              placeholder="https://agent.example.com" />
          </div>
        </Show>

        <Show when={err()}>
          <div style="color:#ff6b6b;font-size:12px;margin:6px 0;">⚠ {err()}</div>
        </Show>

        <div class="modal-foot">
          <button class="btn-q" onClick={() => props.onClose()} disabled={busy()}>취소</button>
          <button class="btn-go" onClick={() => void create()}
            disabled={busy() || (kind() === "machine" && !pickedAgent())}>{busy() ? "추가 중…" : "추가"}</button>
        </div>
        <div class="hint">
          원격 머신의 에이전트를 골라 단방향 친구로 추가 (opt-in) · 외부는 AgentCard URL 로 A2A 통신.
        </div>
      </div>
    </div>
  );
}
