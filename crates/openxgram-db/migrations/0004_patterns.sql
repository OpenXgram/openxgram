-- L3 patterns — 반복 행동·발화 분류 (NEW / RECURRING / ROUTINE)
-- 분류는 frequency 기반 (column 저장 안 함, query 시 derive). PRD §7 임계값:
--   1: NEW · 2~4: RECURRING · 5+: ROUTINE.

CREATE TABLE patterns (
    id TEXT PRIMARY KEY,
    pattern_text TEXT NOT NULL UNIQUE,
    frequency INTEGER NOT NULL DEFAULT 1,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_patterns_frequency ON patterns(frequency);
CREATE INDEX idx_patterns_last_seen ON patterns(last_seen);
