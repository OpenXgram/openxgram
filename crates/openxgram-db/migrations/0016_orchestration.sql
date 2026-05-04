-- Orchestration: scheduled messages + message chains (PRD-ORCH-01)
-- Multi-agent workflow primitives.
-- All timestamps in KST (Asia/Seoul, epoch seconds).

CREATE TABLE IF NOT EXISTS scheduled_messages (
    id TEXT PRIMARY KEY,
    target_kind TEXT NOT NULL,           -- 'role' | 'platform'
    target TEXT NOT NULL,                -- "res" or "discord:CHANNEL_ID"
    payload TEXT NOT NULL,
    msg_type TEXT NOT NULL DEFAULT 'info',
    schedule_kind TEXT NOT NULL,         -- 'once' | 'cron'
    schedule_value TEXT NOT NULL,        -- ISO8601 KST or cron expression
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | sent | failed | cancelled
    created_at_kst INTEGER NOT NULL,
    last_sent_at_kst INTEGER,
    last_error TEXT,
    next_due_at_kst INTEGER,
    audit_row_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_scheduled_due
    ON scheduled_messages(status, next_due_at_kst);

CREATE TABLE IF NOT EXISTS message_chains (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at_kst INTEGER NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS chain_steps (
    id TEXT PRIMARY KEY,
    chain_id TEXT NOT NULL REFERENCES message_chains(id) ON DELETE CASCADE,
    step_order INTEGER NOT NULL,
    target_kind TEXT NOT NULL,
    target TEXT NOT NULL,
    payload TEXT NOT NULL,
    delay_secs INTEGER NOT NULL DEFAULT 0,
    condition_kind TEXT,                 -- NULL | 'always' | 'response_contains' | 'response_not_contains'
    condition_value TEXT,
    UNIQUE(chain_id, step_order)
);

CREATE INDEX IF NOT EXISTS idx_chain_steps_chain
    ON chain_steps(chain_id, step_order);
