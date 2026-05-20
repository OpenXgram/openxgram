-- UI-MESSENGER-SPEC v1.3 §2.4 + L4 — 서브 지갑 + HD 영구 점유 인덱스.
--
-- 결정:
--  M-3: 마스터(🔑 신원) + 세션별 서브(메신저). HD 파생.
--  L4: derivation_index 영구 점유 — Decommissioned 도 재사용 X. 같은 derived_address
--      재할당 시 과거 거래 트레이스 노출 위험.
--  M-6: 자동 충전 정책 (임계·1회·일 한도).
--  S6: daily_limit = LLM 토큰비 + x402 결제 합산.
--
-- 마스터 지갑은 🔑 신원 카드 책임 — 본 테이블은 서브 지갑만.

CREATE TABLE IF NOT EXISTS sub_wallets (
    agent_id            TEXT PRIMARY KEY,
    derivation_index    INTEGER NOT NULL UNIQUE,    -- L4: 영구 점유
    derived_address     TEXT NOT NULL UNIQUE,
    -- 잔액 (USDC micro units — 1 USDC = 1_000_000)
    allocated_micro     INTEGER NOT NULL DEFAULT 0,
    spent_micro         INTEGER NOT NULL DEFAULT 0,
    earned_micro        INTEGER NOT NULL DEFAULT 0,
    -- balance = allocated - spent + earned (계산용 — view 로 노출)
    -- 정책 (S6 합산)
    daily_limit_micro       INTEGER NOT NULL DEFAULT 2000000,    -- $2.00 default
    monthly_limit_micro     INTEGER NOT NULL DEFAULT 50000000,   -- $50.00 default
    auto_approve_below_micro INTEGER NOT NULL DEFAULT 1000000,   -- $1.00 자동 승인 한계
    -- M-6 자동 충전
    auto_topup_enabled       INTEGER NOT NULL DEFAULT 0,
    auto_topup_threshold_micro INTEGER NOT NULL DEFAULT 2000000,
    auto_topup_amount_micro    INTEGER NOT NULL DEFAULT 5000000,
    auto_topup_max_per_day_micro INTEGER NOT NULL DEFAULT 20000000,
    auto_topup_consumed_today_micro INTEGER NOT NULL DEFAULT 0,
    auto_topup_consumed_date TEXT,                  -- ISO date, reset 키
    -- 상태 (L4 + N8: Active | Decommissioned 영구 점유)
    status              TEXT NOT NULL DEFAULT 'Active', -- 'Active' | 'Decommissioned'
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sub_wallets_status ON sub_wallets(status);

-- HD 인덱스 영구 점유 (L4): Decommissioned 후에도 별도 row 로 남겨 재사용 방지.
-- 같은 derivation_index 가 두 번 등장하지 않음을 보장.
CREATE TABLE IF NOT EXISTS hd_index_history (
    derivation_index INTEGER PRIMARY KEY,
    agent_id         TEXT NOT NULL,
    derived_address  TEXT NOT NULL,
    occupied_at      TEXT NOT NULL,
    released_at      TEXT  -- NULL 이면 현재 사용 중; non-null 이면 Decommissioned (재사용 X)
);

-- 마스터 지갑 메타 (단일 row). 마스터는 🔑 신원 카드 정본이지만 메신저 view 용 캐시.
CREATE TABLE IF NOT EXISTS master_wallet_view (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    master_address  TEXT,
    free_micro      INTEGER NOT NULL DEFAULT 0,   -- 미할당 free 잔액
    last_synced_at  TEXT
);

INSERT OR IGNORE INTO master_wallet_view (id, free_micro, last_synced_at)
VALUES (1, 0, '');
