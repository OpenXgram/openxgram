# Paperclip Orchestration — Extraction for OpenXgram Absorption

Source: github.com/paperclipai/paperclip (HEAD). Stack: Node/TS server, Drizzle + Postgres,
React UI (`ui/`), pnpm monorepo (`packages/db`, `packages/adapter-utils`, `packages/shared`,
`server/`, `cli/`, `ui/`). 3191 files, 86 schema tables, 89 migrations.
Extraction only — no code changed. This doc is the mirroring spec.

---

## 1. Data Model / Schema (key tables + columns + relations)

All tables are company-scoped (`company_id` FK on nearly every row → tenant isolation, mirrors our
maker_id rule). Drizzle pgTable, UUID PKs.

### companies (`packages/db/src/schema/companies.ts`)
- id, name, description, status (active/paused), pauseReason, pausedAt
- **issuePrefix** ("PAP"), **issueCounter** (per-company issue numbering)
- **budgetMonthlyCents / spentMonthlyCents** (company-level budget)
- requireBoardApprovalForNewAgents (governance toggle), brandColor, attachmentMaxBytes

### agents (`agents.ts`) — THE org-chart node
- id, companyId FK, name, **role** (default "general"), title, icon, status (idle/...)
- **reportsTo** → self-FK agents.id (org hierarchy / who-delegates-to-whom)
- capabilities (text), **adapterType** (default "process"), **adapterConfig** (jsonb),
  runtimeConfig (jsonb), defaultEnvironmentId FK
- budgetMonthlyCents/spentMonthlyCents (per-agent budget), pauseReason/pausedAt
- permissions (jsonb), lastHeartbeatAt, metadata
- Indexes: (companyId,status), (companyId,reportsTo), (companyId,defaultEnvironmentId)
- **agent_memberships** — links agents to userIds (which humans "follow"/own an agent)

### projects / goals (`projects.ts`, `goals.ts`, `project_goals.ts`)
- projects: companyId, goalId FK, name, status (backlog), leadAgentId FK, env (jsonb),
  executionWorkspacePolicy (jsonb), targetDate, archivedAt
- goals: companyId, title, **level** (task/...), status, **parentId** → self-FK (goal ancestry tree),
  ownerAgentId FK
- project_goals: M:N join (projectId,goalId composite PK)

### issues (`issues.ts`) — THE unit of work
- id, companyId, projectId, projectWorkspaceId, goalId, **parentId** → self-FK (issue tree / child fan-out)
- title, description, status (backlog/in_progress/...), workMode (standard), priority
- **assigneeAgentId** FK + assigneeUserId (delegation target)
- **checkoutRunId / executionRunId** FK → heartbeat_runs (atomic checkout lock),
  **executionAgentNameKey**, **executionLockedAt** (lock timestamp)
- createdByAgentId / createdByUserId
- **issueNumber + identifier** (e.g. PAP-123), **originKind** (manual/routine/webhook/...),
  originId, originRunId, **originFingerprint** (dedup), **requestDepth** (delegation depth guard)
- assigneeAdapterOverrides, executionPolicy, executionState (jsonb)
- monitor* fields (monitorNextCheckAt, monitorWakeRequestedAt, attemptCount) — self-rescheduling watchdog
- executionWorkspaceId FK, executionWorkspacePreference/Settings, sourceTrust (jsonb)
- startedAt/completedAt/cancelledAt/hiddenAt
- **issue_relations** — edges, type="blocks" (dependency DAG), unique(company,issue,related,type)

### issue tree / parallel fan-out
- **issue_plan_decompositions** (`issue_plan_decompositions.ts`) — parent issue → N children:
  sourceIssueId, acceptedPlanRevisionId FK→documentRevisions, status (in_flight),
  requestFingerprint, requestedChildCount, requestedChildren (jsonb), **childIssueIds** (jsonb array),
  ownerAgentId, ownerRunId FK. Partial index on status='in_flight' (one active decomposition/owner).
- **issue_tree_holds** + **issue_tree_hold_members** — pause/gate an entire subtree:
  hold has rootIssueId, mode, status (active), releasePolicy; members snapshot each issue in tree
  with depth, assigneeAgentId, activeRunId, skipped/skipReason.

