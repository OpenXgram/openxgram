-- UI-MESSENGER-SPEC v1.3 완전 구현용 테이블들.
--
-- M-2: 영구 Agent ID (ULID). PID·tmux 핸들 변경에도 동일.
-- L2: 3-레이어 정체성 (user_did + machine + agent ULID).
-- V6: cross-machine outbound queue (Tailscale P2P). 30일 보관 + ULID dedup.
-- N4: 글로벌 검색 FTS5 + sqlite-vec (현재 FTS5 만; vec 별도).
-- N7: 시스템 cron 보호 마크.

CREATE TABLE IF NOT EXISTS agent_identities (
    id              TEXT PRIMARY KEY,       -- ULID 영구 (M-2)
    display_name    TEXT NOT NULL,
    user_did        TEXT,                   -- L2: did:openxgram:...
    machine         TEXT NOT NULL,          -- L2: hostname/alias
    project_path    TEXT,
    role            TEXT,
    locale          TEXT DEFAULT 'ko-KR',
    status          TEXT NOT NULL DEFAULT 'Active',     -- Active | Idle | Dormant | Offline | Decommissioned
    llm_mode        TEXT NOT NULL DEFAULT 'Working',    -- Working | Waiting | SubAgent | TerminalOnly
    auto_respond_override TEXT,             -- NULL | 'true' | 'false'
    -- session handle (current, 갱신 빈번)
    handle_kind     TEXT,                   -- Tmux | Iterm | VSCodeIntegrated | WindowsTerminal | Screen
    handle_id       TEXT,                   -- "tmux:0"
    current_pid     INTEGER,
    cwd             TEXT,
    -- L4: HD index (sub_wallets 와 매핑)
    hd_derivation_index INTEGER,
    -- timestamps
    started_at      TEXT NOT NULL,
    last_attached_at TEXT,
    last_seen_at    TEXT,
    -- status_message
    status_message  TEXT,
    status_until    TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_identities_machine ON agent_identities(machine);
CREATE INDEX IF NOT EXISTS idx_agent_identities_status ON agent_identities(status);

-- V6: outbound queue (cross-machine, 30일 보관, ULID dedup)
CREATE TABLE IF NOT EXISTS outbound_queue (
    msg_ulid        TEXT PRIMARY KEY,
    target_machine  TEXT NOT NULL,
    target_alias    TEXT NOT NULL,
    body            TEXT NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_retry_at   TEXT,                   -- 지수 backoff (1s → ... 5min)
    last_error      TEXT,
    enqueued_at     TEXT NOT NULL,
    sent_at         TEXT
);

CREATE INDEX IF NOT EXISTS idx_outbound_queue_pending ON outbound_queue(sent_at, next_retry_at);

-- N4: 글로벌 검색 FTS5 (메시지 + 위키 + 패턴 + 실수 통합).
-- sqlite-vec 는 별 단계.
CREATE VIRTUAL TABLE IF NOT EXISTS global_search USING fts5(
    kind UNINDEXED,                         -- 'message' | 'wiki' | 'mistake' | 'pattern' | 'trait'
    ref_id UNINDEXED,
    title,
    body,
    tokenize = 'porter unicode61'
);

-- V11: RoutingRule (에이전트 ↔ 에이전트 internal scope). 인간 ↔ 에이전트는 채널 카드.
CREATE TABLE IF NOT EXISTS routing_rules (
    id              TEXT PRIMARY KEY,
    scope           TEXT NOT NULL DEFAULT 'Internal', -- Internal 만 (V11)
    from_pattern    TEXT NOT NULL,
    to_pattern      TEXT NOT NULL,
    action          TEXT NOT NULL,          -- e.g. "forward", "summarize_and_send", "block"
    created_at      TEXT NOT NULL,
    active          INTEGER NOT NULL DEFAULT 1
);

-- V12: 3-layer 버전 mismatch 기록 (release / GUI / daemon).
CREATE TABLE IF NOT EXISTS version_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    release_version TEXT,
    gui_version     TEXT,
    daemon_version  TEXT,
    machine         TEXT,
    recorded_at     TEXT NOT NULL
);

-- N7: 시스템 cron 보호 — 사용자 비활성화 시도 감사.
CREATE TABLE IF NOT EXISTS system_cron_protect_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    cron_name       TEXT NOT NULL,
    attempted_at    TEXT NOT NULL,
    result          TEXT NOT NULL DEFAULT 'rejected'
);

-- N8: Decommissioned audit (강제종료 vs 영구 비활성 구분).
CREATE TABLE IF NOT EXISTS agent_lifecycle_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id        TEXT NOT NULL,
    action          TEXT NOT NULL,          -- 'force_kill' | 'decommission' | 'restart' | 'sleep' | 'wake'
    reason          TEXT,
    at              TEXT NOT NULL
);
