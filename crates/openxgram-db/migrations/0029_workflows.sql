-- UI-MESSENGER-SPEC v1.4 §20 — 오케스트레이션 워크플로 (W-1~W-10).
CREATE TABLE IF NOT EXISTS workflows (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT,
    yaml_body       TEXT NOT NULL,                -- W-1 YAML 저장
    orchestrator    TEXT,                          -- W-10 오케스트레이터 에이전트 alias
    cron_expr       TEXT,                          -- W-4/W-9 cron 시작 (옵션)
    message_trigger TEXT,                          -- W-5 메시지 트리거 (옵션, json)
    cost_limit      REAL,                          -- W-8 비용 한도 USDC
    enabled         INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_runs (
    id              TEXT PRIMARY KEY,
    workflow_id     TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    status          TEXT NOT NULL DEFAULT 'running',  -- running | success | failed | aborted | waiting_human
    current_step    TEXT,
    error           TEXT,
    total_cost      REAL DEFAULT 0,
    trigger_source  TEXT,                              -- cron | message | manual
    FOREIGN KEY (workflow_id) REFERENCES workflows(id)
);
CREATE TABLE IF NOT EXISTS workflow_step_logs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id          TEXT NOT NULL,
    step_name       TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    status          TEXT NOT NULL,
    output_preview  TEXT,
    cost            REAL DEFAULT 0,
    FOREIGN KEY (run_id) REFERENCES workflow_runs(id)
);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow ON workflow_runs(workflow_id, started_at DESC);
