-- rc.132 — agent_templates: agency-agents (msitarzewski/agency-agents) 카탈로그.
-- 첫 daemon 시작 시 자동 seed. GUI 에서 "🔄 갱신" 시 UPSERT.
-- 사용자 수정한 row 는 customized=1 → 갱신 시 보존.

CREATE TABLE IF NOT EXISTS agent_templates (
    id              TEXT PRIMARY KEY,         -- '{source_repo}::{source_path}' 또는 user-custom UUID
    source_repo     TEXT NOT NULL,            -- 'msitarzewski/agency-agents' / 'user-custom' 등
    source_path     TEXT,                     -- 원본 path (예: 'engineering/engineering-ai-engineer.md')
    category        TEXT NOT NULL,            -- 'engineering' / 'design' / ...
    name            TEXT NOT NULL,            -- 'AI Engineer'
    description     TEXT,                     -- frontmatter description
    color           TEXT,                     -- frontmatter color
    emoji           TEXT,                     -- frontmatter emoji
    vibe            TEXT,                     -- frontmatter vibe
    body            TEXT NOT NULL,            -- 전체 markdown body (frontmatter 제외)
    customized      INTEGER NOT NULL DEFAULT 0, -- 1 = 사용자 수정, 갱신 시 보존
    fetched_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_templates_category ON agent_templates(category);
CREATE INDEX IF NOT EXISTS idx_agent_templates_customized ON agent_templates(customized);
