-- Vault — PRD §8 자격증명 인프라
-- encrypted_value 는 keystore::encrypt_blob 결과 (ChaCha20-Poly1305 + Argon2id).
-- tags JSON array, metadata JSON object.

CREATE TABLE vault_entries (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    encrypted_value BLOB NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    last_accessed TEXT NOT NULL
);

CREATE INDEX idx_vault_key ON vault_entries(key);
CREATE INDEX idx_vault_last_accessed ON vault_entries(last_accessed);
