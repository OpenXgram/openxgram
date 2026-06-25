-- Phase B 통합 현황 그리드 — 인지도(awareness/views).
-- 인지도 = views(조회수). uses=사용횟수(보조 지표). alias 1:1.
-- Phase B 는 read-only(gui_roster LEFT-JOIN). SET 엔드포인트는 자동/메트릭 수집(차후).

CREATE TABLE IF NOT EXISTS agent_metrics (
    alias       TEXT PRIMARY KEY,
    views       INTEGER NOT NULL DEFAULT 0,
    uses        INTEGER NOT NULL DEFAULT 0,
    updated_at  TEXT
);
