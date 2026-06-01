-- rc.227 — application-level ACK (conversation_id 매칭).
-- transport ACK (ack_at/ack_status) 는 receiver inbox INSERT / tmux inject 성공 여부.
-- 그러나 LLM 실제 처리 (답신 작성) 여부는 별도 — app_ack_*.
--
-- 흐름:
--   1. sender peer_send 직후 outbound_queue INSERT 시 app_ack_check_after = NOW + 5min 설정
--   2. receiver 측 daemon.process_inbound 에서 envelope.conversation_id 가 있고
--      그 conversation_id 가 자기가 보낸 outbound_queue row 의 conversation_id 와 일치하면
--      → 그 row 의 app_ack_at = NOW, app_ack_status = 'processed' UPDATE
--   3. app_ack_timeout_drain worker (60s tick) 가 app_ack_check_after < NOW
--      AND app_ack_at IS NULL → app_ack_status = 'blocked' 로 마킹

ALTER TABLE outbound_queue ADD COLUMN conversation_id TEXT;      -- envelope.conversation_id (매칭 키)
ALTER TABLE outbound_queue ADD COLUMN app_ack_at TEXT;            -- ISO 8601, 답신 도착 시각
ALTER TABLE outbound_queue ADD COLUMN app_ack_status TEXT;        -- 'processed' | 'blocked' | 'nack'
ALTER TABLE outbound_queue ADD COLUMN app_ack_check_after TEXT;   -- ISO 8601, 송신 시각 + 5분

-- app_ack timeout 폴링 가속 (worker 60s tick 마다 NULL && check_after < NOW 조회).
CREATE INDEX IF NOT EXISTS idx_outbound_queue_app_ack_pending
    ON outbound_queue(app_ack_at, app_ack_check_after);

-- conversation_id 매칭 가속 (receiver inbound hook 매 envelope 마다 조회).
CREATE INDEX IF NOT EXISTS idx_outbound_queue_conversation
    ON outbound_queue(conversation_id, app_ack_at);
