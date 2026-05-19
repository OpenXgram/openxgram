-- 0020_action_patterns.sql — 패턴 매칭 제안 엔진 (W의 규칙 2)
-- 정본: docs/PRD-OpenXgram.md §4.3 (openxgram-patterns crate)
--
-- "내 행동을 패턴화하여, 새로운 행동이 어떤 패턴과 유사한지 매칭해서 다음 행동을 제안하는 것."
--
-- 기존 자산: L3 patterns(0004) — NEW/RECURRING/ROUTINE 분류 + frequency.
-- 본 마이그레이션은 행동 시퀀스 + 성공률 추적 + 임베딩 인덱스 추가.
--
-- 절대 규칙 3 (DB 변경 마스터 승인): 신규 CREATE TABLE만. 기존 patterns(0004) 무영향.

CREATE TABLE IF NOT EXISTS action_patterns (
    id              TEXT PRIMARY KEY,
    pattern_id      TEXT NOT NULL REFERENCES patterns(id) ON DELETE CASCADE,
    action_sequence TEXT NOT NULL,              -- JSON: [{"step": "...", "tool": "..."}, ...]
    avg_duration_ms INTEGER,                    -- 누적 평균 (성공 케이스만)
    success_count   INTEGER NOT NULL DEFAULT 0,
    failure_count   INTEGER NOT NULL DEFAULT 0,
    last_executed   INTEGER,                    -- epoch ms (성공·실패 무관 마지막 호출)
    embedding_hash  TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_action_patterns_pattern
    ON action_patterns(pattern_id);

CREATE INDEX IF NOT EXISTS idx_action_patterns_recent
    ON action_patterns(last_executed DESC) WHERE last_executed IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_action_patterns_success_rate
    ON action_patterns(success_count DESC);

-- 임베딩 (plain BLOB; sqlite-vec 도입은 후속).
CREATE TABLE IF NOT EXISTS action_pattern_embeddings (
    action_pattern_id TEXT PRIMARY KEY,
    embedding         BLOB NOT NULL,
    dim               INTEGER NOT NULL DEFAULT 384,
    model             TEXT NOT NULL DEFAULT 'bge-small',
    updated_at        INTEGER NOT NULL,
    FOREIGN KEY (action_pattern_id) REFERENCES action_patterns(id) ON DELETE CASCADE
);
