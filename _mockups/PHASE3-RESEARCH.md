# OpenXgram Phase 3/4 Research — Workflow Orchestration + Scheduling

> Research for: agent "hire" flow (Phase 3), goal→agent-team workflow builder + A2A delegation, heartbeat/cron scheduler (Phase 4).
> Stack target: Rust (tokio/axum/rusqlite + SQLite), existing ACP + A2A + agent_profiles + xgram-ops agent.
> License constraint: we are MIT — may adopt MIT / Apache-2.0 / BSD only. **No GPL, no fair-code (n8n), no BSL runtime (Restate), no Flowise enterprise dir.**

---

## 1. Improvement Areas (concrete capabilities to add)

| # | Capability | Why / gap today |
|---|-----------|-----------------|
| A | **Workflow definition data model** (persisted DAG: goal → steps → agent bindings) | We delegate ad-hoc via A2A; no durable record of "a workflow" that can be inspected/resumed. |
| B | **Goal → agent-team planner** (LLM emits needed-roles list; reconcile owned vs hire-needed) | xgram-ops can create agents but there is no structured "what team does this GOAL need" step. |
| C | **Capability-match → hire reconciliation** | agent_profiles already have classification/execution_mode; need a match function: required_capability → existing profile OR template-to-hire. |
| D | **Durable/resumable run state** (workflow_runs + step_runs with status lifecycle) | If a step fails mid-delegation nothing resumes. Need persisted run + step states + retry. |
| E | **Scheduler / heartbeat tick loop** (cron + interval wake for execution_mode=heartbeat) | Currently a stub. Need a tokio loop reading a schedules table, firing due jobs. |
| F | **Task lifecycle states aligned to A2A** (submitted/working/input-required/completed/failed/canceled) | Gives a standard vocabulary for step_runs and resumability, and HITL pause points. |
| G | **Template catalog → profile materialization** (hire flow) | agency-agents markdown → agent_profile row. |

---

## 2. GitHub Research (verified repos + licenses)

All licenses below were fetched from each repo's actual LICENSE file (not assumed).

| Repo | URL | License (verified) | Patterns worth adopting |
|------|-----|--------------------|--------------------------|
| **a2aproject/A2A** (spec) | https://github.com/a2aproject/A2A | **Apache-2.0** ✅ | (1) **AgentCard** = id/capabilities/**skills**/endpoint/auth JSON — model for our hire reconciliation & discovery. (2) **Task lifecycle** states: submitted→working→input-required→completed/failed/canceled — adopt verbatim for step_runs. (3) **Context id** to group related tasks/messages = our workflow_run id. (4) Async-first + push-notification (webhook) pattern for long-running delegation. |
| **agentclientprotocol/agent-client-protocol** | https://github.com/agentclientprotocol/agent-client-protocol | **Apache-2.0** ✅ | JSON-RPC-over-stdio session model (we already use via claude-agent-acp). Its **proxy/conductor chain** pattern (rust-sdk) = insert middleware between client and agent without modifying agent — useful for a "scheduler-injected" wake message. |
| **crewAIInc/crewAI** | https://github.com/crewaiinc/crewai | **MIT** ✅ | (1) **Crew = agents + tasks + process(sequential/hierarchical)** — clean data model for a workflow definition. (2) **Flows** (event-driven, `@start`/`@listen`, persisted state) = resumable run model. (3) Role/goal/backstory per agent maps to our agent_profile. *Pattern adoption only — do not vendor Python.* |
| **langchain-ai/langgraph** | https://github.com/langchain-ai/langgraph | **MIT** ✅ | (1) **Graph = nodes + typed edges over a shared State** — DAG data model. (2) **Checkpointer** (persist state after every node → resume from any node) = the gold pattern for durable/resumable runs in SQLite. (3) **interrupt()** for human-in-the-loop pause = our input-required state. |
| **temporalio/temporal** | https://github.com/temporalio/temporal | **MIT** ✅ | **Durable execution**: event-history + deterministic replay; **activity retry policies** (backoff, max attempts); **timers/cron schedules** as first-class. Adopt the *concepts* (retry policy struct, schedule spec), not the engine — too heavy. |
| **microsoft/autogen** (ag2ai/ag2 fork) | https://github.com/microsoft/autogen · https://github.com/ag2ai/ag2 | autogen code **MIT** ✅ (root LICENSE is CC-BY-4.0 for docs; `LICENSE-CODE` = MIT). ag2 = **Apache-2.0** ✅ | GroupChat / conversational-handoff orchestration pattern (agent-to-agent turn passing). Lower priority — our A2A already covers delegation transport. |
| **msitarzewski/agency-agents** | https://github.com/msitarzewski/agency-agents | **MIT** ✅ (our template catalog) | Template = single `.md` with YAML-ish frontmatter: `name`, `description`, `color`, `emoji`, `vibe` + body sections (`Identity & Memory`, processes, deliverables, success metrics). Organized by **division dirs** (engineering/design/marketing/finance/...). Hire flow = parse frontmatter → agent_profile, map division → classification. |
| **FlowiseAI/Flowise** | https://github.com/FlowiseAI/Flowise | **Apache-2.0** for core (⚠️ `packages/server/src/enterprise` is Commercial — avoid that dir) | Visual node-graph JSON persistence format (nodes[], edges[]) — reference for a future GUI workflow builder serialization. Adopt only the open core's graph-JSON shape. |
| **apache/airflow** | https://github.com/apache/airflow | **Apache-2.0** ✅ | DAG + **scheduler** reference: schedule_interval / cron expr, DagRun + TaskInstance state machine, catchup. Mature reference for our scheduler table design. |

