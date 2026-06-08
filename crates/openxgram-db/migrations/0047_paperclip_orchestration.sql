-- rc.276 — Paperclip orchestration absorption, Phase 1 (core entities / schema).
-- Source spec: docs/research/paperclip-orchestration-extraction.md §1 + integration plan + Phase 1.
-- Thin overlay on existing peer / agent_capabilities identity. company-scoped (maker_id rule mirror).
-- Migration runner (migrate.rs) graceful-skips "duplicate column name" / "already exists",
-- so plain ALTER ADD COLUMN + CREATE TABLE IF NOT EXISTS are idempotent here.

-- 1) companies — org container (tenant).
CREATE TABLE IF NOT EXISTS companies (
    id                   TEXT PRIMARY KEY,
    name                 TEXT NOT NULL,
    description          TEXT,
    status               TEXT NOT NULL DEFAULT 'active',  -- active | paused
    pause_reason         TEXT,
    paused_at            TEXT,
    issue_prefix         TEXT,                            -- per-company issue numbering prefix (e.g. "PAP")
    issue_counter        INTEGER NOT NULL DEFAULT 0,
    budget_monthly_cents INTEGER,
    spent_monthly_cents  INTEGER NOT NULL DEFAULT 0,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);

-- 2) agent_capabilities extension — org-chart node overlay.
-- Existing columns (alias/role/description/capabilities/orchestration_role/group_name/...) preserved.
ALTER TABLE agent_capabilities ADD COLUMN company_id TEXT;
ALTER TABLE agent_capabilities ADD COLUMN reports_to TEXT;                       -- self-ref alias (org hierarchy)
ALTER TABLE agent_capabilities ADD COLUMN adapter_type TEXT DEFAULT 'peer_send'; -- peer_send | process | http
ALTER TABLE agent_capabilities ADD COLUMN adapter_config TEXT;                   -- JSON, e.g. {"alias":"..."}
ALTER TABLE agent_capabilities ADD COLUMN budget_monthly_cents INTEGER;
ALTER TABLE agent_capabilities ADD COLUMN status TEXT;                           -- idle | running | paused | ...
ALTER TABLE agent_capabilities ADD COLUMN paused_at TEXT;

CREATE INDEX IF NOT EXISTS idx_agent_caps_company ON agent_capabilities(company_id);
CREATE INDEX IF NOT EXISTS idx_agent_caps_reports_to ON agent_capabilities(reports_to);

-- 3) goals (+ parent_id ancestry tree).
CREATE TABLE IF NOT EXISTS goals (
    id              TEXT PRIMARY KEY,
    company_id      TEXT,
    title           TEXT NOT NULL,
    description     TEXT,
    level           TEXT NOT NULL DEFAULT 'task',         -- task | objective | ...
    status          TEXT NOT NULL DEFAULT 'backlog',
    parent_id       TEXT REFERENCES goals(id) ON DELETE SET NULL,  -- goal ancestry
    owner_agent_id  TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_goals_company ON goals(company_id);
CREATE INDEX IF NOT EXISTS idx_goals_parent ON goals(parent_id);

-- 3b) projects.
CREATE TABLE IF NOT EXISTS projects (
    id            TEXT PRIMARY KEY,
    company_id    TEXT,
    goal_id       TEXT REFERENCES goals(id) ON DELETE SET NULL,
    name          TEXT NOT NULL,
    description   TEXT,
    status        TEXT NOT NULL DEFAULT 'backlog',
    lead_agent_id TEXT,
    env           TEXT,                                   -- JSON
    target_date   TEXT,
    archived_at   TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_projects_company ON projects(company_id);
CREATE INDEX IF NOT EXISTS idx_projects_goal ON projects(goal_id);

-- 3c) project_goals — M:N join.
CREATE TABLE IF NOT EXISTS project_goals (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    goal_id    TEXT NOT NULL REFERENCES goals(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, goal_id)
);

-- 4) issues — unit of work (+ parent_id tree, assignee, atomic checkout lock cols).
CREATE TABLE IF NOT EXISTS issues (
    id                  TEXT PRIMARY KEY,
    company_id          TEXT,
    project_id          TEXT REFERENCES projects(id) ON DELETE SET NULL,
    goal_id             TEXT REFERENCES goals(id) ON DELETE SET NULL,
    parent_id           TEXT REFERENCES issues(id) ON DELETE SET NULL,  -- issue tree / child fan-out
    title               TEXT NOT NULL,
    body                TEXT,
    status              TEXT NOT NULL DEFAULT 'backlog',  -- backlog | in_progress | done | ...
    priority            INTEGER NOT NULL DEFAULT 0,
    assignee_agent_id   TEXT,                             -- delegation target
    checkout_run_id     TEXT,                             -- atomic checkout lock (run holding the issue)
    execution_locked_at TEXT,                             -- lock timestamp
    origin_kind         TEXT,                             -- manual | routine | webhook | ...
    origin_fingerprint  TEXT,                             -- dedup
    request_depth       INTEGER NOT NULL DEFAULT 0,       -- delegation depth guard
    issue_number        INTEGER,
    identifier          TEXT,                             -- e.g. PAP-123
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_issues_company_status ON issues(company_id, status);
CREATE INDEX IF NOT EXISTS idx_issues_parent ON issues(parent_id);
CREATE INDEX IF NOT EXISTS idx_issues_assignee ON issues(assignee_agent_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_origin_fingerprint
    ON issues(company_id, origin_fingerprint)
    WHERE origin_fingerprint IS NOT NULL;

-- 5) issue_relations — dependency edges (DAG).
CREATE TABLE IF NOT EXISTS issue_relations (
    id              TEXT PRIMARY KEY,
    company_id      TEXT,
    issue_id        TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    related_issue_id TEXT NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    relation_type   TEXT NOT NULL DEFAULT 'blocks',       -- blocks | ...
    created_at      TEXT NOT NULL,
    UNIQUE(company_id, issue_id, related_issue_id, relation_type)
);
CREATE INDEX IF NOT EXISTS idx_issue_relations_issue ON issue_relations(issue_id);
CREATE INDEX IF NOT EXISTS idx_issue_relations_related ON issue_relations(related_issue_id);

-- 5b) activity_log — unified event feed.
CREATE TABLE IF NOT EXISTS activity_log (
    id          TEXT PRIMARY KEY,
    company_id  TEXT,
    actor       TEXT,                                     -- actor (agent alias / user / system)
    kind        TEXT NOT NULL,                            -- action / event kind
    target      TEXT,                                     -- entity reference (e.g. issue:ID)
    payload     TEXT,                                     -- JSON details
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_activity_log_company ON activity_log(company_id, created_at);
CREATE INDEX IF NOT EXISTS idx_activity_log_target ON activity_log(target);
