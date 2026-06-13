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

// 친구(Friend) 추가 모달 — "다른 머신·외부"를 친구로 등록.
// 종류 2종:
//   🖥 머신     — 다른 머신의 OpenXgram. 별칭(=alias) + 주소(Tailscale IP/host).
//                 기존 agents_register 라우트 재사용(machine 필드 + messenger_enabled=true →
//                 sub-keypair + peers 등록). project_path 는 비워(원격 = 로컬 파일트리 없음).
//   🌐 외부 A2A — 외부 에이전트. 별칭(alias) + AgentCard base URL.
//                 위임 시 a2a_send target=URL 로 쓰는 그 값. 백엔드에 영속 라우트가 없으므로
//                 로컬(localStorage)에 등록만 — TalkTab 이 친구 섹션에 합쳐 표시한다.
//
// 신규 백엔드 라우트는 만들지 않는다(머신=agents_register 재사용, 외부=로컬 상태).

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
  { v: "machine", icon: "🖥", label: "머신", hint: "다른 머신의 OpenXgram (peer 등록)" },
  { v: "external", icon: "🌐", label: "외부 A2A", hint: "외부 에이전트 (AgentCard URL)" },
];

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

  // 장치 선택 → 별칭 기본값 = 장치 이름, 주소 = 장치 IP 자동 prefill.
  function pickDevice(d: TailnetDevice) {
    if (d.self) return; // 자기 머신은 선택 불가.
    setSelectedDev(d.ip);
    if (!alias().trim()) setAlias(d.name);
    setAddr(d.ip);
    setErr(null);
  }

  async function create() {
    const name = alias().trim();
    const address = addr().trim();
    if (!name) { setErr("별칭(대화명)을 입력하세요."); return; }
    if (!address) {
      setErr(kind() === "machine" ? "주소(Tailscale IP/host)를 입력하세요." : "AgentCard base URL 을 입력하세요.");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      if (kind() === "machine") {
        // 머신 친구 — 기존 agents_register 재사용. project_path 는 비움(원격 = 로컬 파일트리 없음).
        // machine 에 주소를 담아 친구로 분류되게 한다(TalkTab 분류 기준: machine 이 원격값).
        // messenger_enabled=true → sub-keypair + peers 등록(실제 peer 통신 경로).
        await invoke("agents_register", {
          alias: name,
          role: "친구 머신",
          description: `원격 머신 OpenXgram (${address})`,
          project_path: null,
          group_name: null,
          messenger_enabled: true,
          ai_type: "claude",
          classification: "friend",
          execution_mode: "on_demand",
          machine: address,
          worktree: null,
          is_public: false,
        });
        props.onCreated(name, "machine");
      } else {
        // 외부 A2A 친구 — 로컬 등록(백엔드 외부 영속 라우트 부재). url = a2a_send target.
        const list = loadExternalFriends();
        const next = list.filter((f) => f.alias !== name);
        next.push({ alias: name, url: address, kind: "external" });
        saveExternalFriends(next);
        props.onCreated(name, "external");
      }
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
        <p class="sub">다른 머신·외부 에이전트를 친구로 등록 — A2A/peer 로 통신합니다.</p>

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

        {/* 🖥 머신 종류 — Tailscale 장치 자동 목록. 클릭 선택 시 별칭·주소 자동 채움.
            라우트 없음/tailscale 부재 시 graceful: 목록 숨기고 아래 수동 IP 입력만. */}
        <Show when={kind() === "machine"}>
          <div class="fld">
            <label>Tailscale 장치 선택</label>
            <Show when={devLoading()}>
              <div class="hint" style="margin-top:2px;">장치 목록 불러오는 중…</div>
            </Show>
            <Show when={!devLoading() && devFailed()}>
              <div class="hint" style="margin-top:2px;">
                장치 목록을 가져올 수 없습니다 (tailscale 라우트 없음). 아래에 IP 를 직접 입력하세요.
              </div>
            </Show>
            <Show when={!devLoading() && !devFailed() && devices().length === 0}>
              <div class="hint" style="margin-top:2px;">탐지된 장치가 없습니다. 아래에 IP 를 직접 입력하세요.</div>
            </Show>
            <Show when={!devLoading() && devices().length > 0}>
              <div style="display:flex;flex-direction:column;gap:4px;max-height:180px;overflow-y:auto;margin-top:4px;">
                <For each={devices()}>
                  {(d) => (
                    <div
                      onClick={() => pickDevice(d)}
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
          </div>
        </Show>

        <div class="fld">
          <label>별칭 (대화명)</label>
          <input class="ctl" value={alias()} onInput={(e) => setAlias(e.currentTarget.value)}
            placeholder={kind() === "machine" ? "zalman" : "moneyprinter"} />
        </div>

        <div class="fld">
          <label>{kind() === "machine" ? "주소 (Tailscale IP / host) — 장치 선택 시 자동, 수동 입력 가능" : "AgentCard base URL"}</label>
          <input class="ctl" value={addr()} onInput={(e) => { setAddr(e.currentTarget.value); setSelectedDev(null); }}
            placeholder={kind() === "machine" ? "100.x.x.x 또는 host" : "https://agent.example.com"} />
        </div>

        <Show when={err()}>
          <div style="color:#ff6b6b;font-size:12px;margin:6px 0;">⚠ {err()}</div>
        </Show>

        <div class="modal-foot">
          <button class="btn-q" onClick={() => props.onClose()} disabled={busy()}>취소</button>
          <button class="btn-go" onClick={() => void create()} disabled={busy()}>{busy() ? "추가 중…" : "추가"}</button>
        </div>
        <div class="hint">
          머신 친구는 그쪽 primary 가 자기 에이전트를 처리합니다 · 외부는 AgentCard URL 로 A2A 통신.
        </div>
      </div>
    </div>
  );
}
