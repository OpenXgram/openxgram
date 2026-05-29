-- rc.170 — auto-echo enforcer 의 중복 echo 방지.
-- last_echoed_ulid: 이 binding 에서 마지막으로 Discord 로 echo 한 assistant message ULID.
-- 새 worker 가 jsonl tail 의 마지막 assistant message 와 비교 → 큰 것만 echo.

ALTER TABLE session_channel_bindings ADD COLUMN last_echoed_ulid TEXT;
