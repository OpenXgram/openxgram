-- 마켓 (c)갈래 — 지갑 거래 원장 (ledger).
--
-- 결정:
--  - purchase_service 가 실제 결제될 때 sub_wallets.spent_micro 를 차감하고
--    여기에 1 row 기록 (가짜 영수증 금지 — 실제 잔액 검증 + 차감의 감사 추적).
--  - topup(충전)·purchase(구매)·earn(수익) 모두 동일 원장에 기록.
--  - amount_micro 부호: 잔액 증가(+: topup/earn), 잔액 감소(-: purchase).
--  - intent_id: PaymentReceipt.intent_id (내부 결제 의도 id, on-chain 미사용 시도 내부 ledger txn).
--  - tx_ref: on-chain tx hash (온체인 결제 시) 또는 내부 ledger ref (예: "ledger:<uuid>").
--
-- 온체인 USDC 결제는 funded wallet + RPC 가 필요 — 현 단계는 내부 ledger 1차 구현.
-- (openxgram_payment::submit_intent 로 온체인 전환 시 tx_ref 에 실제 tx_hash 채움.)

CREATE TABLE IF NOT EXISTS wallet_ledger (
    id              TEXT PRIMARY KEY,            -- uuid
    agent_id        TEXT NOT NULL,               -- sub_wallets.agent_id (또는 'master')
    kind            TEXT NOT NULL,               -- 'topup' | 'purchase' | 'earn'
    amount_micro    INTEGER NOT NULL,            -- 부호 있음 (+ 충전/수익, - 구매)
    chain           TEXT,                        -- 결제 체인 (예: 'base'), topup 은 NULL 가능
    counterparty    TEXT,                        -- 상대(payee / 외부 사용자), 옵션
    intent_id       TEXT,                        -- PaymentReceipt.intent_id
    tx_ref          TEXT,                        -- on-chain tx hash 또는 internal ledger ref
    memo            TEXT,                        -- job_id / agent=.. svc=.. 등
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wallet_ledger_agent ON wallet_ledger(agent_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_wallet_ledger_kind ON wallet_ledger(kind);
