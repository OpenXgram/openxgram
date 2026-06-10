import { createSignal, createEffect, onCleanup, For, Show } from "solid-js";
import { acpFetch, acpStream } from "../api/client";

// ACP 대화방 (Phase B-3) — 로컬 ACP 에이전트 subprocess 를 daemon `/v1/acp/*` 로
// 구동하고 `session/update` SSE 를 카카오톡 정본 대화 UI(.msgs/.me/.agent/.toolcall/
// pre.code/composer)로 렌더. peer 대화(TalkTab)와 동일한 마크업·CSS 재사용 —
// 데이터 출처만 peer_send/peer_conversation 대신 ACP 스트림.
//
// daemon 계약(daemon_gui_acp.rs)과 1:1:
//   POST /sessions            body {agent, cwd, executionMode?}  → {sessionId, agent, cwd, executionMode, spawned}
//   POST /sessions/{id}/prompt body {text}                       → {stopReason, updates}
//   GET  /sessions/{id}/stream  (SSE event: session_update)      → session/update params
//   POST /sessions/{id}/cancel                                   → session/cancel 결과
//   DELETE /sessions/{id}                                        → close + reap
//
// session/update payload(types.rs SessionUpdate, sessionUpdate 태그 snake_case):
//   agent_message_chunk / agent_thought_chunk / user_message_chunk → {content:{type:"text",text}}
//   tool_call / tool_call_update → {toolCallId,title,kind,status,content[]}
//   plan → {entries:[{content,priority,status}]}

// ACP 세션 기본 작업 디렉토리 — 라벨 상수(흩뿌리지 않음). 향후 설정값으로 대체 가능.
const DEFAULT_ACP_CWD = "/home/llm/projects/starian-set/openxgram";

interface AgentInfo {
  name: string;
  installed: boolean;
}

// 대화 버블 모델 — peer 대화와 동일한 시각 표현으로 매핑.
type Bubble =
  | { id: number; kind: "me"; text: string; time: string }
  | { id: number; kind: "agent"; segs: Seg[]; time: string }
  | { id: number; kind: "tool"; toolId: string; title: string; status: string; time: string }
  | { id: number; kind: "plan"; entries: { content: string; status: string }[]; time: string }
  | { id: number; kind: "note"; text: string; time: string };

type Seg = { kind: "text"; text: string } | { kind: "code"; text: string };

// 에이전트 본문을 펜스드 코드블록 기준으로 text/code 분해 (TalkTab.segmentBody 와 동일 정책).
function segmentText(body: string): Seg[] {
  const out: Seg[] = [];
  const fence = /```[\w-]*\n?([\s\S]*?)```/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = fence.exec(body)) !== null) {
    if (m.index > last) {
      const t = body.slice(last, m.index).replace(/^\n+|\n+$/g, "");
      if (t) out.push({ kind: "text", text: t });
    }
    out.push({ kind: "code", text: m[1].replace(/\n$/, "") });
    last = fence.lastIndex;
  }
  if (last < body.length) {
    const t = body.slice(last).replace(/^\n+|\n+$/g, "");
    if (t) out.push({ kind: "text", text: t });
  }
  if (out.length === 0) out.push({ kind: "text", text: body });
  return out;
}