### heartbeat (run engine) tables
- **heartbeat_runs** (`heartbeat_runs.ts`) — one agent execution:
  companyId, agentId, invocationSource (on_demand/...), triggerDetail, status (queued/running/...),
  startedAt/finishedAt/error, **wakeupRequestId** FK, exitCode, signal, usageJson, resultJson,
  sessionIdBefore/After, log* (logStore, logRef, logBytes, logSha256, stdout/stderrExcerpt),
  externalRunId, **processPid / processGroupId / processStartedAt** (orphan recovery),
  lastOutputAt/Seq/Stream/Bytes (output watchdog), **retryOfRunId** self-FK, processLossRetryCount,
  scheduledRetryAt/Attempt/Reason, **livenessState/livenessReason**, continuationAttempt,
  lastUsefulActionAt, nextAction, contextSnapshot (jsonb)
- **heartbeat_run_events** — bigserial seq stream of run output/events (runId, seq, eventType,
  stream, level, message, payload) — the live transcript.
- **heartbeat_run_watchdog_decisions** — decision (continue/snooze/kill), snoozedUntil, reason.
- **agent_wakeup_requests** (`agent_wakeup_requests.ts`) — THE wake queue:
  companyId, agentId, **source**, triggerDetail, reason, payload (jsonb),
  status (queued/claimed/skipped/failed), **coalescedCount** (dedup counter),
  requestedByActorType/Id, **idempotencyKey**, runId, requestedAt/claimedAt/finishedAt, error.
- **agent_task_sessions** — per (company,agent,adapter,taskKey) session reuse:
  sessionParamsJson, sessionDisplayId, lastRunId, lastError. Unique(company,agent,adapter,taskKey).
- **agent_runtime_state** — agentId PK: adapterType, sessionId, stateJson, lastRunId/Status,
  cumulative totalInput/Output/CachedTokens, totalCostCents.

### budget / cost / governance
- **budget_policies** — scopeType (company/agent/project/...), scopeId, metric (billed_cents),
  windowKind (monthly/...), amount, **warnPercent** (80), **hardStopEnabled** (true),
  notifyEnabled, isActive. Unique(company,scope,metric,window).
- **budget_incidents** — policyId, scope, window start/end, thresholdType, amountLimit/Observed,
  status (open/dismissed), **approvalId** FK → approvals.
- **cost_events** — per run cost ledger: agentId, issueId, projectId, goalId, heartbeatRunId,
  provider, biller, billingType, model, input/cached/outputTokens, **costCents**, occurredAt.
- **approvals** — type, requestedByAgentId/UserId, status (pending/approved/rejected),
  payload (jsonb), decisionNote, decidedByUserId, decidedAt. **issue_approvals** join (issue↔approval).
- **issue_execution_decisions** — staged decision tracking: stageId, **stageType**, actorAgentId/UserId,
  **outcome**, body, createdByRunId. (approval/plan-accept stages per issue.)

### routines / triggers
- **routines** (`routines.ts`) — companyId, projectId, goalId, parentIssueId, title, assigneeAgentId,
  priority, status (active), **concurrencyPolicy** (coalesce_if_active), **catchUpPolicy** (skip_missed),
  variables (jsonb), env (jsonb), latestRevisionId/Number, lastTriggeredAt, lastEnqueuedAt.
- **routine_revisions** — versioned snapshot (revisionNumber, snapshot jsonb, changeSummary, restoredFrom).
- (triggers stored per-routine: kind = cron | webhook | manual | api — see §5.)

### execution environment / secrets / workspace
- **environments** — driver (local/...), config (jsonb), status. **environment_leases** (run holds lease).
- **execution_workspaces** — projectId, sourceIssueId, mode, strategyType, cwd, repoUrl, baseRef,
  branchName, providerType (local_fs), derivedFromExecutionWorkspaceId (fork lineage), cleanup* fields.
- **project_workspaces / workspace_operations / workspace_runtime_services** — workspace lifecycle + logs.
- **company_secrets** (+ _versions, _bindings, _provider_configs) — key, name, provider
  (local_encrypted), managedMode, externalRef, latestVersion, lastRotatedAt. **secret_access_events** audit.
- **company_skills** — reusable skill defs (key/id), mentioned by `@id` in issue/instructions.
- **activity_log** — actorType/Id, action, entityType/entityId, agentId, runId, details (jsonb).

---

## 2. Heartbeat Execution Loop (file/function map)

Core service: `server/src/services/heartbeat.ts` (~7600 lines, `heartbeatService(db, options)` at L3056).
Scheduler/tick parsing: `server/src/services/cron.ts` (`parseCron`, `nextCronTick`, `nextCronTickFromExpression`).

