-- conversation_id: 한 inbound + 그에 따른 응답·서브 호출·outbox 회신을 묶는 ID.
-- 단순 UUID 문자열 (lowercase hex, 32자). 새 메시지는 앱이 채우고, 기존 row 는 row 단위로 fresh ID 부여.
-- SQLite ALTER ADD COLUMN 은 함수형 DEFAULT 미지원이라 NULL 로 추가 후 backfill.

ALTER TABLE messages ADD COLUMN conversation_id TEXT;

UPDATE messages
   SET conversation_id = lower(hex(randomblob(16)))
 WHERE conversation_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_messages_conversation
    ON messages(conversation_id, timestamp);
