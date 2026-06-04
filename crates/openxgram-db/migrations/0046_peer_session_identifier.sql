-- rc.245 — peer ↔ tmux 세션 결정적 매핑.
-- 기존: Messenger.tsx normalizeAlias 가 alias 문자열 정규화로 추정 (fragile).
--   peer alias "akashic" ↔ tmux session "aoe_akashic_5054a80a" 를 prefix/suffix strip 으로 매칭.
--   naming 불일치 시 "unsupported identifier" 표시.
-- 신규: 등록 시 명시적 세션 식별자를 peer row 에 저장 → capture_session 이 바로 resolve.
--   format: collect_sessions(/v1/gui/sessions) 가 내는 prefix 형식과 동일
--   (예: "tmux:<name>", "aoe:<...>", "portal:<...>", "claude:<...>").
--   UI 에서 사용자가 수동 override 도 가능 (PATCH /v1/gui/peers/{alias}/session).

ALTER TABLE peers ADD COLUMN session_identifier TEXT;   -- 명시적 tmux 세션 식별자 (NULL = 자동 추정 fallback)
