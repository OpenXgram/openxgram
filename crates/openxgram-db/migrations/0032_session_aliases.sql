-- v32: 사용자가 부여한 display_name (tmux/claude_project/peer 등 모든 식별자에 적용)
CREATE TABLE IF NOT EXISTS session_aliases (
    identifier TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    note TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
