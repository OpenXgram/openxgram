-- 에이전트별 컴포저 설정 영속 — 모델/effort(thinking)/권한모드를 agent_profiles 에 저장하여
-- 새로고침·재부팅 후에도 각 에이전트가 마지막 설정을 유지한다.
--   perm_mode = bypassPermissions(기본) | acceptEdits | plan | default
--   model     = default | haiku | sonnet | opus | <openrouter id>
--   thinking  = high(기본) | medium | low | none
-- 기존 행은 NULL → 프론트에서 기본값(bypassPermissions/default/high) 적용.
ALTER TABLE agent_profiles ADD COLUMN perm_mode TEXT;
ALTER TABLE agent_profiles ADD COLUMN model TEXT;
ALTER TABLE agent_profiles ADD COLUMN thinking TEXT;
