-- rc.92 D1 — 에이전트 발견 + 역할 라우팅 메타.
-- register_subagent 호출 시 capabilities/description 저장 → list_peers 가 그 메타 반환 →
-- Claude system prompt 에 자동 inject → request_help 가 capabilities 매칭 라우팅.

CREATE TABLE IF NOT EXISTS agent_capabilities (
    alias        TEXT PRIMARY KEY,        -- register_subagent 가 발급한 alias
    role         TEXT NOT NULL,           -- 짧은 역할명 (예: researcher, writer, portal-dev)
    description  TEXT,                    -- 1~3 문장: "이 에이전트는 X 를 잘함"
    capabilities TEXT,                    -- JSON array: ["web_search", "code_review", ...]
    tool_list    TEXT,                    -- JSON array of MCP tool names (자동 추출)
    project_path TEXT,                    -- 작업 디렉토리 (있으면)
    updated_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_caps_role ON agent_capabilities(role);
