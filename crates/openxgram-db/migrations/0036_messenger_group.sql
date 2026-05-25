-- rc.122 D1 — 에이전트 메신저 등록 명시 + 그룹.
-- 마스터 요구: 외부 채널 바인딩과 별개의 메신저 등록 흐름.
--   messenger_enabled = TRUE → 다른 peer 의 list_peers 에 노출
--   group_name        = 협업 단위 (peer_send fan-out 대상)
--
-- 기존 agent_capabilities (rc.92 migration 0035) 확장.

ALTER TABLE agent_capabilities ADD COLUMN group_name TEXT;
ALTER TABLE agent_capabilities ADD COLUMN messenger_enabled INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_agent_caps_group ON agent_capabilities(group_name);
CREATE INDEX IF NOT EXISTS idx_agent_caps_messenger ON agent_capabilities(messenger_enabled);