**Rejected for licensing:** n8n (Sustainable Use / fair-code — not OSI, restricts commercial), Restate runtime (BSL).

---

## 3. Adoptable Design Patterns (concrete → Rust+SQLite+ACP/A2A)

### P1 — Workflow definition data model (from CrewAI Crew + LangGraph Graph)
```
workflows(id, name, goal_text, process TEXT,            -- 'sequential'|'hierarchical'
          created_by, created_at)
workflow_steps(id, workflow_id, idx, required_capability,
               agent_profile_id NULL,                   -- bound owned agent, or NULL=hire-needed
               input_template, depends_on JSON)          -- edge list = DAG
```
Process enum + ordered steps + `depends_on` edge list = a persisted DAG. Goal stored verbatim for replanning.

### P2 — Goal → agent-team planner (from CrewAI hierarchical + AgentCard skills)
LLM (via xgram-ops/ACP) returns a structured `needed_agents[]` = `{role, required_capability, rationale}`. Reconcile each against `agent_profiles` by capability match → tag `owned` vs `hire-needed`. AgentCard `skills[]` is the canonical shape for "what a capability is."

### P3 — Delegation transport (already have A2A — align vocabulary)
Keep A2A. Adopt A2A **Task lifecycle** as the step_run status enum and **context_id = workflow_run_id** to group a run's tasks. Use A2A push-notification (webhook) semantics for long-running step completion callbacks.

### P4 — Durable / resumable execution (from LangGraph checkpointer + Temporal)
```
workflow_runs(id, workflow_id, context_id, status, goal_snapshot, created_at)
step_runs(id, run_id, step_id, status, attempt, max_attempts,
          input, output, error, started_at, finished_at)
```
status ∈ {submitted, working, input-required, completed, failed, canceled} (A2A). **Checkpoint after every step transition** (write step_runs row) → resume = "find first non-completed step, re-dispatch." Temporal-style retry policy: `attempt < max_attempts` ⇒ re-enqueue with backoff.

### P5 — Scheduler design (from Airflow + Temporal Schedules)
```
schedules(id, target_profile_id NULL, workflow_id NULL,
          kind TEXT,                  -- 'cron'|'interval'|'heartbeat'
          cron_expr, interval_secs, next_run_at, last_run_at, enabled)
```
A single tokio loop (`tokio::time::interval(1s)`) selects `WHERE enabled AND next_run_at <= now()`, fires the job (wake heartbeat agent via ACP, or start a workflow_run), then recomputes `next_run_at` (cron via the `cron` crate; interval via add). Replaces the current stub. execution_mode=heartbeat agents get an auto-registered `kind='heartbeat'` schedule.

### P6 — Template → profile materialization (from agency-agents)
Parse agency-agents `.md` frontmatter (`name`, `description`, `color`, `emoji`, `vibe`) + division dir → insert `agent_profile` (classification from division, execution_mode default on_demand, body → system prompt). This is the Phase 3 hire flow.

---

## 4. Feasibility + License Notes

| Pattern | Effort | License blocker | Note |
|---------|--------|-----------------|------|
| **P1** Workflow data model | **Easy** | none | Pure SQLite migration + serde structs. Pattern-only adoption (no code vendored). |
| **P3** A2A lifecycle alignment | **Easy** | none (Apache-2.0, we already use A2A) | Just adopt the state enum + context_id; reuses existing transport. |
| **P6** Template→profile (hire) | **Easy** | none (agency-agents MIT) | Frontmatter parse already partly done in xgram-ops template catalog; map division→classification. |
| **P4** Durable/resumable runs | **Medium** | none (LangGraph/Temporal MIT, concepts only) | Checkpoint-per-step is simple in SQLite; retry/backoff + resume-scan needs care. No replay-determinism needed (we persist explicit state, not event history). |
| **P5** Scheduler/heartbeat loop | **Medium** | none (Airflow Apache-2.0, concept only) | tokio interval loop + `cron` crate (MIT/Apache). Risk: missed ticks on restart → use `next_run_at` in DB so restart-safe. |
| **P2** Goal→team planner | **Medium-Hard** | none | Hardest part is reliable structured LLM output + capability reconciliation; mitigate with a strict JSON schema prompt through ACP. |
| Flowise graph-JSON GUI | Hard (defer) | ⚠️ avoid `enterprise/` dir; core Apache-2.0 OK | GUI builder is Phase-later; only borrow nodes[]/edges[] JSON shape. |
| Vendoring Temporal engine | Hard (reject) | none but overkill | Adopt retry/schedule *structs* only, not the server. |

### Recommended top 3 to implement first (steps 5-8)
1. **P1 + P3 — Workflow definition table + A2A-aligned run/step lifecycle.** Foundation everything else builds on; Easy; zero license risk.
2. **P5 — Restart-safe scheduler loop** (`schedules` table + tokio tick + `cron` crate), wiring execution_mode=heartbeat agents. Turns the existing stub into real function; Medium.
3. **P4 — Checkpoint-per-step durable runs** (resume = re-dispatch first non-completed step + retry policy). Makes delegation reliable; Medium. P2 planner and P6 hire-flow follow once the run substrate exists.

### Verified license summary
A2A spec, ACP, ag2, Flowise-core, Airflow = **Apache-2.0**. CrewAI, LangGraph, Temporal, autogen-code (`LICENSE-CODE`), agency-agents = **MIT**. All compatible with our MIT codebase. n8n (fair-code) and Restate-runtime (BSL) excluded.
