-- v31: role_policies — auto_respond 마스터 정책 편집 가능
CREATE TABLE IF NOT EXISTS role_policies (
    role TEXT PRIMARY KEY,
    auto_respond_default INTEGER NOT NULL DEFAULT 0,
    max_concurrent INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO role_policies (role, auto_respond_default, max_concurrent) VALUES
    ('researcher', 1, 3),
    ('reviewer', 0, 2),
    ('coder', 1, 2),
    ('orchestrator', 1, 5),
    ('scribe', 1, 1),
    ('analyst', 1, 2),
    ('tester', 0, 2),
    ('ops', 0, 1);
