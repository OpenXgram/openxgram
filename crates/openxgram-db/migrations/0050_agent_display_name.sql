-- 에이전트 대화명(표시 이름) — 로스터·대화 헤더에 alias 대신 노출(없으면 alias fallback).
-- 상세 패널에서 수정 가능(POST /v1/gui/agent/{alias}/profile).
ALTER TABLE agent_profiles ADD COLUMN display_name TEXT;
