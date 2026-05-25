-- rc.125 — 에이전트 메신저 등록 확장.
-- 자동 감지 + 수정 가능 + 자유 orchestration_role + special_instructions.

ALTER TABLE agent_capabilities ADD COLUMN orchestration_role TEXT;
ALTER TABLE agent_capabilities ADD COLUMN special_instructions TEXT;

CREATE INDEX IF NOT EXISTS idx_agent_caps_orch_role ON agent_capabilities(orchestration_role);
