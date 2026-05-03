-- Payment intent 인프라 (PRD §16) — 데이터 구조 + 서명 검증만. 실 on-chain
-- 제출은 후속 PR (alloy/ethers RPC 통합).
--
-- USDC 는 6 decimals — amount_usdc_micro 는 micro USDC (10^-6 USDC).
-- 즉 1 USDC = 1_000_000.

CREATE TABLE payment_intents (
    id TEXT PRIMARY KEY,
    amount_usdc_micro INTEGER NOT NULL,   -- 음수 거부 (CHECK)
    chain TEXT NOT NULL,                  -- "base" | "polygon" | "ethereum" | ...
    payee_address TEXT NOT NULL,          -- 0x... 수취인 ETH 주소
    memo TEXT,
    nonce TEXT NOT NULL UNIQUE,           -- replay 방지
    signature_hex TEXT,                   -- master ECDSA 서명 (서명 후 채워짐)
    state TEXT NOT NULL DEFAULT 'draft',  -- draft / signed / submitted / confirmed / failed
    created_at TEXT NOT NULL,
    signed_at TEXT,
    submitted_tx_hash TEXT,                -- 0x... 트랜잭션 해시 (제출 후)
    submitted_at TEXT,
    confirmed_at TEXT,
    error_reason TEXT,
    CHECK (amount_usdc_micro > 0),
    CHECK (state IN ('draft', 'signed', 'submitted', 'confirmed', 'failed'))
);

CREATE INDEX idx_payment_intents_state ON payment_intents(state);
CREATE INDEX idx_payment_intents_nonce ON payment_intents(nonce);
CREATE INDEX idx_payment_intents_payee ON payment_intents(payee_address);
