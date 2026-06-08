# Paperclip — Deploy & OpenXgram-Peer Adapter Integration (extraction only)

Source: github.com/paperclipai/paperclip @ HEAD (cloned 2026-06-08, shallow). MIT licensed
(root LICENSE + `packages/db` package.json `"license":"MIT"`). This is extraction for the
PARENT to execute deploy — no code was changed, nothing was deployed.

Stack confirmed: pnpm monorepo (`pnpm@9.15.4`), Node `>=20` (our seoul host has v22.22.2,
pnpm 9 present, docker 28 + compose 2.40). Server is TS run via `tsx`. DB = Postgres
(prod) or embedded PGlite (dev, zero-setup). UI is React served by the API server when
`SERVE_UI=true` (single port 3100).

---

## 1. Run methods (clone -> install -> run)

Three paths. For our prod we recommend **Docker compose (prod)** = real Postgres.

### A. Dev / fastest smoke (embedded PGlite, no DB to manage)
    git clone https://github.com/paperclipai/paperclip.git
    cd paperclip
    pnpm install
    pnpm dev            # API+UI watch at http://localhost:3100
Leave `DATABASE_URL` unset → embedded PGlite auto-creates `data/pglite`. Health:
`curl http://localhost:3100/api/health` and `curl http://localhost:3100/api/companies`.
Reset dev DB: `rm -rf data/pglite && pnpm dev`. (`pnpm dev:once` = no file-watch.)

### B. Docker compose — production (real Postgres) — RECOMMENDED for seoul
File `docker/docker-compose.yml` already defines two services:
- `db`: `postgres:17-alpine`, user/pw/db all `paperclip`, port 5432, volume `pgdata`,
  healthcheck `pg_isready`.
- `server`: built from root `Dockerfile`, port 3100, volume `paperclip-data:/paperclip`,
  `depends_on db healthy`. Env it sets:
  `DATABASE_URL=postgres://paperclip:paperclip@db:5432/paperclip`, `PORT=3100`,
  `SERVE_UI=true`, `PAPERCLIP_DEPLOYMENT_MODE=authenticated`,
  `PAPERCLIP_DEPLOYMENT_EXPOSURE=private`,
  `PAPERCLIP_PUBLIC_URL=${PAPERCLIP_PUBLIC_URL:-http://localhost:3100}`,
  `BETTER_AUTH_SECRET=${BETTER_AUTH_SECRET:?...}` (REQUIRED — compose fails without it).
Run:
    cd paperclip/docker
    export BETTER_AUTH_SECRET=<random-32+ char secret, NOT committed>
    export PAPERCLIP_PUBLIC_URL=https://paperclip.starian.us
    docker compose up -d --build
Migrations: the prod Dockerfile builds `@paperclipai/db` (which copies SQL migrations into
dist) and the server applies them via `packages/db/src/migrate.ts` at boot against
`DATABASE_URL`. To run manually if needed: `pnpm db:migrate` (root) =
`pnpm --filter @paperclipai/db migrate` (requires `DATABASE_URL` set). `pnpm db:generate`
only needed when changing schema. Optional demo data: `pnpm --filter @paperclipai/db seed`.

The Dockerfile prod stage also npm-installs the agent CLIs globally
(`@anthropic-ai/claude-code`, `@openai/codex`, `opencode-ai`, `@google/gemini-cli`) so the
claude/codex/etc adapters work inside the container. Defaults baked in: `HOST=0.0.0.0`,
`PORT=3100`, `PAPERCLIP_HOME=/paperclip`, `PAPERCLIP_CONFIG=/paperclip/instances/default/config.json`.

### C. npx onboarding (interactive, writes a config; not ideal for headless prod)
    npx paperclipai onboard --yes              # trusted loopback
    npx paperclipai onboard --yes --bind lan   # or --bind tailnet (auth/private)
Good for laptop trials, not for our server flow.

### Env vars that matter (grepped from server/packages source)
Required-ish: `DATABASE_URL` (unset=PGlite), `PORT` (3100), `SERVE_UI`, `BETTER_AUTH_SECRET`
(auth mode), `PAPERCLIP_DEPLOYMENT_MODE` (authenticated|trusted), `PAPERCLIP_DEPLOYMENT_EXPOSURE`
(private|...), `PAPERCLIP_PUBLIC_URL`, `HOST`, `PAPERCLIP_HOME`, `PAPERCLIP_CONFIG`,
`PAPERCLIP_INSTANCE_ID`. Provider keys passed through to agent CLIs: `ANTHROPIC_API_KEY`,
`OPENAI_API_KEY`, etc. Telemetry on by default — disable with `PAPERCLIP_TELEMETRY_DISABLED=1`
or `DO_NOT_TRACK=1`. `.env.example` at root shows the dev defaults.

---

## 2. Adding a custom adapter (the core task) — `openxgram_peer`

There are TWO mechanisms. **Prefer the plugin package (no fork).**

### Adapter contract (what every adapter is)
A `ServerAdapterModule` object (type in `packages/adapter-utils/src/types.ts`):
- required: `type: string`, `execute(ctx): Promise<AdapterExecutionResult>`,
  `testEnvironment(ctx): Promise<AdapterEnvironmentTestResult>`.
