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

// 친구(Friend) 추가 모달 — rc.334 Phase 4a: "친구 추가"를 두 흐름으로 명시 분리.
// 종류 3종:
//   🖥 머신 추가 (one-sided) — 내가 관리하는 머신을 추가. 추가한 쪽이 전권을 가진다(상호 동의 불필요).
//                 그 머신의 PRIMARY 피어를 등록하면 머신의 모든 실행 중 에이전트가 명부에
//                 그 머신 그룹으로 보인다(명부는 이미 머신별 그룹 + peer-sync). 상대가 나를 또 추가하면
//                 자연스럽게 상호가 된다(독립적인 두 번의 한쪽 추가). 2단계:
//                 (1) Tailscale 장치 선택(또는 IP 수동 조회) → 그 머신의 에이전트 로스터 fetch,
//                 (2) 프라이머리 피어 1개 선택 → 한쪽 등록(agents_register classification="friend").
//   🤝 에이전트 추가 (mutual · sandboxed · owner-priced) — 다른 사람의 에이전트를 사용 요청.
//                 4a = UI shell + 기존 재사용. 내가 요청 → 소유자가 수락 AND 가격 책정.
//                 통신은 격리 컨테이너의 fresh worktree 에서만(강제). 가격은 소유자가 결정(지갑/마켓 정산).
//                 ⚠️ 4b 백엔드 미구현: 상호 동의 handshake(요청→수락), 소유자 가격책정, 격리-컨테이너
//                 실행. 4a 에서는 이 부분을 명시 라벨 UI 로만 표기(가격·수락을 날조하지 않음).
//                 기존 친구 정책(rc.321 권한/격리/비용)을 재사용하되 격리는 강제, 가격은 "상대 책정 대기".
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

type FriendKind = "newlocal" | "machine" | "agent" | "external";

