-- Phase 2-D — 에이전트 프로필 (카카오톡 셸 GUI 재구축).
-- 8단계 추가 폼의 신규 차원만 보관. folder/group/role 은 기존 agent_capabilities
-- (project_path / group_name / role) 재사용 — 중복 보관 안 함.
--
--   classification  = 명부 그룹화 (primary / project / special)
--   execution_mode  = 생명주기 (always 상시 / on_demand 선택 / heartbeat 깨움)
--   ai_type         = 동적 설정 탐지 분기 (claude / codex / gemini)
--   worktree        = git worktree 경로 (선택, 격리 작업용)
--   is_public       = 마켓 공개 여부 (Phase 6 마켓 노출)
--
-- 기존 데이터 보존: 신규 테이블이므로 agent_capabilities/peer 레지스트리 무손상.

CREATE TABLE IF NOT EXISTS agent_profiles (
    alias           TEXT PRIMARY KEY,        -- agent_capabilities.alias 와 1:1 (FK 강제는 안 함, soft link)
    ai_type         TEXT NOT NULL DEFAULT 'claude',   -- claude | codex | gemini
    classification  TEXT NOT NULL DEFAULT 'project',  -- primary | project | special
    execution_mode  TEXT NOT NULL DEFAULT 'on_demand',-- always | on_demand | heartbeat
    worktree        TEXT,                    -- git worktree 경로 (있으면)
    is_public       INTEGER NOT NULL DEFAULT 0,        -- 0/1 — 마켓 공개
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_profiles_class ON agent_profiles(classification);
CREATE INDEX IF NOT EXISTS idx_agent_profiles_exec ON agent_profiles(execution_mode);
CREATE INDEX IF NOT EXISTS idx_agent_profiles_public ON agent_profiles(is_public);