- optional: `models`, `listModels`, `sessionCodec`, `agentConfigurationDoc`,
  `getConfigSchema`, `supportsLocalAgentJwt`, capability flags, `onHireApproved`, etc.

`execute(ctx: AdapterExecutionContext)` input fields:
- `runId: string`
- `agent: { id, companyId, name, adapterType, adapterConfig }`  ← our alias lives here
- `runtime: { sessionId, sessionParams, sessionDisplayId, taskKey }` (continuation)
- `config: Record<string,unknown>` (resolved adapterConfig, secrets already injected)
- `context: Record<string,unknown>` (the prompt/issue/org/skills payload to send)
- `onLog(stream:"stdout"|"stderr", chunk): Promise<void>` ← stream partial replies here
- `onMeta?`, `onSpawn?` (pid for orphan recovery — N/A for HTTP), `authToken?`

`execute` returns `AdapterExecutionResult`:
- `exitCode: number|null`, `signal: string|null`, `timedOut: boolean`
- `errorMessage?`, `errorCode?`, `errorFamily?` ("transient_upstream" → triggers retry),
  `retryNotBefore?`
- `usage?: { inputTokens, outputTokens, cachedInputTokens? }`
- `sessionId?/sessionParams?/sessionDisplayId?` (persisted, fed back next run)
- `provider?`, `biller?`, `model?`, `billingType?`, `costUsd?`, `resultJson?`, `summary?`
- `question?: { prompt, choices[] }` (ask a human → approval flow)

### Template = the HTTP adapter (`server/src/adapters/http/`)
`execute.ts` is tiny and is the exact pattern our peer adapter follows:
reads `config.url`, `config.method` (def POST), `config.headers`, `config.payloadTemplate`,
`config.timeoutMs`; POSTs `{ ...payloadTemplate, agentId, runId, context }` as JSON; non-2xx
throws; returns `{ exitCode:0, summary }`. On abort returns `{ timedOut:true, errorCode:"timeout" }`.
The process adapter (`process/execute.ts`) shows the spawn+`onLog`+`resultJson{stdout,stderr}`
shape if we ever want a CLI variant.

### Mechanism A — external adapter PLUGIN (recommended; no fork, survives upgrades)
The registry auto-loads external adapters from a plugin store
(`server/src/adapters/plugin-loader.ts` → `server/src/services/adapter-plugin-store.js`).
Contract (from `plugin-loader.ts` `validateAdapterModule`):
- Ship an npm package (or local `file:` path) whose main entry **exports
  `createServerAdapter()`** returning a `ServerAdapterModule` (must have `.type`).
- Optional `exports["./ui-parser"]` + `package.json` `paperclip.adapterUiParser:"1"` to
  render transcripts in the UI (contract major must be "1").
- Register it via the Adapter Plugin manager (UI `PluginManager`/`AdapterManager` page, or
  the plugin-store API) pointing at `@your/openxgram-peer-adapter` or a `file:` path.
  On boot `buildExternalAdapters()` loads every store record and the registry calls
  `registerServerAdapter(module)`. `reloadExternalAdapter(type)` hot-reloads in dev.
Our package's `createServerAdapter()` returns:
    { type: "openxgram_peer",
      execute,            // see below
      testEnvironment,    // ping OpenXgram health, return pass/fail checks
      models: [],
      agentConfigurationDoc: "...alias, baseUrl, timeoutSec..." }
