-- 마켓 (d)갈래 — free-tier 요금제 게이팅.
--
-- 개념:
--   - 구매(purchase_service) 실행 전 무료 할당량(quota) 확인. 무료 잔여가 있으면
--     과금 없이 통과(사용량 +1), 소진이면 (c)갈래 원장 결제로. 둘 다 불가면 명시 에러.
--   - 할당량 단위: 에이전트별 무료 N회/일 (free_calls_per_day). 0 이면 무료 없음(항상 유료).
--   - 전역 기본(agent_id='*') + 에이전트별 override. 가장 구체적인 설정 우선.
--
-- free_tier_config: 무료 할당량 설정 (전역 기본 + 에이전트별).
--   - agent_id='*' : 전역 기본 1 row (migration 에서 시드).
--   - agent_id='agent:xxx' : 특정 에이전트 override (있으면 우선).
CREATE TABLE IF NOT EXISTS free_tier_config (
    agent_id            TEXT PRIMARY KEY,           -- '*' (전역 기본) 또는 'agent:xxx'
    free_calls_per_day  INTEGER NOT NULL DEFAULT 0, -- 1일 무료 호출 횟수 (0=무료 없음)
    updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

-- 전역 기본 — 무료 없음(0). 운영자가 UI/route 로 조절.
INSERT OR IGNORE INTO free_tier_config (agent_id, free_calls_per_day) VALUES ('*', 0);

-- free_tier_usage: per-agent per-UTC-day 무료 사용량 카운터.
--   - day = UTC 날짜 (YYYY-MM-DD). 윈도우는 day 경계 기준 (날짜 바뀌면 자동 리셋).
--   - 구매가 무료로 통과될 때마다 used += 1 (UPSERT).
CREATE TABLE IF NOT EXISTS free_tier_usage (
    agent_id    TEXT NOT NULL,
    day         TEXT NOT NULL,                       -- UTC 'YYYY-MM-DD'
    used        INTEGER NOT NULL DEFAULT 0,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (agent_id, day)
);

CREATE INDEX IF NOT EXISTS idx_free_tier_usage_day ON free_tier_usage(day);