Pipeline (function call sites confirmed in heartbeat.ts):
1. **Wake enqueue** — `agent_wakeup_requests` row inserted (status=queued) by routine/assignment/monitor.
   Coalescing: if a live continuation/wake for same agent exists, increment `coalescedCount`
   instead of new row (L6179-6192) — `mergeCoalescedContextSnapshot` (L2361) merges payloads.
2. **Claim** — `setWakeupStatus(wakeupRequestId, "claimed", {claimedAt})` (L6802); creates a
   `heartbeat_runs` row (status=queued) linked via wakeupRequestId.
3. **Atomic start lock** — `withAgentStartLock(agentId, fn)` (`agent-start-lock.ts`) — in-process
   per-agent mutex w/ stale timeout (AGENT_START_LOCK_STALE_MS); ensures one run per agent at a time.
   Issue-level checkout uses `issues.checkoutRunId` / `executionLockedAt` columns (DB lock).
4. **Budget check (hard-stop gate)** — `budgets.getInvocationBlock(companyId, agentId, scope)`
   (L4712, L5017, L5649, L6710) BEFORE spawning. `budgets.ts`: `budgetStatusFromObserved` →
   hard_stop if observed≥amount; `pauseAndCancelScopeForBudget` pauses+cancels; creates
   budget_incident → approval. If blocked, wakeup set "skipped" (L6857,7063).
5. **Run-next ordering** — `startNextQueuedRunForAgent(agentId)` (L7612) ranks queued runs by issue
   readiness (in_progress first, blocked last).
6. **Workspace resolve** — `resolveWorkspaceForRun` (L4263) → `buildRealizedExecutionWorkspaceFromPersisted`
   (L713), low-trust sandbox preflight (`preflightLowTrustWorkspaceIsolation` L810,
   `resolveWorkspaceAfterLowTrustPreflight` L839), git-sensitivity guard (`assertGitSensitiveAdapterWorkspaceValid`).
   `workspace-realization.ts` / `workspace-runtime.ts` do the actual fs/branch setup.
7. **Secret injection** — `secretsSvc.resolveAdapterConfigForRuntime(...)` (L446) returns
   {config, secretKeys, manifest}; `secretsSvc.resolveEnvBindings(...)` (L462,488) injects env from
   company_secrets. `secrets.ts` + `secret_access_events` audit. Redacted in logs (`log-redaction.ts`).
8. **Skill loading** — `extractMentionedSkillIdsFromSources` (L524) / `applyRunScopedMentionedSkillKeys`
   (L539) resolves `@skill` mentions → keys from company_skills (L613-628); passed into prompt.
9. **Prompt build** — `buildPaperclipWakePayload` (L2386) + `buildPaperclipTaskMarkdown` (L2689)
   assemble TASK.md-style prompt (issue + context + org + skills). Model profile via
   `resolveModelProfileApplication` (L1485) / `mergeModelProfileAdapterConfig` (L1549).
10. **Adapter invoke** — `getServerAdapter(adapterType)` (L2881) → `adapter.execute(ctx)` with
    AdapterExecutionContext (see §4). Spawn metadata captured via `onSpawn` → processPid/groupId for
    orphan recovery; live output via `onLog` → heartbeat_run_events.
11. **Session continuation** — `resolveNextSessionState` (L2919), `buildExplicitResumeSessionOverride`
    (L1751), `deriveTaskKeyWithHeartbeatFallback` (L2006), `shouldResetTaskSessionForWake` (L2019)
    persist sessionIdAfter into agent_task_sessions / agent_runtime_state.
12. **Result + cost** — usage→cost_events; `summarizeHeartbeatRunListResultJson` (L1610),
    `summarizeHeartbeatRunContextSnapshot` (L1586); wakeup set finished; activity_log written.

Watchdog / orphan recovery:
- Output watchdog (migration 0070_active_run_output_watchdog) — runs with no output past threshold →
  heartbeat_run_watchdog_decisions (continue/snooze/kill). `run-liveness.ts`, `issue-liveness.ts`.
- Process recovery — processPid/processGroupId/processStartedAt reconciled on restart;
  `computeBoundedTransientHeartbeatRetrySchedule` (L559) for transient-upstream retries
  (retryOfRunId, scheduledRetryAt). Tests: heartbeat-process-recovery, heartbeat-retry-scheduling.
- Stale queue invalidation, continuation: `run-continuations.ts`, `issue-continuation-summary.ts`.

---

## 3. Delegation Model

