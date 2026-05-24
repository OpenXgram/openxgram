-- rc.92 — 멀티 디스코드 봇. 채널·세션 별로 다른 봇 사용 (다른 메이커 봇 공존).
-- W: "채널별로 다른 디스코드봇과 연결이 될 수 있어야 — 다른 사람의 에이전트를 사용하는 경우"

CREATE TABLE IF NOT EXISTS discord_bots (
    id          TEXT PRIMARY KEY,         -- ULID
    alias       TEXT NOT NULL UNIQUE,     -- 표시명 (예: "내 봇", "친구 봇")
    bot_token   TEXT NOT NULL,            -- Discord bot token
    bot_user_id TEXT,                     -- Discord 의 application_id (검증 시 채움)
    owner       TEXT,                     -- "self" | "shared:<peer_alias>"
    active      INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_discord_bots_active ON discord_bots(active);

-- session_channel_bindings 에 bot_id 추가 (NULL 이면 default 봇 = notify.toml)
ALTER TABLE session_channel_bindings ADD COLUMN bot_id TEXT REFERENCES discord_bots(id);
CREATE INDEX IF NOT EXISTS idx_session_bindings_bot ON session_channel_bindings(bot_id);
