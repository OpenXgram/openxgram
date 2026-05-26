-- rc.133+ — L2 memories 의미검색을 위한 임베딩 테이블.
-- message_embeddings / message_embedding_map 과 동일한 패턴.
-- memory_embeddings: sqlite-vec virtual table (384d float32).
-- memory_embedding_map: memory_id ↔ embedding rowid 매핑.

CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings
    USING vec0(embedding float[384]);

CREATE TABLE IF NOT EXISTS memory_embedding_map (
    memory_id       TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    embedding_rowid INTEGER NOT NULL,
    PRIMARY KEY (memory_id)
);

CREATE INDEX IF NOT EXISTS idx_memory_embedding_map_rowid
    ON memory_embedding_map(embedding_rowid);
