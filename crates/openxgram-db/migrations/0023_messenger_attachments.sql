-- UI-MESSENGER-SPEC v1.3 S7 — 첨부 파일 저장 (Vault 아님, S7 별도 disk).
-- V2: content-addressed (thread_id prefix 제거).
-- V3: refcount immediate unlink (lazy GC 아님).

CREATE TABLE IF NOT EXISTS attachment_refs (
    content_hash    TEXT PRIMARY KEY,    -- SHA-256 hex
    refcount        INTEGER NOT NULL DEFAULT 0,
    size_bytes      INTEGER NOT NULL DEFAULT 0,
    mime            TEXT,
    created_at      TEXT NOT NULL
);

-- inline blob (< 1MB) 는 messages 테이블에 직접 column 추가 가능하지만 호환성 위해 별 테이블.
CREATE TABLE IF NOT EXISTS attachment_inline (
    content_hash    TEXT PRIMARY KEY,
    data            BLOB NOT NULL,
    mime            TEXT,
    size_bytes      INTEGER NOT NULL
);

-- M-5 화이트리스트 패턴 (지금은 default 만 노출, 사용자 추가 패턴 보관용).
CREATE TABLE IF NOT EXISTS whitelist_patterns (
    id              TEXT PRIMARY KEY,
    priority        INTEGER NOT NULL DEFAULT 1,
    pattern_type    TEXT NOT NULL,        -- 'command' | 'tmux' | 'cwd'
    pattern         TEXT NOT NULL,
    default_role    TEXT NOT NULL,
    auto_register   INTEGER NOT NULL DEFAULT 0,
    auto_approve_pending INTEGER NOT NULL DEFAULT 0,
    active          INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

-- M-5 자동 등록 audit
CREATE TABLE IF NOT EXISTS whitelist_match_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id        TEXT NOT NULL,
    matched_pattern_id TEXT,
    action          TEXT NOT NULL,        -- 'auto_register' | 'manual_approve_pending'
    at              TEXT NOT NULL
);
