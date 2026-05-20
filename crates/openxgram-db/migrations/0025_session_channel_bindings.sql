-- UI-MESSENGER-SPEC v1.3 §5 탭 3 — 세션별 채널 바인딩.
-- 사용자 W: "메신저 카드에서 터미널별 텔레그램 설정이나 디스코드 채널설정이 각각 가능해야해"

CREATE TABLE IF NOT EXISTS session_channel_bindings (
    id              TEXT PRIMARY KEY,        -- ULID
    agent_id        TEXT NOT NULL,           -- session/peer alias
    platform        TEXT NOT NULL,           -- 'discord' | 'telegram' | 'slack' | 'web'
    channel_ref     TEXT NOT NULL,           -- discord channel_id / telegram chat_id / slack channel
    bot_label       TEXT,                    -- 표시용 ("스타리안#3534")
    mention_trigger TEXT,                    -- "@researcher" or "@all"
    permission      TEXT NOT NULL DEFAULT 'reply', -- 'reply' | 'read_only' | 'command'
    active          INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_session_bindings_agent ON session_channel_bindings(agent_id);
CREATE INDEX IF NOT EXISTS idx_session_bindings_platform ON session_channel_bindings(platform);