`execute(ctx)` body: read `alias = config.alias`, `base = config.baseUrl`
(e.g. http://localhost:<oxg-gui-port>); build prompt string from `ctx.context`; then either
  - POST `${base}/v1/gui/orchestration/agents/${alias}/invoke` with `{ prompt, runId }`, OR
  - call peer_send + poll recv_messages.
Stream chunks via `await ctx.onLog("stdout", chunk)`; on success return
`{ exitCode:0, summary:<reply>, usage:{...}, sessionId:<conv id>, costUsd:<n> }`; on
upstream/network failure return `{ exitCode:1, errorFamily:"transient_upstream", errorMessage }`.

### Mechanism B — built-in (fork edit; only if a plugin can't reach OpenXgram)
The registry is now mutable (this is the HenkDz externalize-hermes fork). Public functions in
`server/src/adapters/registry.ts`: `registerServerAdapter(adapter)`,
`unregisterServerAdapter(type)`, `requireServerAdapter`, `getServerAdapter` (falls back to
`processAdapter`), `findServerAdapter`, `findActiveServerAdapter`. Built-ins are loaded into a
`Map` by `registerBuiltInAdapters()`. To hardcode: create `server/src/adapters/openxgram-peer/`
({execute.ts,test.ts,index.ts} mirroring `http/`), import it in `registry.ts`, and add it to
the `registerBuiltInAdapters()` array. Also note: `server/src/routes/agents.ts`
`assertKnownAdapterType()`/`findServerAdapter()` gate which adapter strings are accepted on
agent create/patch — a registered type passes automatically (shared validation already accepts
any non-empty string per `packages/shared/src/adapter-type.ts`). Plugin route = zero fork; pick B
only as fallback.

---

## 3. Built-in hermes / codex adapters (config)

This is the HenkDz `feat/externalize-hermes-adapter` fork: **Hermes is NOT built-in** here — it
ships as plugin `@henkey/hermes-paperclip-adapter` (or a `file:` path) installed via the same
Adapter Plugin manager as §2-A. (Registry still references `hermes_local` for the externalize
shim, but core has no hermes dependency.) Codex IS built-in: `type:"codex_local"`,
`execute=codexExecute`, runtime cmd `buildNpmRuntimeCommandSpec(config,"codex","@openai/codex")`
— i.e. it spawns the `codex` CLI (npm-installed in the prod image). To use OUR hermes/codex,
two options: (a) point the built-in `codex_local`/installed-hermes adapter's command/config at
our binaries, or (b) wrap them behind our `openxgram_peer` adapter so paperclip just talks HTTP
to OpenXgram and OpenXgram drives hermes/codex. (b) keeps paperclip vendor-clean.

---

## 4. Defining agents / roles / assignment

Agent = a row in the `agents` table: key columns `companyId`, `name`, `role` (free text, default
"general"), `title`, `status`, `adapterType` (default "process"), `adapterConfig` (jsonb),
`reportsTo` (self-FK → org hierarchy), `budgetMonthlyCents`. Three creation paths:

- **Seed** (`packages/db/src/seed.ts`): inserts a company + CEO + Engineer with
  `reportsTo:ceo.id`, `adapterType:"process"`, `adapterConfig:{command,args}`. Copy this shape,
  set `adapterType:"openxgram_peer"`, `adapterConfig:{alias:"<peer alias>", baseUrl:"..."}`.
- **REST API**: `POST /api/companies/:companyId/agents` with body validated by
  `createAgentSchema` (fields incl. name, role, title, adapterType, adapterConfig, reportsTo,
  budgetMonthlyCents). Hiring flow: `POST /api/companies/:companyId/agent-hires`. Update:
  `PATCH /api/agents/:id`. List: `GET /api/companies/:companyId/agents`. Self:
  `GET /api/agents/me`, inbox `GET /api/agents/me/inbox/mine`.
- **CLI** (`cli/src/commands/client/agent.ts`): `paperclipai ... agent create -C <companyId>
  --payload-json '{...createAgentSchema...}'`, `agent list -C <id>`, `agent patch`, etc.
  (`paperclipai` = `node cli/.../tsx cli/src/index.ts`.) Needs `PAPERCLIP_COMPANY_ID`/`-C`.

Issue assignment = delegation: set `issues.assigneeAgentId` (via issues routes
`server/src/routes/issues.ts`, `issues-checkout-wakeup.ts`) → wakes that agent. Parallel
fan-out, locks, routines, budget, approvals: see companion doc
`paperclip-orchestration-extraction.md` (already in this dir) §2–6.

Integration play: for each OpenXgram `list_peers` entry, create one paperclip agent with
`adapterType:"openxgram_peer"`, `adapterConfig:{alias}`, and map primary→sub-agent as
`reportsTo` (oxg.md §6 #7: only a machine primary is reachable directly).

---

## 5. Ports / reverse proxy (Caddy coexistence)

Paperclip binds ONE port (3100, single-process API+UI). Our Caddy already fronts
`xgram.starian.us`. Recommendation: run paperclip on `127.0.0.1:3100` (compose maps
`3100:3100`; restrict to localhost or tailnet — do NOT expose 5432/3100 publicly), add a Caddy
site block for `paperclip.starian.us` reverse-proxying to `127.0.0.1:3100`, and set
`PAPERCLIP_PUBLIC_URL=https://paperclip.starian.us` + keep
`PAPERCLIP_DEPLOYMENT_EXPOSURE=private` with a real `BETTER_AUTH_SECRET`. The Postgres port
(5432) should NOT be published outside the compose network in prod — drop the `ports:` mapping
on `db` if only the server needs it. The `openxgram_peer` adapter then calls OpenXgram's GUI/
peer HTTP on the loopback of the same host (no extra proxy needed).

---

## Risks / cautions
- **DB**: prod needs Postgres 17 (compose provides it) — don't rely on PGlite for prod.
  `BETTER_AUTH_SECRET` is mandatory in authenticated mode; generate a strong random value,
  keep it out of git/logs (store in vault).
- **Resources**: prod image npm-installs 4 agent CLIs globally → larger image + build time;
  the embedded-postgres beta is patched (`patches/`), so build from this repo, not ad-hoc.
- **Fork drift**: this is HenkDz's fork (hermes externalized). Treat `AGENTS.md §11` as
  authoritative; upstream docs may differ.
- **License**: MIT confirmed (root LICENSE + package licenses) — compatible with our stack.
- **Plugin > fork**: the `createServerAdapter()` plugin route needs zero source changes and
  survives `git pull`; the parent should implement `openxgram_peer` as a plugin package.
- **Secrets**: never bake provider keys / BETTER_AUTH_SECRET into the image or compose file —
  pass via env from a secret store at runtime.

Clone used for extraction was at /tmp/paperclip-inspect and removed after.
