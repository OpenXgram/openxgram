-- 남은 사양 전체: Identity·Vault MCP·Channel 모더레이션·Autonomy SelfTrigger·Memory merge.

-- UI-IDENTITY-SPEC v1.0 M-9 머신 sub-DID, M-15 키 교체 이력.
CREATE TABLE IF NOT EXISTS sub_dids (
    id              TEXT PRIMARY KEY,        -- did:openxgram:0x...
    machine         TEXT NOT NULL,
    parent_did      TEXT,
    derived_address TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'Active', -- Active | Revoked
    created_at      TEXT NOT NULL,
    revoked_at      TEXT,
    revoke_reason   TEXT
);
CREATE INDEX IF NOT EXISTS idx_sub_dids_machine ON sub_dids(machine);

-- M-8 5회 실패 lockout
CREATE TABLE IF NOT EXISTS auth_failures (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    attempted_at    TEXT NOT NULL,
    backoff_until   TEXT
);

-- UI-VAULT-MCP-SPEC §3.2 MCP 서버 등록
CREATE TABLE IF NOT EXISTS mcp_servers (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    transport       TEXT NOT NULL DEFAULT 'stdio',  -- stdio | http
    command         TEXT,                            -- stdio 시
    url             TEXT,                            -- http 시
    scope           TEXT NOT NULL DEFAULT 'user',    -- user | project
    health_status   TEXT DEFAULT 'unknown',          -- ok | error | unknown
    last_check_at   TEXT,
    created_at      TEXT NOT NULL,
    active          INTEGER NOT NULL DEFAULT 1
);

-- UI-VAULT-MCP-SPEC §3.3 도구 카탈로그 + ACL
CREATE TABLE IF NOT EXISTS tool_acl (
    tool_name       TEXT PRIMARY KEY,                -- filesystem | shell | net | payment | llm-call
    default_policy  TEXT NOT NULL DEFAULT 'confirm', -- auto | confirm | mfa | block
    description     TEXT,
    updated_at      TEXT NOT NULL
);

-- UI-CHANNEL-SPEC §3.5 모더레이션 (차단·일 한도)
CREATE TABLE IF NOT EXISTS channel_blocks (
    person_id       TEXT PRIMARY KEY,
    reason          TEXT,
    blocked_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS channel_person_limits (
    person_id       TEXT PRIMARY KEY,
    daily_limit     INTEGER NOT NULL DEFAULT 100,
    today_used      INTEGER NOT NULL DEFAULT 0,
    reset_date      TEXT
);

-- UI-AUTONOMY-SPEC SelfTrigger (이벤트 → 작업 규칙)
CREATE TABLE IF NOT EXISTS self_trigger_rules (
    id              TEXT PRIMARY KEY,
    event_pattern   TEXT NOT NULL,                    -- "discord:new_message" / "tmux:idle_15min"
    target_agent    TEXT NOT NULL,                    -- ULID 또는 alias
    action          TEXT NOT NULL,                    -- "wake_and_call_recv_messages" 등
    active          INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    last_fired_at   TEXT,
    fire_count      INTEGER NOT NULL DEFAULT 0
);

-- UI-AUTONOMY-SPEC Reflection 실행 이력
CREATE TABLE IF NOT EXISTS reflection_runs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    success         INTEGER,
    summary         TEXT,
    new_pages       INTEGER NOT NULL DEFAULT 0,
    patterns_found  INTEGER NOT NULL DEFAULT 0
);

-- UI-MEMORY-SPEC M-2 자동 통합 (merge) 큐
CREATE TABLE IF NOT EXISTS wiki_merge_candidates (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_a_id       TEXT NOT NULL,
    page_b_id       TEXT NOT NULL,
    similarity      REAL,
    detected_at     TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',  -- pending | merged | dismissed
    merged_into     TEXT
);

-- M-10 편집 충돌 락 (사용자 편집 중 = AI 갱신 차단)
CREATE TABLE IF NOT EXISTS wiki_edit_locks (
    page_id         TEXT PRIMARY KEY,
    holder          TEXT NOT NULL,                    -- 'user' | 'ai'
    acquired_at     TEXT NOT NULL,
    expires_at      TEXT NOT NULL                     -- 자동 만료 (예: 5분)
);

-- Peer 등록용 keypair 메타 (실 private key 는 keystore. 본 테이블은 metadata)
CREATE TABLE IF NOT EXISTS peer_keypairs (
    alias           TEXT PRIMARY KEY,
    public_key_hex  TEXT NOT NULL,
    address         TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    note            TEXT
);
