-- OpenXgram 초기 스키마 (Phase 1 MVP)
-- sessions, messages, memories, contacts, share_policy
-- 모든 timestamp: ISO8601 with KST offset (+09:00)

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    participants TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    last_active TEXT NOT NULL,
    home_machine TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_sessions_last_active ON sessions(last_active);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sender TEXT NOT NULL,
    body TEXT NOT NULL,
    signature TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    parent_message_id TEXT,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_messages_session ON messages(session_id, timestamp);
CREATE INDEX idx_messages_sender ON messages(sender, timestamp);

CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('fact', 'decision', 'reference', 'rule')),
    content TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0,
    importance REAL NOT NULL DEFAULT 0.5,
    access_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_accessed TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_memories_session ON memories(session_id);
CREATE INDEX idx_memories_kind_pinned ON memories(kind, pinned);

CREATE TABLE contacts (
    address TEXT PRIMARY KEY,
    alias TEXT NOT NULL UNIQUE,
    role TEXT,
    machine TEXT,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_contacts_alias ON contacts(alias);
CREATE INDEX idx_contacts_role ON contacts(role);

CREATE TABLE share_policy (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_agent TEXT NOT NULL,
    target_agent TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('push', 'pull', 'both', 'none')),
    scope TEXT NOT NULL DEFAULT 'all',
    realtime INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (source_agent, target_agent, scope)
);
