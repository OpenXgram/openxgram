-- rc.150 — message ack tracking. peer_send → ack chain (delivered/read/processing/done/failed).
-- portal × OpenXgram backend 통합 + multi-transport fallback 의 기반.

ALTER TABLE messages ADD COLUMN ack_status TEXT NOT NULL DEFAULT 'sent';
ALTER TABLE messages ADD COLUMN acked_at TEXT;
ALTER TABLE messages ADD COLUMN ack_via TEXT;  -- 'p2p' | 'discord' | 'telegram' | 'queue'
ALTER TABLE messages ADD COLUMN ack_note TEXT; -- 처리 결과/실패 사유

CREATE INDEX idx_messages_ack_status ON messages(ack_status, timestamp);
