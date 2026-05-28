-- rc.167 — peer 의 GUI server URL 별도 컬럼.
-- transport (`/v1/message`) 와 GUI (`/v1/gui/*`) 가 다른 port 일 때 필요.
-- 예: zalman daemon = 7300 (transport) + 7302 (GUI).
-- NULL = 같은 address 사용 (fallback).

ALTER TABLE peers ADD COLUMN gui_address TEXT;
