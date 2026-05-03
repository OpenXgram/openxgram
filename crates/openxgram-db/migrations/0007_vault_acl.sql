-- Vault ACL + audit — PRD §8 정책 (마스터 승인 auto/confirm/mfa, 일일 한도, 감사 로그).
-- ACL 비어있는 key 는 master only (모든 에이전트 거부, master 만 직접 호출 가능).

CREATE TABLE vault_acl (
    id TEXT PRIMARY KEY,
    -- '*' 는 와일드카드 (key 와 무관하게 매칭). 그렇지 않으면 정확 일치 또는 prefix/* 패턴.
    key_pattern TEXT NOT NULL,
    -- 호출자 식별 (예: 0xMyAddr). '*' 는 모든 에이전트.
    agent TEXT NOT NULL,
    -- 콤마 구분: get, set, delete (list 는 항상 허용 — 메타만)
    allowed_actions TEXT NOT NULL DEFAULT 'get',
    -- 일일 한도 (호출 횟수). 0 = 무제한.
    daily_limit INTEGER NOT NULL DEFAULT 0,
    -- auto / confirm / mfa — Phase 1 은 auto 만 즉시 적용 (정책 후속).
    policy TEXT NOT NULL DEFAULT 'auto',
    created_at TEXT NOT NULL,
    UNIQUE(key_pattern, agent)
);

CREATE INDEX idx_vault_acl_pattern ON vault_acl(key_pattern);
CREATE INDEX idx_vault_acl_agent ON vault_acl(agent);

CREATE TABLE vault_audit (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL,
    agent TEXT NOT NULL,
    action TEXT NOT NULL,            -- get/set/delete/list
    allowed INTEGER NOT NULL,        -- 0/1
    reason TEXT,                     -- 거부 사유 (allowed=0 시)
    timestamp TEXT NOT NULL
);

CREATE INDEX idx_vault_audit_key ON vault_audit(key);
CREATE INDEX idx_vault_audit_agent ON vault_audit(agent);
CREATE INDEX idx_vault_audit_timestamp ON vault_audit(timestamp);