- **Org chart up/down**: `agents.reportsTo` self-FK. heartbeatService builds org rows
  (`listCompanyAgentOrgRows`, `toAgentOrgRow`, `groupAgentOrgRowsByCompany`) and injects org context
  into the prompt so an agent knows its manager/reports.
- **Delegation = assign an issue**: setting `issues.assigneeAgentId` triggers
  `queueIssueAssignmentWakeup` (`issue-assignment-wakeup.ts`) → wakes the assignee (unless backlog).
  `requestDepth` on issues guards runaway delegation depth.
- **Parallel child fan-out**: an agent decomposes a parent issue into N children via
  `issue_plan_decompositions` (childIssueIds[]); each child is its own issue with its own assignee →
  each gets its own wakeup → parallel runs. `issue_relations` type="blocks" enforces ordering;
  `startNextQueuedRunForAgent` ranks ready vs blocked.
- **Atomic task checkout (lock)**: issue claimed by writing `checkoutRunId`/`executionRunId` +
  `executionLockedAt` + `executionAgentNameKey`; combined with in-process `withAgentStartLock`.
  Prevents two runs grabbing the same issue. Subtree gating via issue_tree_holds.

---

## 4. Adapter System

Interface in `packages/adapter-utils/src/types.ts`; registry `server/src/adapters/registry.ts`;
barrel `server/src/adapters/index.ts`.

A `ServerAdapterModule` is an object keyed by adapter type with fields:
`execute(ctx)`, `sessionCodec`, `listModels`, plus display/env-check metadata.

Registered adapter types: **claude, acpx, codex, cursor, cursor-cloud, gemini, grok,
openclaw-gateway, opencode, pi, hermes**, plus generic **process** (`adapters/process/`) and
**http** webhook (`adapters/http/`). Plugins add more via `adapters/plugin-loader.ts`.

Input — `AdapterExecutionContext`:
- runId, agent {id, companyId, name, adapterType, adapterConfig}
- runtime {sessionId, sessionParams, sessionDisplayId, taskKey} (continuation)
- config (resolved, secrets injected), context (prompt/issue/org/skills)
- runtimeCommandSpec, executionTarget (local/remote/sandbox), executionTransport.remoteExecution
- callbacks: **onLog(stream, chunk)**, **onMeta(invocationMeta)**, **onSpawn({pid,processGroupId,startedAt})**
- authToken
- `AdapterInvocationMeta`: adapterType, command, cwd, commandArgs, env, prompt, promptMetrics, context.

Output — `AdapterExecutionResult`:
- exitCode, signal, timedOut, errorMessage/Code, **errorFamily** ("transient_upstream"), retryNotBefore
- **usage** {inputTokens, outputTokens, cachedInputTokens}
- sessionId/sessionParams/sessionDisplayId (persisted for next run)
- provider, biller, model, billingType, **costUsd**, resultJson, summary
- runtimeServices[] (long-lived services the run started), clearSession
- **question** {prompt, choices[]} — adapter can ask a human (→ approval/decision flow).

Process adapter = spawn CLI + stream stdout/stderr. HTTP adapter = POST to webhook, map response.
Execution targets: local / SSH / sandbox / remote-managed (`execution-target.ts`,
`remote-managed-runtime.ts`, `sandbox-managed-runtime.ts`, `ssh.ts`).

---

## 5. Routines / Triggers

`server/src/services/routines.ts` (+ `routes/routines.ts`, `cli/src/commands/routines.ts`).
Trigger kinds per routine: **cron** (timezone-aware: `nextCronTickInTimeZone`, `matchesCronMinute`),
**webhook** (`routineWebhookSecretConfigPath`, HMAC `normalizeWebhookTimestampMs`), **manual**, **api**.

Flow on trigger:
1. Resolve variables (`resolveRoutineVariableValues`, source = schedule|manual|api|webhook;
   webhook payload merged in).
2. Build dispatch fingerprint (`createRoutineDispatchFingerprint` + `createRoutineEnvFingerprint`)
   for dedup (migration 0062_routine_run_dispatch_fingerprint).
3. **concurrencyPolicy=coalesce_if_active** → if a live execution issue from this routine exists,
   coalesce (status text "Coalesced into an existing live execution issue") instead of new issue.
4. Else create an issue (originKind=routine, originFingerprint=fingerprint, parentIssueId optional) →
   `queueIssueAssignmentWakeup` enqueues a wakeup for assigneeAgentId.
