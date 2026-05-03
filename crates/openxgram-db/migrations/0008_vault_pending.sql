-- Vault pending confirmations — policy=confirm 호출 시 마스터 응답 대기 큐.
-- 마스터가 xgram vault approve/deny <id> 또는 디스코드 응답으로 status 갱신.

CREATE TABLE vault_pending_confirmations (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL,
    agent TEXT NOT NULL,
    action TEXT NOT NULL,        -- get / set / delete
    -- pending → approved / denied / expired
    status TEXT NOT NULL DEFAULT 'pending',
    requested_at TEXT NOT NULL,
    decided_at TEXT,
    -- TOTP code (mfa policy 일 때) — base32 secret 검증 결과
    mfa_validated INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_vault_pending_status ON vault_pending_confirmations(status);
CREATE INDEX idx_vault_pending_key_agent ON vault_pending_confirmations(key, agent);

-- TOTP mfa secret 저장 — base32 인코딩, agent 별 또는 글로벌 (agent='*').
CREATE TABLE vault_mfa_secrets (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL UNIQUE,
    secret_base32 TEXT NOT NULL,
    created_at TEXT NOT NULL
);
