-- 0021_marketplace_cache.sql — OpenAgentX 마켓 응답 캐싱 + 구매 이력
-- 정본: docs/PRD-OpenXgram.md §4.4 (openxgram-marketplace crate)
--
-- "사용자 LLM이 마켓 에이전트를 자기 도구처럼 부를 수 있게 — 검색·상세 응답 캐시 +
--  구매 이력 (감사·history 도구 후속용)."
--
-- 절대 규칙 1 (fallback 금지): 모든 INSERT/UPDATE는 명시적 검증.
-- 절대 규칙 3 (DB 변경 마스터 승인): 신규 CREATE TABLE만, 기존 데이터 무영향.

-- 에이전트 메타데이터 캐시 (GET /api/agents/[id] 결과).
CREATE TABLE IF NOT EXISTS marketplace_agents (
    agent_id        TEXT PRIMARY KEY,           -- 마켓 발급 id (예: agent:<...>)
    name            TEXT NOT NULL,
    description     TEXT NOT NULL,
    maker_id        TEXT,
    category        TEXT,
    rating          REAL,
    rating_count    INTEGER,
    services_json   TEXT NOT NULL DEFAULT '[]', -- Service[] JSON
    cached_at       INTEGER NOT NULL,           -- unix epoch ms
    expires_at      INTEGER NOT NULL            -- TTL (예: 1시간)
);

CREATE INDEX IF NOT EXISTS idx_marketplace_agents_expires
    ON marketplace_agents(expires_at);

-- 검색 결과 캐시 (GET /api/agents?q=...).
CREATE TABLE IF NOT EXISTS marketplace_search_cache (
    cache_key       TEXT PRIMARY KEY,           -- "q=<...>|limit=<n>" 정규화
    query           TEXT NOT NULL,
    agents_json     TEXT NOT NULL,              -- Agent[] JSON
    cached_at       INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_marketplace_search_expires
    ON marketplace_search_cache(expires_at);

-- 구매 이력 (purchase_service 호출마다 1 row — 자동 승인·사용자 승인 모두).
CREATE TABLE IF NOT EXISTS marketplace_purchases (
    id                  TEXT PRIMARY KEY,         -- 내부 uuid
    job_id              TEXT,                     -- 마켓이 발급한 job id (NeedsConfirmation 시 NULL)
    agent_id            TEXT NOT NULL,
    service_id          TEXT NOT NULL,
    amount_usdc_micro   INTEGER NOT NULL,
    decision            TEXT NOT NULL             -- 'auto_approved' | 'needs_confirmation'
        CHECK (decision IN ('auto_approved', 'needs_confirmation')),
    decision_reason     TEXT,                     -- needs_confirmation 사유
    payment_tx_hash     TEXT,                     -- AutoApproved 시 on-chain tx
    payment_intent_id   TEXT,                     -- openxgram-payment 연계 id
    requested_at        INTEGER NOT NULL,
    settled_at          INTEGER                   -- 결제 confirm 시각
);

CREATE INDEX IF NOT EXISTS idx_marketplace_purchases_agent
    ON marketplace_purchases(agent_id, requested_at DESC);
CREATE INDEX IF NOT EXISTS idx_marketplace_purchases_decision
    ON marketplace_purchases(decision, requested_at DESC);
CREATE INDEX IF NOT EXISTS idx_marketplace_purchases_job
    ON marketplace_purchases(job_id) WHERE job_id IS NOT NULL;

-- 자동 결제 화이트리스트 (SpendPolicy.whitelist를 DB로 영속).
CREATE TABLE IF NOT EXISTS marketplace_whitelist (
    agent_id    TEXT PRIMARY KEY,
    note        TEXT,
    added_at    INTEGER NOT NULL
);