5. catchUpPolicy=skip_missed governs missed cron windows. Plugins can manage routines
   (`plugin-managed-routines.ts`).

---

## 6. Approval / Governance

- **Staged decisions**: `issue_execution_decisions` (stageType, outcome, body) — plan-accept and
  approval stages tracked per issue. `issue-execution-policy.ts`, `issue-approvals.ts`.
- **Approvals**: `approvals` table (pending→approved/rejected, decidedBy, decisionNote);
  `issue_approvals` links to issue; `approvals.ts` service + UI Approvals/ApprovalDetail.
- **Budget hard-stop**: `budgets.ts` `budgetStatusFromObserved` (warning at warnPercent, hard_stop at
  amount). On hard_stop+hardStopEnabled → `pauseAndCancelScopeForBudget` (pause scope + cancel runs),
  create budget_incident → approval to resume. `getInvocationBlock` is the pre-run gate.
- **Pause/resume/terminate**: pauseReason/pausedAt on company/agent/project; `resumeScopeFromBudget`;
  run cancel paths cancel queued/running and release wakeups + environment leases.
- **Decision/audit tracking**: activity_log (every action), secret_access_events, cost_events ledger,
  heartbeat_run_watchdog_decisions, routine_revisions.
- New-agent governance: `companies.requireBoardApprovalForNewAgents` + board_api_keys / board-claim.

---

## 7. UI Structure

React SPA `ui/src/`, pages in `ui/src/pages/`, transcript adapters in `ui/src/adapters/`
(claude-local, codex, cursor-local/cloud, gemini-local, opencode-local — render run transcripts),
plus `ui/src/api`, `components`, `context`, `hooks`, `plugins`.

Orchestration-relevant pages:
- **OrgChart.tsx / Org.tsx** — visualize agents.reportsTo hierarchy; add/place agents.
- **Agents.tsx / AgentDetail.tsx / NewAgent.tsx** — agent CRUD, adapter config, budget, pause.
- **Issues.tsx / IssueDetail.tsx / MyIssues.tsx / Inbox.tsx** — issue board, chat thread, child tree,
  assignment, approvals inline (IssueChat* are perf/ux labs).
- **Projects/ProjectDetail, Goals/GoalDetail** — goal ancestry + project view.
- **Routines.tsx / RoutineDetail.tsx** — cron/webhook trigger config, variables, revisions.
- **Approvals.tsx / ApprovalDetail.tsx** — pending approvals + decision.
- **Costs.tsx** — cost_events analytics; budgets surfaced in CompanySettings.
- **Dashboard.tsx / DashboardLive.tsx / Activity.tsx** — live runs + activity_log feed.
- **AdapterManager.tsx** — adapter registry/models. **Secrets.tsx / CompanySkills.tsx /
  CompanyEnvironments.tsx / Workspaces / ExecutionWorkspaceDetail** — supporting config.
- **PluginManager / PluginPage** — plugin-contributed adapters/routines/skills.

API: REST routes under `server/src/routes/` (issues-checkout-wakeup.ts, routines.ts, approvals,
agents, heartbeat-list, etc.). CLI mirror in `cli/src/commands/` (heartbeat-run, routines).

---

## Mapping — Paperclip concept → OpenXgram (have / partial / none)

- company (tenant) → OpenXgram: NONE explicit. We have per-peer data_dir; maker_id concept exists in
  platform DB. Need a company/org container. (partial — multi-tenant exists at platform layer)
- agents (org node, reportsTo) → **PARTIAL**: `agent_capabilities` (daemon_gui.rs) has alias, role,
  description, capabilities, **orchestration_role**, group_name, special_instructions; `peers` table
  has identity/address. No reportsTo hierarchy, no adapterType/adapterConfig/budget columns.
- adapterType/adapterConfig → **NONE**. Our peers ARE the runtime (tmux Claude Code via peer_send).
  No pluggable adapter registry; peer_send is the only "adapter".
- issues (unit of work) → **NONE**. We have workflow YAML steps (workflow_engine.rs StepDef:
  id, agent, action, input, to, body, depends_on) — DAG, not an issue/assignee/lock model.
- goals/projects/goal-ancestry → **NONE**.
- heartbeat_runs / run engine → **PARTIAL**: workflow_runs + workflow_step_logs exist
  (run_workflow / execute_step), but no per-agent wake queue, no session continuation, no orphan
  recovery, no live event stream table.