function nowClock(): string {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// content 블록 배열에서 text 추출 (ACP ContentBlock {type:"text",text} 위주, diff/resource 폴백).
function blocksToText(content: unknown): string {
  if (!Array.isArray(content)) return "";
  const parts: string[] = [];
  for (const b of content) {
    if (b && typeof b === "object") {
      const o = b as Record<string, unknown>;
      if (typeof o.text === "string") parts.push(o.text);
      else if (o.content && typeof o.content === "object") {
        const c = o.content as Record<string, unknown>;
        if (typeof c.text === "string") parts.push(c.text);
      }
    }
  }
  return parts.join("\n");
}

export function AcpConversation(props: { onClose: () => void }) {
  const [agents, setAgents] = createSignal<AgentInfo[] | null>(null);
  const [agentsErr, setAgentsErr] = createSignal<string | null>(null);
  const [sessionId, setSessionId] = createSignal<string | null>(null);
  const [activeAgent, setActiveAgent] = createSignal<string | null>(null);
  const [spawnErr, setSpawnErr] = createSignal<string | null>(null);
  const [bubbles, setBubbles] = createSignal<Bubble[]>([]);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal(false); // 세션 생성/프롬프트 진행 중
  const [streaming, setStreaming] = createSignal(false);

  let nextId = 1;
  let stopStream: (() => void) | null = null;
  // 현재 진행 중인 에이전트 turn 버블 id (chunk 누적용) + tool_call id→bubble id 매핑.
  let curAgentBubbleId: number | null = null;
  const toolBubbleByCall = new Map<string, number>();

  let msgsRef: HTMLDivElement | undefined;
  function scrollDown() {
    queueMicrotask(() => {
      if (msgsRef) msgsRef.scrollTop = msgsRef.scrollHeight;
    });
  }

  // 설치된 ACP 에이전트 목록 로드.
  async function loadAgents() {
    setAgentsErr(null);
    try {
      const r = await acpFetch<{ agents: AgentInfo[] }>("GET", "/agents");
      setAgents(r.agents ?? []);
    } catch (e) {
      setAgentsErr((e as Error)?.message ?? String(e));
      setAgents([]);
    }
  }
  loadAgents();

  function pushBubble(b: Bubble) {
    setBubbles((prev) => [...prev, b]);
    scrollDown();
  }

  // session/update 한 건을 버블에 반영 (스트림 + prompt 응답 updates 공용).
  function applyUpdate(u: unknown) {
    if (!u || typeof u !== "object") return;
    const o = u as Record<string, unknown>;
    const tag = o.sessionUpdate as string | undefined;
    if (!tag) return;
    if (tag === "agent_message_chunk" || tag === "agent_thought_chunk") {
      const text = blocksToText([o.content]);
      if (!text) return;
      if (curAgentBubbleId == null) {
        const id = nextId++;
        curAgentBubbleId = id;
        pushBubble({ id, kind: "agent", segs: segmentText(text), time: nowClock() });
      } else {
        const id = curAgentBubbleId;
        setBubbles((prev) =>
          prev.map((b) => {
            if (b.id !== id || b.kind !== "agent") return b;
            const merged = b.segs
              .filter((s) => s.kind === "text")
              .map((s) => s.text)
              .join("");
            return { ...b, segs: segmentText(merged + text) };
          }),
        );
        scrollDown();
      }
    } else if (tag === "tool_call") {
      const callId = String(o.toolCallId ?? "");
      const id = nextId++;
      if (callId) toolBubbleByCall.set(callId, id);
      pushBubble({
        id,
        kind: "tool",
        toolId: callId,
        title: String(o.title ?? o.kind ?? "tool"),
        status: String(o.status ?? "pending"),
        time: nowClock(),
      });
      curAgentBubbleId = null; // tool 이후 새 에이전트 chunk 는 새 버블로.
    } else if (tag === "tool_call_update") {
      const callId = String(o.toolCallId ?? "");
      const bid = toolBubbleByCall.get(callId);
      if (bid != null && o.status) {
        setBubbles((prev) =>
          prev.map((b) => (b.id === bid && b.kind === "tool" ? { ...b, status: String(o.status) } : b)),
        );
      }
    } else if (tag === "plan") {
      const entries = Array.isArray(o.entries)
        ? (o.entries as Record<string, unknown>[]).map((e) => ({
            content: String(e.content ?? ""),
            status: String(e.status ?? ""),
          }))
        : [];
      pushBubble({ id: nextId++, kind: "plan", entries, time: nowClock() });
      curAgentBubbleId = null;
    } else if (tag === "user_message_chunk") {
      // 에이전트가 user 입력을 replay — 이미 .me 로 그렸으므로 무시.
    }
  }

  // 에이전트 선택 → 세션 생성 + SSE 구독.
  async function spawn(agent: string) {
    if (busy()) return;
    setBusy(true);
    setSpawnErr(null);
    try {
      const r = await acpFetch<{ sessionId: string; agent: string; spawned: boolean }>(
        "POST",
        "/sessions",
        { agent, cwd: DEFAULT_ACP_CWD, executionMode: "on_demand" },
      );
      setSessionId(r.sessionId);
      setActiveAgent(agent);
      setBubbles([]);
      curAgentBubbleId = null;
      toolBubbleByCall.clear();
      pushBubble({
        id: nextId++,
        kind: "note",
        text: `⚡ ACP 세션 시작 — ${agent}${r.spawned ? " (구동됨)" : " (첫 프롬프트 시 구동)"}`,
        time: nowClock(),
      });
      // SSE 구독 시작 (prompt turn 중 발생한 update 가 relay 됨).
      stopStream?.();
      setStreaming(true);
      stopStream = acpStream(
        r.sessionId,
        (payload) => applyUpdate(payload),
        (msg) => {
          setStreaming(false);
          pushBubble({ id: nextId++, kind: "note", text: `⚠ 스트림: ${msg}`, time: nowClock() });
        },
      );
    } catch (e) {
      setSpawnErr((e as Error)?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  async function sendPrompt() {
    const id = sessionId();
    const text = draft().trim();
    if (!id || !text || busy()) return;
    setBusy(true);
    setSpawnErr(null);
    pushBubble({ id: nextId++, kind: "me", text, time: nowClock() });
    setDraft("");
    curAgentBubbleId = null;
    try {
      // SSE 가 동일 update 를 먼저 relay 할 수 있으므로, prompt 응답의 updates 는
      // 스트림이 죽었을 때의 폴백으로만 적용. stopReason 은 note 로 표시.
      const r = await acpFetch<{ stopReason: string; updates?: unknown[] }>(
        "POST",
        `/sessions/${encodeURIComponent(id)}/prompt`,
        { text },
      );
      if (!streaming() && Array.isArray(r.updates)) {
        for (const u of r.updates) applyUpdate(u);
      }
      if (r.stopReason && r.stopReason !== "end_turn") {
        pushBubble({ id: nextId++, kind: "note", text: `· turn 종료: ${r.stopReason}`, time: nowClock() });
      }
    } catch (e) {
      pushBubble({ id: nextId++, kind: "note", text: `⚠ 구동 실패: ${(e as Error)?.message ?? e}`, time: nowClock() });
    } finally {
      setBusy(false);
    }
  }

  async function cancelTurn() {
    const id = sessionId();
    if (!id) return;
    try {
      await acpFetch("POST", `/sessions/${encodeURIComponent(id)}/cancel`);
      pushBubble({ id: nextId++, kind: "note", text: "· 취소 요청 전송", time: nowClock() });
    } catch (e) {
      pushBubble({ id: nextId++, kind: "note", text: `⚠ 취소 실패: ${(e as Error)?.message ?? e}`, time: nowClock() });
    }
  }

  // 세션 닫기 → DELETE + 스트림 중단. roster 로 복귀하지 않고 에이전트 선택 화면으로.
  async function closeSession() {
    const id = sessionId();
    stopStream?.();
    stopStream = null;
    setStreaming(false);
    if (id) {
      try {
        await acpFetch("DELETE", `/sessions/${encodeURIComponent(id)}`);
      } catch {
        // best-effort — 닫힘 실패해도 UI 는 선택 화면으로 복귀.
      }
    }
    setSessionId(null);
    setActiveAgent(null);
    setBubbles([]);
  }

  onCleanup(() => {
    stopStream?.();
    const id = sessionId();
    if (id) void acpFetch("DELETE", `/sessions/${encodeURIComponent(id)}`).catch(() => {});
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void sendPrompt();
    }
  }

  createEffect(() => {
    bubbles();
    scrollDown();
  });

  return (
    <Show
      when={sessionId()}
      fallback={
        // ── 에이전트 선택 화면 (세션 미생성) ──
        <div class="kk-talk-chat">
          <div class="chat-top">
            <span class="back" onClick={() => props.onClose()}>←</span>
            <div class="ava c-claude">⚡</div>
            <div class="nm">ACP 에이전트</div>
            <div class="meta-r">
              <span class="pill">로컬 subprocess</span>
            </div>
          </div>
          <div class="msgs">
            <div class="kk-acp-pick">
              <div class="kk-acp-pick-h">구동할 ACP 에이전트를 선택하세요</div>
              <Show when={agentsErr()}>
                <div class="kk-talk-empty">⚠ 에이전트 목록 실패: {agentsErr()}</div>
              </Show>
              <Show when={agents() == null && !agentsErr()}>
                <div class="kk-talk-empty">불러오는 중…</div>
              </Show>
              <Show when={agents() && (agents() as AgentInfo[]).length === 0 && !agentsErr()}>
                <div class="kk-talk-empty">등록된 ACP 어댑터가 없습니다.</div>
              </Show>
              <For each={agents() ?? []}>
                {(ag) => (
                  <div
                    class={`kk-acp-agent${ag.installed ? "" : " off"}`}
                    onClick={() => ag.installed && spawn(ag.name)}
                  >
                    <div class="av c-claude">⚡</div>
                    <div class="kk-acp-agent-meta">
                      <div class="kk-acp-agent-nm">{ag.name}</div>
                      <div class="kk-acp-agent-st">
                        {ag.installed ? "설치됨 · 클릭하여 세션 시작" : "ACP 에이전트 미설치"}
                      </div>
                    </div>
                    <Show when={!ag.installed}><span class="kk-acp-badge">미설치</span></Show>
                  </div>
                )}
              </For>
              <Show when={spawnErr()}>
                <div class="kk-talk-err">⚠ 세션 생성 실패: {spawnErr()}</div>
              </Show>
              <Show when={busy()}><div class="kk-talk-empty">세션 생성 중…</div></Show>
            </div>
          </div>
        </div>
      }
    >
      {/* ── ACP 대화방 (세션 활성) — peer 대화와 동일 마크업 ── */}
      <div class="kk-talk-chat">
        <div class="chat-top">
          <span class="back" onClick={() => void closeSession()}>←</span>
          <div class="ava c-claude">⚡</div>
          <div class="nm">{activeAgent()}</div>
          <div class="meta-r">
            <Show when={streaming()} fallback={<span class="pill off"><span class="pdot" />스트림 끊김</span>}>
              <span class="pill"><span class="pdot" />스트리밍</span>
            </Show>
            <span class="pill">⚡ ACP</span>
            <span class="kk-acp-x" title="세션 닫기" onClick={() => void closeSession()}>✕ 닫기</span>
          </div>
        </div>

        <div class="msgs" ref={msgsRef}>
          <For each={bubbles()}>
            {(b) =>
              b.kind === "me" ? (
                <div class="me">
                  <div class="mr"><div class="tm">{b.time}</div></div>
                  <div class="b">{b.text}</div>
                </div>
              ) : b.kind === "note" ? (
                <div class="day">{b.text}</div>
              ) : b.kind === "tool" ? (
                <div class="agent">
                  <div class="body">
                    <div class={`toolcall${b.status === "failed" ? " fail" : ""}`}>
                      <span class={b.status === "failed" ? "no" : "ok"}>{b.status === "failed" ? "✗" : "✓"}</span>{" "}
                      <span class="cmd">{b.title}</span>
                      <span class="kk-acp-tstat">{b.status}</span>
                    </div>
                  </div>
                </div>
              ) : b.kind === "plan" ? (
                <div class="agent">
                  <div class="body">
                    <div class="kk-acp-plan">
                      <div class="kk-acp-plan-h">계획</div>
                      <For each={b.entries}>
                        {(e) => (
                          <div class={`kk-acp-plan-item st-${e.status}`}>
                            <span class="kk-acp-plan-dot" /> {e.content}
                          </div>
                        )}
                      </For>
                    </div>
                  </div>
                </div>
              ) : (
                <div class="agent">
                  <div class="head">
                    <div class="av c-claude">⚡</div>
                    <div class="nm">{activeAgent()}</div>
                    <div class="tm">{b.time}</div>
                  </div>
                  <div class="body">
                    <For each={b.segs}>
                      {(seg) =>
                        seg.kind === "code" ? <pre class="code">{seg.text}</pre> : <p>{seg.text}</p>
                      }
                    </For>
                  </div>
                </div>
              )
            }
          </For>
          <Show when={bubbles().length === 0}>
            <div class="kk-talk-empty">세션 준비됨. 아래에서 첫 프롬프트를 보내세요.</div>
          </Show>
        </div>

        {/* ── 컴포저 (TalkTab 정본 Claude Code 다크 재사용) ── */}
        <div class="composer-wrap">
          <div class="composer">
            <textarea
              class="ph-input"
              rows="2"
              placeholder="프롬프트 입력···  ⚡ ACP 에이전트로 전송"
              value={draft()}
              disabled={busy()}
              onInput={(e) => setDraft(e.currentTarget.value)}
              onKeyDown={onKey}
            />
            <div class="bar">
              <div class="bar-l">
                <span class="ic-btn">@</span>
                <span class="ic-btn">/</span>
                <span class="ic-btn">📎</span>
                <span class="divv" />
                <span class="mode perm">🛡 Bypass Permissions <span class="car">⌃</span></span>
                <span class="mode model">Default (recommended) <span class="car">⌃</span></span>
                <span class="mode think">High <span class="car">⌃</span></span>
              </div>
              <div class="spacer" />
              <div class="bar-r">
                <Show when={busy()}>
                  <span class="kk-acp-cancel" onClick={() => void cancelTurn()}>■ 취소</span>
                </Show>
                <span class="usage">⚡ {activeAgent()}</span>
                <span
                  class={`send${draft().trim() && !busy() ? "" : " dis"}`}
                  onClick={() => void sendPrompt()}
                >
                  {busy() ? "…" : "➤"}
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
}
