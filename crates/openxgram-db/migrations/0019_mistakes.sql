-- 0019_mistakes.sql — 실수 레지스트리 (W의 규칙 1)
-- 정본: docs/PRD-OpenXgram.md §4.2 (openxgram-mistakes crate)
--
-- "내가 한 모든 것을 체계적으로 기록하고 벡터 검색해서, 같은 실수를 반복하지 않는 것."
--
-- 절대 규칙 1 (fallback 금지): 모든 INSERT/UPDATE는 명시적 검증.
-- 절대 규칙 3 (DB 변경 마스터 승인): 신규 CREATE TABLE만, 기존 데이터 무영향.

CREATE TABLE IF NOT EXISTS mistakes (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    occurred_at     INTEGER NOT NULL,           -- unix epoch ms
    intended_action TEXT NOT NULL,              -- 하려던 것
    actual_outcome  TEXT NOT NULL,              -- 실제로 일어난 일
    failure_reason  TEXT NOT NULL,              -- 왜 실패했는가
    lesson          TEXT NOT NULL,              -- 다음에 어떻게 다르게 할지
    severity        INTEGER NOT NULL DEFAULT 5  -- 1 (사소) ~ 10 (치명)
        CHECK (severity BETWEEN 1 AND 10),
    resolved        INTEGER NOT NULL DEFAULT 0  -- 0=open, 1=resolved
        CHECK (resolved IN (0, 1)),
    resolution      TEXT,                       -- resolved=1일 때 해결 내용
    related_wiki    TEXT,                       -- 관련 위키 페이지 id (page_id 또는 path)
    embedding_hash  TEXT NOT NULL,              -- 임베딩 재생성 트리거
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mistakes_session ON mistakes(session_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_mistakes_severity ON mistakes(severity DESC, resolved, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_mistakes_unresolved ON mistakes(resolved, severity DESC) WHERE resolved = 0;

-- 임베딩 벡터 (wiki와 동일 패턴 — plain BLOB부터 시작, vec0 도입은 후속).
CREATE TABLE IF NOT EXISTS mistake_embeddings (
    mistake_id TEXT PRIMARY KEY,
    embedding  BLOB NOT NULL,
    dim        INTEGER NOT NULL DEFAULT 384,
    model      TEXT NOT NULL DEFAULT 'bge-small',
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (mistake_id) REFERENCES mistakes(id) ON DELETE CASCADE
);
