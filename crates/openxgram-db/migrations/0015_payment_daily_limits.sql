-- Payment daily limit (PRD §16) — agent 별 / chain 별 microUSDC 한도 정식 분리.
--
-- 배경: PR #93 의 Tauri payment_get/set_daily_limit 핸들러가 vault_acl row
--       (key_pattern='payment.usdc.transfer', agent='default') 의 daily_limit 컬럼을
--       의미적으로 재사용했음. ACL 권한과 결제 한도는 다른 관심사 — 정식 필드로 분리.
--
-- 한도 단위: microUSDC (1 USDC = 1_000_000 micro). 0 = 한도 미설정 (결제 차단).
--
-- updated_at_kst: KST(Asia/Seoul) RFC3339 문자열. KST 운영 정책 (CLAUDE.md).

CREATE TABLE payment_daily_limits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,                 -- 'default' / agent 식별자
    chain_id TEXT NOT NULL,                 -- 'base' | 'polygon' | 'ethereum' | ...
    daily_micro INTEGER NOT NULL,           -- microUSDC 한도 (>= 0)
    updated_at_kst TEXT NOT NULL,           -- RFC3339 KST 타임스탬프
    UNIQUE(agent_id, chain_id),
    CHECK (daily_micro >= 0)
);

CREATE INDEX idx_payment_daily_limits_agent ON payment_daily_limits(agent_id);
CREATE INDEX idx_payment_daily_limits_chain ON payment_daily_limits(chain_id);
