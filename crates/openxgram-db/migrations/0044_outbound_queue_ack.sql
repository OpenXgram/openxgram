-- rc.219 — outbound_queue ACK 추적.
-- sender 측 envelope 송신 성공 (sent_at IS NOT NULL) 후 receiver 의 ACK envelope 가
-- 도착하면 ack_at + ack_status UPDATE. ACK 미수신 시 재발송 worker 가 backoff 정책
-- 으로 재전송 (30s, 5min, 30min). 3회 후 fail mark.

ALTER TABLE outbound_queue ADD COLUMN ack_at TEXT;       -- ISO 8601 timestamp, NULL = 미 ACK
ALTER TABLE outbound_queue ADD COLUMN ack_status TEXT;    -- inbox_stored / tmux_injected / both / fail

-- ack 추적 lookup 가속 (재발송 worker 가 sent_at IS NOT NULL AND ack_at IS NULL 항목 빈번 조회).
CREATE INDEX IF NOT EXISTS idx_outbound_queue_ack_pending ON outbound_queue(sent_at, ack_at);
