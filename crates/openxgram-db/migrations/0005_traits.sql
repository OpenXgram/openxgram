-- L4 traits — 정체성·성향. PRD §6 L4.
-- source: 'derived' (야간 reflection 자동 도출) 또는 'manual' (마스터 수동 편집).
-- source_refs: JSON array of pattern/memory id (derived 의 출처 추적).

CREATE TABLE traits (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('derived', 'manual')),
    source_refs TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_traits_source ON traits(source);
CREATE INDEX idx_traits_updated_at ON traits(updated_at);
