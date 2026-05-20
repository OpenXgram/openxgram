-- UI-MEMORY-SPEC v1.1 깊은 구현 — 휴지통, 태그, 카테고리, 페이지 잠금, 이력, 공유, 패턴/실수 보드.

-- M-12 V-4 휴지통 (30일 보관, 1일 전 알림).
CREATE TABLE IF NOT EXISTS wiki_trash (
    id              TEXT PRIMARY KEY,        -- 원래 wiki_pages.id
    title           TEXT,
    page_type       TEXT,
    content         TEXT,                    -- markdown body snapshot
    deleted_at      TEXT NOT NULL,
    purge_at        TEXT NOT NULL            -- deleted_at + 30 days
);

-- M-7 페이지 잠금 (사용자 표시 페이지 — AI 수정 금지).
CREATE TABLE IF NOT EXISTS wiki_locks (
    page_id         TEXT PRIMARY KEY,
    locked_by       TEXT NOT NULL,           -- "user" | "ai"
    locked_at       TEXT NOT NULL,
    reason          TEXT
);

-- M-11 편집 이력 영구 보관 (V-1 merge 이벤트 포함).
CREATE TABLE IF NOT EXISTS wiki_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id         TEXT NOT NULL,
    revision        INTEGER NOT NULL,
    content_hash    TEXT NOT NULL,
    title           TEXT,
    content         TEXT,                    -- snapshot
    author          TEXT NOT NULL,           -- "user" | "ai"
    event_type      TEXT NOT NULL DEFAULT 'edit', -- edit | create | merge
    merge_source_id TEXT,                    -- M-2 V-1 merge 시 합쳐진 다른 페이지 id
    at              TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wiki_history_page ON wiki_history(page_id, revision DESC);

-- M-8 V-6 카테고리 트리 (최대 5단) + V-7 태그.
-- wiki_pages 에 columns 추가 — 기존 테이블이라 ALTER.
ALTER TABLE wiki_pages ADD COLUMN category_path TEXT DEFAULT '';
ALTER TABLE wiki_pages ADD COLUMN tags TEXT DEFAULT '[]'; -- JSON array
ALTER TABLE wiki_pages ADD COLUMN authors TEXT DEFAULT '[]'; -- M-1 누가 썼는지

-- M-4 V-3 페이지 공유.
CREATE TABLE IF NOT EXISTS wiki_shares (
    id              TEXT PRIMARY KEY,        -- share token
    page_id         TEXT NOT NULL,
    mode            TEXT NOT NULL,           -- 'public' | 'secret' | 'password'
    password_hash   TEXT,                    -- mode='password' 시
    expires_at      TEXT,                    -- NULL = 무기한
    created_at      TEXT NOT NULL,
    noindex         INTEGER NOT NULL DEFAULT 1  -- V-12 robots noindex 기본
);

-- M-5 V-5 패턴 보드 (AI 발견 + 사용자 추가).
CREATE TABLE IF NOT EXISTS memory_patterns (
    id              TEXT PRIMARY KEY,
    pattern_type    TEXT NOT NULL,           -- 'behavior' | 'utterance' | 'preference'
    description     TEXT NOT NULL,
    confidence      REAL NOT NULL DEFAULT 1.0, -- AI 발견 = 점수, 사용자 = 1.0
    source          TEXT NOT NULL,           -- 'ai' | 'user'
    examples        TEXT,                    -- JSON array
    created_at      TEXT NOT NULL
);

-- M-13 V-9 실수 보드 (3가지 발견 방식).
CREATE TABLE IF NOT EXISTS memory_mistakes (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    description     TEXT NOT NULL,
    discovery_method TEXT NOT NULL,           -- 'user_edit_diff' | 'llm_conflict' | 'user_explicit'
    context         TEXT,                    -- 어느 메시지/대화에서 발견
    resolved        INTEGER NOT NULL DEFAULT 0,
    resolved_at     TEXT,
    created_at      TEXT NOT NULL
);

-- M-6 신규 페이지 알림 큐 (수정은 X — M-6 정책).
CREATE TABLE IF NOT EXISTS wiki_new_alerts (
    page_id         TEXT PRIMARY KEY,
    title           TEXT,
    notified_at     TEXT,
    created_at      TEXT NOT NULL
);
