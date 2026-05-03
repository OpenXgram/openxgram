-- L0 messages 임베딩 (sqlite-vec 가상 테이블 + 매핑)
-- 차원 384 — multilingual-e5-small 표준. fastembed 통합 시 동일 차원 사용.

CREATE VIRTUAL TABLE message_embeddings USING vec0(
    embedding float[384]
);

CREATE TABLE message_embedding_map (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    embedding_rowid INTEGER NOT NULL UNIQUE
);

CREATE INDEX idx_message_embedding_rowid ON message_embedding_map(embedding_rowid);
