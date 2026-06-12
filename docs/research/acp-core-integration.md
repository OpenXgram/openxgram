# ACP (Agent Client Protocol) — OpenXgram Core Integration

> Research + architecture spec. **No implementation here** — produces exact spec, existing-code conventions, concrete design, and a phased plan.
> Author: research/architect pass. Date: 2026-06-09.
> Goal: OpenXgram acts as an **ACP Client** that can spawn / session-manage / prompt / relay tool-calls to ACP agents (`claude-agent-acp`, `codex-acp`, `gemini --acp`, `opencode acp`, `pi-acp`, `vibe-acp`).

---

## 1. ACP Spec Core Summary

**Source**: https://agentclientprotocol.com (Zed). JSON-RPC 2.0 over **stdio**. Protocol version is a single integer (`protocolVersion: 1`, the current MAJOR). The **Client spawns the Agent as a subprocess** and drives it.

### 1.1 Transport
- **stdio only** for the agent process: Client writes JSON-RPC requests to the agent's **stdin**, reads responses/notifications from its **stdout**. Each JSON-RPC message is newline-delimited (LDJSON-style, one JSON object per line). **stderr is for logs** — never parse it as protocol (critical: see §6 stdio pollution).
- Bidirectional: the *agent* also calls back into the *client* (fs/*, session/request_permission, terminal/*) over the same pipe. So OpenXgram must run a **full JSON-RPC peer** (both request-sender and request-handler), not just a client.

### 1.2 Message types
- **Methods**: request/response (have `id`).
- **Notifications**: one-way (no `id`), e.g. `session/update`, `session/cancel`.

### 1.3 Method set (direction = caller)

**Client → Agent (we call these):**
- `initialize` — negotiate `protocolVersion` + exchange capabilities. Params: `{ protocolVersion, clientCapabilities: { fs: {readTextFile, writeTextFile}, terminal }, clientInfo: {name,title,version} }`. Response: `{ protocolVersion, agentCapabilities: { loadSession, promptCapabilities: {image,audio,embeddedContext}, mcpCapabilities: {http,sse} }, agentInfo, authMethods }`.
- `authenticate` — only if `authMethods` non-empty.
- `session/new` — params `{ cwd, mcpServers: [{name,command,args,env}] }` → `{ sessionId }`.
- `session/load` — (optional, gated by `loadSession` cap) resume; agent replays history via `session/update`.
- `session/prompt` — params `{ sessionId, prompt: ContentBlock[] }` → `{ stopReason }` (response arrives only when the **turn ends**).
- `session/set_mode` — (optional) switch agent operating mode.
- `logout` — (optional).
- `session/cancel` — **notification**, interrupts current turn.

**Agent → Client (we must handle these):**
- `session/update` — **notification**, the streaming channel. `update.sessionUpdate` discriminator:
  - `agent_message_chunk` / `user_message_chunk` (content: ContentBlock; replay during load)
  - `agent_thought_chunk`
  - `plan` (entries: `{content, priority, status}`)
  - `tool_call` (`{toolCallId, title, kind, status, content[], locations[], rawInput, rawOutput}`)
  - `tool_call_update` (progress/final for an existing `toolCallId`)
- `fs/read_text_file` — request `{ sessionId, path, line?, limit? }` → `{ content }`. Only if we advertised `clientCapabilities.fs.readTextFile`.
- `fs/write_text_file` — request `{ sessionId, path, content }` → `{}`.
- `session/request_permission` — request; we respond `{ outcome: {selected|cancelled, optionId} }`.
- `terminal/*` — (optional, only if we advertise `clientCapabilities.terminal`): create/output/wait/kill/release.

### 1.4 ContentBlock types
Baseline (all agents MUST support in prompt): `text`, `resource_link`. Optional (gated by `promptCapabilities`): `image`, `audio`, `resource` (embeddedContext — inline `{uri,mimeType,text}`).

### 1.5 ToolCall
`kind` ∈ read|edit|delete|move|search|execute|think|fetch|other. `status` ∈ pending|in_progress|completed|failed. Content variants: `content`(ContentBlock), `diff`(`{path,oldText,newText}`), `terminal`(`{terminalId}`).

### 1.6 StopReason
`session/prompt` resolves with: `end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, `cancelled`. On `session/cancel`, agent MUST eventually respond `cancelled` (must NOT surface as a JSON-RPC error).

### 1.7 Turn lifecycle (MVP target)
`initialize` → (`authenticate`?) → `session/new` → `session/prompt` → [stream of `session/update`; possibly `fs/*` + `session/request_permission` callbacks] → `session/prompt` resolves with `stopReason`.

---

## 2. `openxgram-acp` Crate Design

### 2.1 Conventions inherited from `openxgram-a2a` / `openxgram-anp`
Both existing protocol crates establish the pattern the new crate MUST follow:
- `crates/openxgram-<proto>/` with `src/{lib.rs, client.rs, message.rs|task.rs, mcp.rs, discovery.rs?}` + `tests/integration.rs`.
- `lib.rs`: crate doc-comment citing PRD §; `#![forbid(unsafe_code)]`; one `thiserror`-derived error enum (**no silent fallback — every failure is an explicit variant**, "절대 규칙 1"); `pub type Result<T>`; `pub use` re-exports of the public types; consts for protocol literals.
- `Cargo.toml`: `version.workspace = true` etc.; deps `{ serde, serde_json, tokio, thiserror, tracing, anyhow }` from `workspace = true`; path dep `openxgram-core`. dev-deps `tempfile`, `tokio` (macros, rt-multi-thread), `mockito`.
- MCP tools live in `mcp.rs` as a stateless/handle struct (`A2aTools`, `AnpTools`) exposing `async fn` per tool; **the JSON-RPC/MCP wrapping is done by `openxgram-cli/mcp_serve.rs`, not the crate**.
- Workspace `members` already lists `openxgram-a2a` and `openxgram-anp` — add `crates/openxgram-acp` there (workspace root edit is allowed for this work, unlike the per-PRD note on a2a/anp).

### 2.2 Module structure
```
crates/openxgram-acp/
  Cargo.toml
  src/
    lib.rs        # crate doc, AcpError enum, Result, consts (PROTOCOL_VERSION=1, JSONRPC_VERSION)
    transport.rs  # newline-delimited JSON-RPC framing over a child's stdin/stdout (tokio)
    rpc.rs        # JSON-RPC 2.0 peer: id allocation, pending-request map, inbound dispatch
    client.rs     # AcpClient: spawn + initialize + session/new + prompt; owns one agent process
    session.rs    # AcpSession: per-session state (sessionId, cwd, turn channel)
    handlers.rs   # ClientSideHandlers trait: fs/read,fs/write,request_permission,terminal (default-deny / minimal)
    types.rs      # serde types: Initialize*, SessionNew*, ContentBlock, SessionUpdate (tagged), ToolCall, StopReason
    registry.rs   # known ACP agent adapters: name -> {command, args, env} (claude-agent-acp, codex-acp, gemini --acp, ...)
    mcp.rs        # AcpTools: acp_spawn / acp_prompt / acp_cancel / acp_list_agents / acp_close
  tests/integration.rs   # mock agent (a tiny script echoing JSON-RPC) for spawn→initialize→prompt e2e
```
**Reuse note** (중복 검사): `openxgram-mcp` already wraps stdio MCP servers — review it before writing `transport.rs`; the LDJSON framing + child-process plumbing may be partly extractable rather than re-implemented. ACP's wire format is JSON-RPC-over-stdio just like MCP, so the transport layer is largely shared shape.

### 2.3 Core types
- **`AcpClient`** — owns the spawned `tokio::process::Child`, the `RpcPeer`, and a `JoinHandle` for the stdout reader loop. API:
  - `async fn spawn(agent: AgentSpec, client_caps: ClientCapabilities, handlers: Arc<dyn ClientSideHandlers>) -> Result<AcpClient>` — spawns process, runs `initialize`, stores negotiated `AgentCapabilities`.
  - `async fn new_session(&self, cwd, mcp_servers) -> Result<AcpSession>`
  - `fn agent_capabilities(&self) -> &AgentCapabilities`
  - `async fn shutdown(self) -> Result<()>` — drop session, close stdin, await child exit (with kill-on-timeout).
- **`AcpSession`** — `{ session_id, cwd }` + a handle back to the client's `RpcPeer`. API:
  - `async fn prompt(&self, blocks: Vec<ContentBlock>) -> Result<PromptResult>` where `PromptResult { stop_reason, updates: Vec<SessionUpdate> }` OR a streaming variant:
  - `fn prompt_stream(&self, blocks) -> (impl Stream<Item=SessionUpdate>, oneshot<StopReason>)` — preferred so OpenXgram can relay chunks live.
  - `async fn cancel(&self) -> Result<()>` (sends `session/cancel` notification).
- **`RpcPeer`** (rpc.rs) — the heart. Holds `pending: Mutex<HashMap<RequestId, oneshot::Sender<Value>>>` and an `mpsc` to the writer task. Single **reader task** parses each inbound line and routes:
  - response (has `id` we issued) → resolve the matching `oneshot`.
  - request (has `id`, method is agent→client) → invoke `ClientSideHandlers`, write response.
  - notification (no `id`, e.g. `session/update`) → push onto the active session's `mpsc`.
- **`ClientSideHandlers`** trait — what OpenXgram exposes to the agent. MVP: `fs/*` reads/writes scoped to `cwd` (or default-deny if cap not advertised); `request_permission` → policy hook (MVP: auto-grant the first/`allow_once` option or always-deny based on config); `terminal/*` unimplemented in MVP (don't advertise the cap).

### 2.4 Async pattern + runtime-in-runtime panic avoidance (critical)
The existing crates are pure async (`async fn`, awaited by the caller). `mcp_serve.rs` is a **synchronous JSON-RPC dispatcher running inside an existing tokio runtime**; it bridges to async with `tokio::task::block_in_place(|| Handle::current().block_on(async { ... }))` everywhere (see rc.195/rc.197/rc.286 comments — bare `Handle::current().block_on()` panics / drops connections inside the runtime thread). **`openxgram-acp` MUST keep all its public API `async`** and let `mcp_serve` apply the *same* `block_in_place(|| handle.block_on(...))` wrapper. Additional ACP-specific hazards:
  - The reader/writer tasks for the child must be `tokio::spawn`ed on the **outer** runtime (not a nested `Runtime::new()`), so the long-lived agent process outlives a single `block_on`.
  - `acp_spawn` cannot keep the `AcpClient` alive purely on the `block_on` stack — see §3.2 (process registry held by the daemon, not the per-call frame).

### 2.5 Error enum (sketch — no silent fallback)
`AcpError`: `Spawn(io)`, `InitFailed{got,want}` (version mismatch), `RpcError{code,message}`, `InvalidRpcResponse`, `Protocol(String)`, `AuthRequired`, `SessionClosed`, `AgentExited{code}`, `Timeout`, `Serde(#[from])`, `Io(#[from])`.

---

## 3. OpenXgram Integration Points

### 3.1 Where it attaches
- **CLI/MCP tools** (`crates/openxgram-cli/src/mcp_serve.rs`): add the ACP tools to the `match name { ... }` dispatch, exactly like `a2a_*` / `anp_*` handlers are wrapped today. New tools: `acp_list_agents`, `acp_spawn`, `acp_prompt`, `acp_cancel`, `acp_close`. Each wraps `AcpTools` async methods in the established `block_in_place(|| handle.block_on(...))` bridge.
- **Daemon** (`daemon_gui.rs`): holds the **process registry** (long-lived `HashMap<AgentHandleId, AcpClient>`), because spawned agents must outlive a single MCP request. This mirrors how the daemon already owns long-lived state (peer store, bindings). `acp_spawn` returns a handle id; subsequent `acp_prompt`/`acp_cancel`/`acp_close` look the agent up in that registry.

### 3.2 Mapping to the agent/peer model (`agent_capabilities` + `adapter_type`)
OpenXgram already has an **adapter registry** (`orchestration_adapter.rs`, "paperclip registry.ts pattern": `adapter_type` string → Adapter dispatch) and stores agents in the `agent_capabilities` table with columns incl. `adapter_type`, `adapter_config`. Existing values: `peer_send`. **Proposal: a new `adapter_type = 'acp'`.**
- `adapter_config` (JSON) for an ACP agent: `{ "agent": "claude-agent-acp", "cwd": "...", "mcp_servers": [...] , "permission_policy": "..." }`.
- A new `AcpAdapter` registered in `orchestration_adapter::get_adapter("acp")` implements the single-shot run primitive (`daemon_gui.rs` `agent invoke`): spawn (or reuse) the ACP client for that agent row, run one `session/new` + `session/prompt`, collect `session/update` text into the run result, then `stopReason`. This makes ACP agents **first-class OpenXgram org agents** visible in `list_peers`/org-chart, addressable via the existing single-shot invoke, with zero new UI surface (reuses the org-agent panel — satisfies "UI 검증 가능성").
- **Relation to `openxgram_peer` (paperclip)**: `openxgram_peer` is the *cross-agent invoke transport over the OpenXgram network* (peer alias → run). `adapter_type=acp` is the *local spawn-and-drive transport for a subprocess agent*. They compose: a `peer_send`/org invoke can resolve to an agent whose adapter is `acp`, so a remote peer asking OpenXgram to "run task X" can be fulfilled by spawning a local ACP agent. Keep them as **sibling adapters** behind the same registry, not merged.

### 3.3 Capability advertisement
On `initialize` OpenXgram advertises `clientCapabilities` = `{ fs: {readTextFile:true, writeTextFile:true}, terminal:false }` for MVP. `fs/*` is genuinely useful (lets the agent see unsaved/db-backed state and lets OpenXgram audit writes) and is cheap. `terminal` deferred (process-management heavy).

---

## 4. Phased Implementation Plan

**MVP definition of done**: spawn `claude-agent-acp` → `initialize` → `session/new` → `session/prompt("hello")` → receive `agent_message_chunk` updates + `end_turn` stop reason, end-to-end, driven from one `acp_*` MCP tool.

### Phase 1 — Crate skeleton + transport + JSON-RPC peer (foundation)
- New `crates/openxgram-acp` added to workspace `members`. `lib.rs` (error enum, consts), `types.rs` (initialize/session/contentblock/sessionupdate serde, round-trip unit-tested against spec JSON examples), `transport.rs` (LDJSON framing over child stdin/stdout — review/extract from `openxgram-mcp` first), `rpc.rs` (`RpcPeer`: id alloc, pending map, reader loop, inbound request routing).
- **Deliverable**: `cargo test -p openxgram-acp` green; serde types round-trip every spec example; `RpcPeer` unit-tested with an in-memory duplex pipe.

### Phase 2 — `AcpClient` spawn + initialize + session/new (process lifecycle)
- `client.rs` spawn via `tokio::process::Command` (kill_on_drop, piped stdio, **stderr forwarded to `tracing`, never parsed**), run `initialize` (version negotiation, store `AgentCapabilities`), `session.rs` `new_session`. `registry.rs` with `claude-agent-acp` spec entry. Minimal `ClientSideHandlers` (default-deny / no-op for fs+permission).
- **Deliverable**: `tests/integration.rs` spawns a **mock agent script** (tiny node/python echoing JSON-RPC) and asserts initialize+session/new succeed; version-mismatch path returns `AcpError::InitFailed`.

### Phase 3 — `session/prompt` turn + `session/update` relay (MVP e2e)
- `AcpSession::prompt` / `prompt_stream`: send prompt, stream inbound `session/update` notifications via `mpsc`, resolve `stopReason`. Handle `session/cancel`. Wire **real `claude-agent-acp`** (npm) in the integration test (gated behind an env flag so CI without the npm package still passes).
- **Deliverable**: MVP DoD met against real `claude-agent-acp`; cancel returns `cancelled` not an error.

### Phase 4 — Client-side callbacks: `fs/*` + `session/request_permission`
- Implement `ClientSideHandlers` for `fs/read_text_file` / `fs/write_text_file` (cwd-scoped, audited) and `session/request_permission` (config-driven policy: allow_once / deny / always-allow-for-session). Advertise the matching `clientCapabilities`.
- **Deliverable**: an ACP tool-call that reads+edits a file round-trips through OpenXgram's handlers; permission denial cleanly aborts the turn.

### Phase 5 — MCP tools + daemon process registry (surface)
- `mcp.rs` `AcpTools` (`acp_list_agents/spawn/prompt/cancel/close`). Wire into `mcp_serve.rs` `match name` with the `block_in_place(|| handle.block_on(...))` bridge. Daemon-held `HashMap<handle_id, AcpClient>` so spawned agents persist across MCP calls; `acp_close` + idle-timeout reaper for lifecycle.
- **Deliverable**: from an MCP client, `acp_spawn`→`acp_prompt`→`acp_close` works; agents survive between separate tool calls; no orphaned processes after `acp_close`/daemon shutdown.

### Phase 6 — Org-agent adapter (`adapter_type='acp'`) + multi-agent registry
- `AcpAdapter` registered in `orchestration_adapter::get_adapter`. `adapter_type='acp'` rows in `agent_capabilities` runnable via the existing single-shot `agent invoke` and visible in `list_peers`/org-chart UI. Fill `registry.rs` with `codex-acp`, `gemini --acp`, `opencode acp`, `pi-acp`, `vibe-acp`. Version bump + push per project rules.
- **Deliverable**: an ACP agent added through the org UI runs a task and shows output in the existing panel (UI-verifiable); multiple adapter kinds spawnable.

---

## 5. Risks / Cautions

- **Runtime-in-runtime panic**: bare `Handle::current().block_on()` inside `mcp_serve`'s runtime thread panics / drops connections (rc.195/197/286). All ACP public API stays `async`; `mcp_serve` bridges with `block_in_place(|| handle.block_on(...))`. Long-lived reader/writer tasks must be `tokio::spawn`ed on the **outer** runtime — never `Runtime::new()` nested.
- **stdio pollution**: the agent's protocol is on **stdout**; any stray stdout print (or our own logging to stdout) corrupts the JSON-RPC stream. Forward child **stderr** to `tracing`; ensure OpenXgram's own logs never hit the child's stdout pipe. (Mirrors the `register_subagent` subprocess-isolation note in `mcp_serve.rs:1555` — init prints must not collide with stdio JSON-RPC.)
- **Process lifecycle**: spawned agents outlive a single MCP request → must live in the daemon registry, not a `block_on` stack frame. Use `kill_on_drop(true)` + explicit `acp_close` + idle reaper + daemon-shutdown sweep to avoid orphaned/zombie processes.
- **Bidirectional deadlock**: ACP is full-duplex (agent calls back into us mid-`prompt`). The reader loop MUST NOT block on a handler that itself awaits another agent response. Handlers run on spawned tasks; the reader only dispatches.
- **Capability honesty**: never call/accept a method whose capability we didn't advertise (spec MUST). Gate `fs/*` and `terminal/*` strictly on advertised `clientCapabilities`; gate image/audio/embedded prompt content on `agentCapabilities.promptCapabilities`.
- **Version drift**: `protocolVersion` is a single int; on mismatch the agent returns its latest — if we don't support it, fail loud (`AcpError::InitFailed`), don't silently downgrade.
- **MCP-server nesting**: `session/new` lets us pass MCP servers to the ACP agent. Don't accidentally point an ACP agent back at OpenXgram's own MCP server in a way that recurses.

---

## 6. References
- ACP spec: agentclientprotocol.com — overview, v1/initialization, v1/session-setup, v1/prompt-turn, v1/file-system, v1/tool-calls.
- Adapter: github.com/agentclientprotocol/claude-agent-acp (npm `@agentclientprotocol/claude-agent-acp`).
- Existing crates: `crates/openxgram-a2a`, `crates/openxgram-anp` (conventions); `crates/openxgram-mcp` (stdio JSON-RPC reuse); `crates/openxgram-cli/src/{mcp_serve.rs, daemon_gui.rs, orchestration_adapter.rs}` (dispatch, registry, runtime bridge).
