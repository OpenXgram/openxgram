-- 0018_wiki_pages.sql — L2 위키 페이지 인덱스
-- 정본: docs/PRD-OpenXgram.md §4.1 (openxgram-wiki crate)
--
-- 디스크가 정본. 본 테이블은 인덱스:
--   - file_path: {XGRAM_DATA_DIR}/wiki/{type}/{slug}.md 상대 경로
--   - content_hash: 디스크 본문(frontmatter 제외) SHA-256
--   - embedding_hash: content_hash 변경 시 재임베딩 트리거
--
-- 절대 규칙 1 (fallback 금지): silent overwrite 차단을 위해 낙관 잠금 사용
-- (openxgram-wiki::store::WikiStore::upsert의 expected_hash 인자).
--
-- 절대 규칙 3 (DB 변경 마스터 승인): 본 마이그레이션은 신규 CREATE TABLE만 — 기존 데이터 무영향.
-- 적용 전 마스터 승인 필수.

CREATE TABLE IF NOT EXISTS wiki_pages (
    id              TEXT PRIMARY KEY,
    file_path       TEXT UNIQUE NOT NULL,
    page_type       TEXT NOT NULL,            -- entity / concept / comparison / other
    title           TEXT NOT NULL,
    content_hash    TEXT NOT NULL,             -- 본문 SHA-256 (frontmatter 제외)
    related         TEXT,                      -- JSON array of related page ids
    source_refs     TEXT,                      -- JSON array of msg:* / episode:* refs
    embedding_hash  TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wiki_pages_type
    ON wiki_pages(page_type, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_wiki_pages_updated
    ON wiki_pages(updated_at DESC);

-- 임베딩 벡터 (sqlite-vec 활성 시 vec0 가상 테이블로 대체 — Phase v0.3 후속).
-- 본 마이그레이션에서는 plain BLOB 테이블로 시작하여 sqlite-vec 도입과 분리.
CREATE TABLE IF NOT EXISTS wiki_embeddings (
    page_id    TEXT PRIMARY KEY,
    embedding  BLOB NOT NULL,
    dim        INTEGER NOT NULL DEFAULT 384,
    model      TEXT NOT NULL DEFAULT 'bge-small',
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (page_id) REFERENCES wiki_pages(id) ON DELETE CASCADE
);
