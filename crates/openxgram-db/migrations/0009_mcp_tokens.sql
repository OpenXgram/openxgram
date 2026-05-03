-- MCP HTTP transport caller 인증 — Bearer 토큰 ↔ agent 매핑.
-- 평문 토큰은 발급 직후만 노출 (마스터가 클라이언트에 설정).
-- DB 에는 SHA-256 해시만 저장 (DB 유출 시 토큰 복구 불가).

CREATE TABLE mcp_tokens (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    agent TEXT NOT NULL,
    label TEXT,                         -- 마스터 메모용 (예: "claude-code-laptop")
    created_at TEXT NOT NULL,
    last_used TEXT
);

CREATE INDEX idx_mcp_tokens_agent ON mcp_tokens(agent);
CREATE INDEX idx_mcp_tokens_hash ON mcp_tokens(token_hash);
