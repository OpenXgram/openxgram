import { createSignal, Show } from "solid-js";
import { invoke } from "@/api/client";

// 에이전트 추가 모달 — 목업 정본(머신·폴더·AI종류·이름·역할·분류·그룹·실행모드·워크트리·공개).
// POST agents_register → agent_capabilities + agent_profiles 둘 다 기록(이게 있어야 로스터에 노출).
// 만들면 바로 대화방. onCreated(alias) 로 부모가 새로고침 + 선택.

const MACHINES = ["서울", "잘만", "맥미니", "sm-s936n"];
const AI_TYPES = ["claude", "codex", "gemini", "ollama", "hermes"];
const CLASS_OPTS = [
  { v: "project", label: "📁 프로젝트 에이전트" },
  { v: "special", label: "⚙️ 특수 기능 에이전트" },
  { v: "primary", label: "⭐ 통합관리 (프라이머리)" },
];

export function AddAgentModal(props: { onClose: () => void; onCreated: (alias: string) => void }) {
  const [machine, setMachine] = createSignal(MACHINES[0]);
  const [folder, setFolder] = createSignal("~/projects/starian-set");
  const [aiType, setAiType] = createSignal("claude");
  const [alias, setAlias] = createSignal("");
  const [role, setRole] = createSignal("");
  const [classification, setClassification] = createSignal("project");
  const [group, setGroup] = createSignal("");
  const [execMode, setExecMode] = createSignal("on_demand"); // always | on_demand | heartbeat
  const [worktree, setWorktree] = createSignal(false);
  const [isPublic, setIsPublic] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);

  async function create() {
    const name = alias().trim();
    if (!name) {
      setErr("이름(alias)을 입력하세요.");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await invoke("agents_register", {
        alias: name,
        role: role().trim() || null,
        description: role().trim() || null,
        project_path: folder().trim() || null,
        group_name: group().trim() || null,
        messenger_enabled: true, // 실제 에이전트 = 키페어 + peer 등록.
        ai_type: aiType(),
        classification: classification(),
        execution_mode: execMode(),
        machine: machine(),
        worktree: worktree() ? folder().trim() || null : null,
        is_public: isPublic(),
      });
      props.onCreated(name);
    } catch (e) {
      setErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="ovl" onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="modal">
        <h2>에이전트 추가</h2>
        <p class="sub">만들면 바로 대화방이 생깁니다.</p>

        <div class="mrow">
          <div class="fld">
            <label>1 · 머신</label>
            <select class="ctl" value={machine()} onChange={(e) => setMachine(e.currentTarget.value)}>
              {MACHINES.map((m) => <option value={m}>{m}</option>)}
            </select>
          </div>
          <div class="fld">
            <label>3 · AI 종류</label>
            <select class="ctl" value={aiType()} onChange={(e) => setAiType(e.currentTarget.value)}>
              {AI_TYPES.map((t) => <option value={t}>{t}</option>)}
            </select>
          </div>
        </div>

        <div class="fld">
          <label>2 · 프로젝트 폴더</label>
          <input class="ctl" value={folder()} onInput={(e) => setFolder(e.currentTarget.value)} placeholder="~/projects/..." />
        </div>

        <div class="mrow">
          <div class="fld">
            <label>4 · 이름</label>
            <input class="ctl" value={alias()} onInput={(e) => setAlias(e.currentTarget.value)} placeholder="akashic" />
          </div>
          <div class="fld">
            <label>6 · 분류</label>
            <select class="ctl" value={classification()} onChange={(e) => setClassification(e.currentTarget.value)}>
              {CLASS_OPTS.map((c) => <option value={c.v}>{c.label}</option>)}
            </select>
          </div>
        </div>

        <div class="mrow">
          <div class="fld">
            <label>5 · 역할 <span class="opt">(선택)</span></label>
            <input class="ctl" value={role()} onInput={(e) => setRole(e.currentTarget.value)} placeholder="작업 정리 · SNS 포스팅" />
          </div>
          <div class="fld">
            <label>그룹 <span class="opt">(선택)</span></label>
            <input class="ctl" value={group()} onInput={(e) => setGroup(e.currentTarget.value)} placeholder="배포팀" />
          </div>
        </div>

        <div class="fld">
          <label>실행 모드</label>
          <div class="seg">
            <div class={`s${execMode() === "always" ? " on" : ""}`} onClick={() => setExecMode("always")}>⚡ 상시 켜둠</div>
            <div class={`s${execMode() === "on_demand" ? " on" : ""}`} onClick={() => setExecMode("on_demand")}>🕓 필요할 때</div>
            <div class={`s${execMode() === "heartbeat" ? " on" : ""}`} onClick={() => setExecMode("heartbeat")}>😴 하트비트</div>
          </div>
        </div>

        <div class="mrow">
          <div class="fld">
            <label>7 · 워크트리</label>
            <div class="seg">
              <div class={`s${worktree() ? " on" : ""}`} onClick={() => setWorktree(true)}>사용</div>
              <div class={`s${!worktree() ? " on" : ""}`} onClick={() => setWorktree(false)}>안 함</div>
            </div>
          </div>
          <div class="fld">
            <label>8 · 공개 (OpenAgentX)</label>
            <div class="seg">
              <div class={`s${!isPublic() ? " on" : ""}`} onClick={() => setIsPublic(false)}>비공개</div>
              <div class={`s${isPublic() ? " on" : ""}`} onClick={() => setIsPublic(true)}>공개 →</div>
            </div>
          </div>
        </div>

        <Show when={err()}>
          <div style="color:#ff6b6b;font-size:12px;margin:6px 0;">⚠ {err()}</div>
        </Show>

        <div class="modal-foot">
          <button class="btn-q" onClick={() => props.onClose()} disabled={busy()}>취소</button>
          <button class="btn-go" onClick={() => void create()} disabled={busy()}>{busy() ? "만드는 중…" : "만들기"}</button>
        </div>
        <div class="hint">머신·tailscale·토큰은 이 안에서만 — 만든 뒤엔 에이전트 목록에 '이름'만 보입니다.</div>
      </div>
    </div>
  );
}
