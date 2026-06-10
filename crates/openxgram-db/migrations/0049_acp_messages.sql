-- ACP 대화 영속화 — 새로고침/데몬 재시작 후에도 대화가 기록·복원되도록.
-- conv_key = 에이전트 식별자(보통 alias). role = 'me'|'agent'|'note'.
CREATE TABLE IF NOT EXISTS acp_messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    conv_key   TEXT NOT NULL,
    role       TEXT NOT NULL,
    text       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_acp_messages_conv ON acp_messages(conv_key, id);
