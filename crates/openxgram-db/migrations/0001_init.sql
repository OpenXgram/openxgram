-- OpenXgram DB 초기화 마이그레이션
-- Phase 1: 스키마 정의 (placeholder). Phase 2에서 rusqlite 연동 시 적용.

-- 에이전트 신원 테이블
CREATE TABLE IF NOT EXISTS agent_identities (
    id          TEXT PRIMARY KEY,       -- 공개키 해시 (hex)
    alias       TEXT,                   -- 사람이 읽을 수 있는 별칭
    pubkey_hex  TEXT NOT NULL,          -- secp256k1 공개키 (hex)
    created_at  INTEGER NOT NULL        -- Unix timestamp (UTC)
);

-- L0: 원시 메시지
CREATE TABLE IF NOT EXISTS memory_messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL REFERENCES agent_identities(id),
    role        TEXT NOT NULL,          -- 'user' | 'assistant' | 'system'
    content     TEXT NOT NULL,
    ts          INTEGER NOT NULL        -- Unix timestamp (UTC)
);

-- L1: 에피소드 (컨텍스트 묶음)
CREATE TABLE IF NOT EXISTS memory_episodes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL REFERENCES agent_identities(id),
    summary     TEXT NOT NULL,
    start_ts    INTEGER NOT NULL,
    end_ts      INTEGER NOT NULL
);

-- L2: 의미 기억 (임베딩 벡터 — sqlite-vec 확장 필요)
-- TODO(Phase 2): CREATE VIRTUAL TABLE memory_vectors USING vec0(...)
CREATE TABLE IF NOT EXISTS memory_semantic_placeholder (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL,
    text        TEXT NOT NULL,
    embedding   BLOB                    -- f32 벡터 (sqlite-vec 연동 전 BLOB)
);

-- L3: 패턴
CREATE TABLE IF NOT EXISTS memory_patterns (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL REFERENCES agent_identities(id),
    pattern     TEXT NOT NULL,
    weight      REAL NOT NULL DEFAULT 1.0,
    updated_at  INTEGER NOT NULL
);

-- L4: 특성 (장기 페르소나)
CREATE TABLE IF NOT EXISTS memory_traits (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL REFERENCES agent_identities(id),
    trait_key   TEXT NOT NULL,
    trait_value TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Vault: 암호화 자격증명
CREATE TABLE IF NOT EXISTS vault_entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL REFERENCES agent_identities(id),
    key_name    TEXT NOT NULL,
    ciphertext  BLOB NOT NULL,          -- AES-256-GCM 암호화 (Phase 2 구현)
    tags        TEXT,                   -- JSON 배열
    created_at  INTEGER NOT NULL,
    UNIQUE(agent_id, key_name)
);

-- 인덱스
CREATE INDEX IF NOT EXISTS idx_messages_agent ON memory_messages(agent_id, ts DESC);
CREATE INDEX IF NOT EXISTS idx_episodes_agent ON memory_episodes(agent_id, end_ts DESC);
CREATE INDEX IF NOT EXISTS idx_vault_agent ON vault_entries(agent_id, key_name);