const KIND_OPTS: { v: FriendKind; icon: string; label: string; hint: string }[] = [
  { v: "newlocal", icon: "🆕", label: "새 에이전트 (이 머신)", hint: "이 머신에 새 로컬 에이전트를 만들고 설정 — 폴더 · AI 종류 · 역할 · 실행모드 · 워크트리" },
  { v: "machine", icon: "🖥", label: "머신 추가", hint: "내가 관리하는 머신 — 한쪽 추가 (전권 · 상호 동의 불필요)" },
  { v: "agent", icon: "🤝", label: "외부 에이전트 사용", hint: "다른 사람의 에이전트를 사용 요청 (만드는 게 아님) — 상호 수락 · 격리 · 소유자 가격책정" },
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
  // 🆕 "새 에이전트 (이 머신)" 선택 시 — 부모가 기존 AddAgentModal(로컬 생성 흐름)을 연다.
  onPickNewLocal?: () => void;
}) {
  const [kind, setKind] = createSignal<FriendKind>("newlocal");
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
    if (kind() === "machine" || kind() === "agent") {
      // 두 흐름 모두 로스터에서 고른 SPECIFIC 피어가 있어야 한다.
      //   머신 추가 = 내가 관리하는 머신의 프라이머리 피어를 한쪽 등록(전권).
      //   에이전트 추가 = 다른 사람 에이전트 사용 요청(격리 강제 · 가격은 소유자 책정 대기 = 4b).
      const isAgent = kind() === "agent";
      const chosen = remoteAgents().find((a) => a.alias === pickedAgent());
      if (!chosen) {
        setErr(isAgent ? "사용 요청할 에이전트를 먼저 선택하세요." : "머신의 프라이머리 피어를 먼저 선택하세요.");
        return;
      }
      const name = (alias().trim() || chosen.alias);
      // 친구 row 의 machine = 응답 base url(없으면 장치 IP). 단방향 등록 — reciprocal announce 없음.
      const machineAddr = remoteBase() || addr().trim() || selectedDev() || "";
      if (!machineAddr) { setErr("원격 머신 주소를 확인하지 못했습니다. 장치를 다시 선택하세요."); return; }
      setBusy(true);
      try {
        if (isAgent) {
          // 🤝 에이전트 추가 (4b) — 남의 에이전트 사용 요청. 실제 handshake:
          //   요청 row 생성 + 기존 peer envelope 로 소유자에게 전달(상태=대기). 소유자가
          //   수락 AND 가격 책정해야 사용 가능. 여기서 가격/수락을 날조하지 않는다.
          const r = await invoke<{ ok?: boolean; id?: string; delivered?: boolean; delivery_note?: string }>(
            "agent_request_create",
            {
              target_agent: chosen.alias,
              target_owner: chosen.alias, // 소유자 = 그 머신의 대상 에이전트(머신 primary 가 수신·라우팅)
              target_machine: machineAddr,
              note: `${name} 가 ${chosen.alias} 사용 요청`,
            },
          );
          if (!r?.ok) { setErr("요청 생성에 실패했습니다."); return; }
          // 전달 여부를 사용자에게 명시(peer 미도달이어도 row 는 생성됨 — 재시도/소유자 직접 통지 가능).
          props.onCreated(name, kind());
          return;
        }
        // 🖥 머신 추가 (4a) — 한쪽 등록(전권). 기존 동작 보존.
        await invoke("agents_register", {
          alias: name,
          role: chosen.role ?? "머신 프라이머리",
          description: `머신 프라이머리 ${chosen.alias} (${machineAddr})`,
          project_path: null,
          group_name: null,
          messenger_enabled: true,
          ai_type: chosen.ai_type ?? "claude",
          classification: "friend",
          execution_mode: "on_demand",
          machine: machineAddr,
          worktree: null,
          is_public: false,
          // rc.321 — 친구 정책 (권한/격리/비용). 내 머신이므로 내가 정한다.
          friend_permission: permission(),
          friend_isolated: isolated(),
          friend_cost_tracked: costTracked(),
        });
        props.onCreated(name, kind());
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
        <h2>➕ 추가</h2>
        <p class="sub">🆕 새 에이전트(이 머신 · 직접 생성) · 🖥 머신 추가(내 머신 · 한쪽) · 🤝 외부 에이전트 사용(상대 · 상호·격리·가격) · 🌐 외부 A2A.</p>

        <div class="fld">
          <label>무엇을 추가할까요?</label>
          <div class="seg">
            <For each={KIND_OPTS}>
              {(k) => (
                <div class={`s${kind() === k.v ? " on" : ""}`} title={k.hint}
                  onClick={() => {
                    setErr(null);
                    // 🆕 새 에이전트(이 머신) → 기존 AddAgentModal(폴더/모델/설정) 생성 흐름으로 즉시 전환.
                    if (k.v === "newlocal") { if (props.onPickNewLocal) props.onPickNewLocal(); return; }
                    setKind(k.v);
                  }}>
                  {k.icon} {k.label}
                </div>
              )}
            </For>
          </div>
          <div class="hint" style="margin-top:4px;">
            {KIND_OPTS.find((k) => k.v === kind())?.hint}
          </div>
        </div>

        {/* 🆕 새 에이전트 (이 머신) — 폴더/AI종류/역할/실행모드/워크트리 등 전체 설정은 AddAgentModal 에 있다.
            여기서는 그 흐름으로의 진입만 제공(필드 중복 금지). */}
        <Show when={kind() === "newlocal"}>
          <div class="hint" style="margin:2px 0 8px;padding:10px 12px;border-radius:8px;background:#eef6ff;border:1px solid #bcd6f0;color:#1e3a5f;">
            🆕 <b>이 머신에 새 에이전트</b> — 폴더 · AI 종류 · 역할 · 분류 · 그룹 · 실행모드 · 워크트리 · 공개를 직접 설정합니다.
            <div style="margin-top:8px;">
              <button class="btn-go" onClick={() => { if (props.onPickNewLocal) props.onPickNewLocal(); }}>
                새 에이전트 만들기 →
              </button>
            </div>
          </div>
        </Show>

        {/* 🖥 머신 추가 / 🤝 에이전트 추가 — 공통 2단계 (장치→로스터→피어 선택).
            머신 추가  = 내가 관리하는 머신의 프라이머리 피어를 한쪽 등록(전권 · 상호 동의 불필요).
            에이전트 추가 = 다른 사람 에이전트 사용 요청(상호 수락·격리·소유자 가격책정 — 4b). */}
        <Show when={kind() === "machine" || kind() === "agent"}>
          {/* 흐름 안내 배너 — 한쪽/상호 차이를 명시. */}
          <Show when={kind() === "machine"}>
            <div class="hint" style="margin:2px 0 8px;padding:8px 10px;border-radius:8px;background:#1d2a20;border:1px solid #2f5d3a;color:#cfe3d6;">
              🖥 <b>한쪽 추가</b> — 내가 관리하는 머신. 추가한 쪽이 전권. 상호 동의 불필요. 머신의 모든 에이전트가 명부에 그 머신 그룹으로 보입니다.
            </div>
          </Show>
          <Show when={kind() === "agent"}>
            <div class="hint" style="margin:2px 0 8px;padding:8px 10px;border-radius:8px;background:#fff6e6;border:1px solid #e2c98f;color:#6b4f12;">
              🤝 <b>외부 에이전트 사용</b> — 다른 사람의 에이전트를 <b>사용 요청</b> (새로 만드는 게 아닙니다). 내가 요청 → 소유자가 <b>수락 AND 가격 책정</b>. 통신은 격리 컨테이너의 fresh worktree 에서만(강제).
            </div>
          </Show>
          {/* 1단계 — 장치 선택 */}
          <div class="fld">
            <label>① {kind() === "agent" ? "상대 머신 (Tailscale)" : "내 머신 (Tailscale 장치)"} 선택</label>
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
                        `border:1px solid ${selectedDev() === d.ip ? "#2563eb" : "#d0d7e2"};` +
                        (d.self
                          ? "opacity:0.55;cursor:not-allowed;background:#f1f3f6;"
                          : `cursor:pointer;background:${selectedDev() === d.ip ? "#eaf2ff" : "#ffffff"};`)}
                    >
                      <span style={`width:8px;height:8px;border-radius:50%;flex:none;background:${d.online ? "#22a447" : "#b7bdc7"};`} />
                      <span style="font-size:13px;color:#16242f;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">
                        {d.name}{d.self ? " (이 머신)" : ""}
                      </span>
                      <span style="font-size:11px;color:#7a8290;margin-left:auto;flex:none;">{d.ip}</span>
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

          {/* 2단계 — 피어 선택 */}
          <div class="fld">
            <label>② {kind() === "agent" ? "사용 요청할 에이전트 선택" : "머신의 프라이머리 피어 선택"}</label>
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
                        `border:1px solid ${pickedAgent() === a.alias ? "#2563eb" : "#d0d7e2"};` +
                        `background:${pickedAgent() === a.alias ? "#eaf2ff" : "#ffffff"};`}
                    >
                      <span style="font-size:13px;color:#16242f;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">
                        {a.alias}
                      </span>
                      <Show when={a.role}>
                        <span style="font-size:11px;color:#7a8290;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">· {a.role}</span>
                      </Show>
                      <span style="font-size:11px;color:#7a8290;margin-left:auto;flex:none;">{a.ai_type ?? "claude"}</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>

          {/* ③ 머신 추가 — rc.321 친구 정책 (편집 가능). 내 머신이므로 권한/격리/비용을 내가 정한다. */}
          <Show when={kind() === "machine" && pickedAgent()}>
            <div class="fld">
              <label>③ 친구 정책 (내가 설정)</label>
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

          {/* ③ 에이전트 추가 — 상호·격리·소유자 가격. 4a = 라벨 UI(가격·수락 날조 금지).
              실제 handshake/가격책정/격리-컨테이너 실행은 4b 백엔드 미구현. */}
          <Show when={kind() === "agent" && pickedAgent()}>
            <div class="fld">
              <label>③ 사용 조건 (상대가 정함)</label>
              <div style="display:flex;flex-direction:column;gap:8px;margin-top:4px;">
                <div style="display:flex;align-items:center;gap:8px;">
                  <span style="font-size:12px;color:#8b94a3;width:64px;flex:none;">가격</span>
                  <input class="ctl" style="flex:1;" value="상대(소유자)가 책정 — 대기" disabled
                    title="가격은 소유자가 수락 시 책정합니다 (4b — 지갑/마켓 정산). 여기서 날조하지 않습니다." />
                </div>
                <div style="display:flex;align-items:center;gap:8px;font-size:13px;color:#cfe3d6;">
                  <input type="checkbox" checked disabled />
                  격리 강제 — fresh worktree · 상호 격리 컨테이너에서만 실행 (해제 불가)
                </div>
                <div class="hint" style="padding:8px 10px;border-radius:8px;background:#26221a;border:1px solid #5d4a2f;color:#e3dccf;">
                  ⏳ <b>대기 (상대 수락)</b> — 요청을 보내면 소유자가 수락하고 가격을 책정해야 사용 가능합니다.
                  <div style="margin-top:4px;font-size:11px;color:#b5a98f;">
                    요청은 기존 peer 채널로 소유자에게 전달됩니다. 수락 시 격리(fresh worktree)에서만 실행되고
                    사용량은 소유자 가격으로 과금 원장에 기록됩니다. (정산은 결제 인프라 책임 · OS 컨테이너 격리는 미구현)
                  </div>
                </div>
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
          {/* 🆕 새 에이전트 모드는 AddAgentModal 로 전환되므로 이 모달의 생성 버튼 숨김. */}
          <Show when={kind() !== "newlocal"}>
            <button class="btn-go" onClick={() => void create()}
              disabled={busy() || ((kind() === "machine" || kind() === "agent") && !pickedAgent())}>
              {busy()
                ? (kind() === "agent" ? "요청 보내는 중…" : "추가 중…")
                : (kind() === "agent" ? "요청 보내기 (상대 수락·가격책정 대기)" : "추가")}
            </button>
          </Show>
        </div>
        <div class="hint">
          🆕 새 에이전트 = 이 머신에 로컬 생성(폴더·모델·설정) · 🖥 머신 추가 = 내 머신 한쪽 추가(전권) · 🤝 외부 에이전트 사용 = 상대 수락·격리·소유자 가격(4b) · 🌐 외부는 AgentCard URL.
        </div>
      </div>
    </div>
  );
}
