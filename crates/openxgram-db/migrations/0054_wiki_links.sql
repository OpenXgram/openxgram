-- LLM 위키(Karpathy 패턴) Phase 1 — 페이지 간 [[wikilink]] 연결 + backlink.
-- 위키 upsert 시 본문의 [[제목]] 을 파싱해 채운다. to_id 는 같은 제목 페이지 있으면 해석.
CREATE TABLE IF NOT EXISTS wiki_links (
    from_id    TEXT NOT NULL,          -- 링크를 건 페이지 id
    to_title   TEXT NOT NULL,          -- [[...]] 안의 제목
    to_id      TEXT,                   -- 해석된 대상 페이지 id (없으면 NULL = 빨간링크)
    created_at TEXT NOT NULL,
    PRIMARY KEY (from_id, to_title)
);
CREATE INDEX IF NOT EXISTS idx_wiki_links_to_title ON wiki_links(to_title);
CREATE INDEX IF NOT EXISTS idx_wiki_links_to_id ON wiki_links(to_id);
CREATE INDEX IF NOT EXISTS idx_wiki_links_from ON wiki_links(from_id);
