-- L1 episodes — session reflection 결과
-- Phase 1: 1 session = 1 episode 단순 집계. Phase 1.5 에서 시간/주제 기반 분할.
CREATE TABLE episodes (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    message_count INTEGER NOT NULL,
    summary TEXT NOT NULL,
    created_at TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_episodes_session ON episodes(session_id, started_at);
