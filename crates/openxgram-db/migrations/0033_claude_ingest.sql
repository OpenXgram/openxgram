-- v33: Claude Code .jsonl 자동 ingestion 상태 추적
CREATE TABLE IF NOT EXISTS claude_ingest_state (
    file_path TEXT PRIMARY KEY,
    last_offset INTEGER NOT NULL DEFAULT 0,
    session_db_id TEXT,
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    msg_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_claude_ingest_seen ON claude_ingest_state(last_seen_at DESC);
