-- UI-EXTERNAL-AGENT-SPEC v1.0 — 외부 에이전트 카드 (30 결정).
CREATE TABLE IF NOT EXISTS external_outbound_calls (
    id              TEXT PRIMARY KEY,
    to_agent        TEXT NOT NULL,
    protocol        TEXT NOT NULL,
    amount          REAL NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'pending',
    rating          INTEGER,
    started_at      TEXT NOT NULL,
    completed_at    TEXT
);
CREATE TABLE IF NOT EXISTS external_inbound_pending (
    id                  TEXT PRIMARY KEY,
    from_agent          TEXT NOT NULL,
    protocol            TEXT NOT NULL,
    request_summary     TEXT,
    offered_price       REAL,
    status              TEXT NOT NULL DEFAULT 'pending',
    received_at         TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS external_listings (
    id              TEXT PRIMARY KEY,
    agent_id        TEXT NOT NULL,
    marketplace     TEXT NOT NULL,
    price_usdc      REAL NOT NULL DEFAULT 0,
    pricing_model   TEXT NOT NULL DEFAULT 'per-call',
    description     TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS external_reputation (
    external_agent      TEXT PRIMARY KEY,
    avg_rating          REAL,
    review_count        INTEGER NOT NULL DEFAULT 0,
    blacklisted         INTEGER NOT NULL DEFAULT 0,
    last_interaction    TEXT
);
CREATE TABLE IF NOT EXISTS external_protocols (
    name        TEXT PRIMARY KEY,
    enabled     INTEGER NOT NULL DEFAULT 0,
    configured_at TEXT
);
INSERT OR IGNORE INTO external_protocols (name, enabled, configured_at) VALUES
    ('openagentx', 0, datetime('now')),
    ('x402',       0, datetime('now')),
    ('a2a',        0, datetime('now')),
    ('anp',        0, datetime('now')),
    ('virtuals',   0, datetime('now'));