- agent_wakeup_requests (coalescing queue) → **NONE**. peer_send is fire-and-forget; no queue/coalesce.
- atomic checkout lock → **NONE** (no checkoutRunId/executionLockedAt equivalent).
- issue_plan_decompositions / child fan-out → **PARTIAL** via workflow DAG depends_on, but no
  dynamic runtime decomposition by an agent.
- budget_policies / hard-stop / cost_events → **NONE**. EngineResult.total_cost is summed but no
  policy/enforcement/ledger.
- approvals / issue_execution_decisions → **PARTIAL**: EngineResult status can be "waiting_human";
  no approvals table or staged decision tracking.
- routines (cron/webhook → issue+wake) → **NONE**. (Workflows are manually run.)
- company_secrets injection → **PARTIAL**: we have vault (XGRAM_KEYSTORE_PASSWORD, vault_get/set)
  per-peer; not company-scoped, not auto-injected into runs as env.
- skills loading → **PARTIAL**: Claude Code Skill system exists at the peer, not orchestrator-driven.
- activity_log → **PARTIAL**: messages/L2 memory + workflow_step_logs; no unified activity feed.
- UI org chart / issue board / approvals / costs → **PARTIAL**: daemon_gui has messenger cards
  (tmux peers) + workflow GUI; no org chart, issue board, routines, approvals, or cost dashboards.

### "Each fleet peer as an addable org agent" — integration plan
Make an OpenXgram `agents` row a thin overlay on the existing peer/agent_capabilities identity:
- Reuse `agent_capabilities` (alias, role, description, capabilities, orchestration_role) as the agent
  profile. Add columns: companyId, reportsTo, adapterType (default `peer_send`), adapterConfig,
  budgetMonthlyCents, status, pausedAt.
- Introduce a **`peer_send` adapter** implementing the paperclip adapter contract: execute(ctx) →
  peer_send(alias, prompt) + poll recv_messages for the reply; map reply→AdapterExecutionResult
  (summary/usage/cost). onLog streams partial replies. This lets every fleet peer (from `list_peers`)
  be an "addable agent" with adapterType=peer_send, adapterConfig={alias}.
- cross-machine rule (oxg.md §6 #7): only a machine's primary is addable directly; sub-agents are
  reached via their primary → model as reportsTo edges (primary = manager of its sub-agents).

---

## Absorption Phases (dependency-ordered)

**Phase 1 — Core entities (schema).** Add Postgres/SQLite tables: companies, agents (extend
agent_capabilities + reportsTo/adapter/budget), goals(+parentId), projects, project_goals,
issues(+parentId, assignee, checkout lock cols), issue_relations, activity_log.
Output: migrations + Rust models; org chart + issue board readable.

**Phase 2 — Adapter abstraction + peer_send adapter.** Define adapter trait mirroring
AdapterExecutionContext/Result; implement `peer_send`, `process`, `http` adapters. Make every
`list_peers` entry an addable agent (adapterType=peer_send). Output: an agent run = invoke adapter,
capture usage/cost/summary.

**Phase 3 — Heartbeat run engine + wake queue.** Add heartbeat_runs, heartbeat_run_events,
agent_wakeup_requests (queued→claimed→done, coalescedCount, idempotencyKey), agent_task_sessions,
agent_runtime_state. Implement claim→lock(withAgentStartLock + issue checkout cols)→workspace→
secret→skill→adapter→record loop, plus output watchdog + orphan recovery (pid/group).
Output: assign issue → agent wakes → runs → live transcript in GUI.

**Phase 4 — Delegation + parallel fan-out.** issue assignment wakeup, requestDepth guard,
issue_plan_decompositions (child fan-out), issue_tree_holds (subtree gating), blocked-relation
ordering. Map primary→sub-agent as reportsTo. Output: an agent decomposes a goal into parallel
child issues run by reports.

**Phase 5 — Governance: budget + approvals + secrets.** budget_policies/incidents, cost_events,
getInvocationBlock pre-run gate, pause/cancel on hard-stop; approvals + issue_execution_decisions
(staged); company_secrets + auto env injection + secret_access_events. Output: hard-stop + approval
gating enforced before runs.

**Phase 6 — Routines/triggers + UI.** routines + routine_revisions, cron (tz-aware) + webhook + api
triggers → create issue (coalesce_if_active) + wake. UI: OrgChart, Issues/IssueDetail, Routines,
Approvals, Costs, DashboardLive/Activity. Output: full self-driving orchestration visible/operable
in the GUI (satisfies our UI-verification rule).
