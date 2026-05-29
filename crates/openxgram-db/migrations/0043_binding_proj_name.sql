-- rc.170 — auto-echo enforcer 의 alias mapping.
-- session_channel_bindings 의 agent_id (friendly: starianset) 와 messages.session_id 의 proj_name (set) 이 다를 때
-- session_proj_name 명시. NULL 이면 agent_id 그대로 (직접 매칭).
-- 매칭 SQL: pattern = COALESCE(session_proj_name, agent_id) → claude:{pattern}:%

ALTER TABLE session_channel_bindings ADD COLUMN session_proj_name TEXT;
