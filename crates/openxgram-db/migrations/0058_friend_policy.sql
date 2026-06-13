-- rc.321 — 친구(friend) 단위 POLICY 레이어 (권한/격리/비용).
-- rc.320 agent-level 친구(classification="friend") 위에 per-friend 설정을 얹는다.
-- agent_profiles 에 정책 컬럼 추가 + friend_cost_ledger 사용량 원장.
--
--   friend_permission   = blocked | read | request | full (기본 request)
--                         · blocked : 친구의 모든 A2A 요청 거절
--                         · read    : 읽기/상태성 요청만 허용, 작업 실행 거절
--                         · request : 작업 실행 허용 (기본)
--                         · full    : 작업 실행 허용 (추후 권한 작업까지)
--   friend_isolated     = 1 이면 친구의 작업을 격리 cwd 에서 실행 (메인 워크트리 보호)
--   friend_cost_tracked = 1 이면 처리 후 friend_cost_ledger 에 사용량 기록 (기본 1)
--
-- 기존 데이터 보존: ALTER ADD COLUMN — 기존 agent_profiles row 무손상.
-- (이미 수동 ALTER 된 DB 는 migrate.rs 가 'duplicate column' graceful skip.)

ALTER TABLE agent_profiles ADD COLUMN friend_permission TEXT NOT NULL DEFAULT 'request';
ALTER TABLE agent_profiles ADD COLUMN friend_isolated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_profiles ADD COLUMN friend_cost_tracked INTEGER NOT NULL DEFAULT 1;

-- 친구별 사용량 원장 — A2A 요청 처리마다 1 row.
CREATE TABLE IF NOT EXISTS friend_cost_ledger (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    friend_alias    TEXT NOT NULL,           -- 요청을 보낸 친구(=TaskBody.from)
    machine         TEXT,                    -- 친구의 머신 라벨 (agent_profiles.machine)
    occurred_at_kst TEXT NOT NULL,           -- KST 타임스탬프 (Asia/Seoul)
    kind            TEXT,                    -- 요청 종류 (skill id / "task" / "read" 등)
    tokens          INTEGER NOT NULL DEFAULT 0, -- best-available 토큰 수 (없으면 0)
    note            TEXT
);
CREATE INDEX IF NOT EXISTS idx_friend_cost_alias ON friend_cost_ledger(friend_alias);
CREATE INDEX IF NOT EXISTS idx_friend_cost_at ON friend_cost_ledger(occurred_at_kst);
