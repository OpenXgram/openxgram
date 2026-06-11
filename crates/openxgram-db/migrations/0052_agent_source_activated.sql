-- 기본 동봉(built-in) 특수에이전트의 설치/활성화 상태.
-- xgram-ops 같은 OpenXgram 운영 에이전트는 설치 시 동봉되되, 마스터가 GUI에서 활성화해야 동작.
--
--   source     = 'user' (마스터가 직접 생성) | 'built_in' (설치 시 동봉)
--   activated  = 0/1 — built_in 은 0(설치됨·미활성)으로 seed, 활성화 버튼으로 1.
--
-- 기존 데이터 보존: 기존 모든 행은 DEFAULT 로 source='user', activated=1 → 동작 무변경.
ALTER TABLE agent_profiles ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
ALTER TABLE agent_profiles ADD COLUMN activated INTEGER NOT NULL DEFAULT 1;
CREATE INDEX IF NOT EXISTS idx_agent_profiles_source ON agent_profiles(source);
