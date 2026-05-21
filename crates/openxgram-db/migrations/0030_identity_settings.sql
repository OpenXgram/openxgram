-- v30: M-2 auto_lock_minutes editable + M-10 suspicious_dids alerts
-- IMPLEMENTATION-CHECKLIST.md UI-IDENTITY-SPEC

CREATE TABLE IF NOT EXISTS identity_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO identity_settings (key, value) VALUES
    ('auto_lock_minutes', '30'),
    ('session_token_ttl_minutes', '30');

-- M-10: 새 DID 가 외부에서 들어오면 의심 alert 적재
CREATE TABLE IF NOT EXISTS identity_suspicious_dids (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_did TEXT NOT NULL,
    reason TEXT NOT NULL,
    first_seen TEXT NOT NULL DEFAULT (datetime('now')),
    dismissed INTEGER NOT NULL DEFAULT 0,
    dismissed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_susp_dids_active ON identity_suspicious_dids(dismissed, first_seen DESC);
