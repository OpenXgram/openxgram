-- HD derivation index 관리 + KEK 회전 기록 (PRD-ROT-01).
-- m/44'/0'/0'/0/N 의 N 을 rotation 카운터로. retired_at 7일 유예 후 zeroize.

CREATE TABLE vault_kek_rotations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    derivation_index INTEGER NOT NULL UNIQUE,
    rotated_at_kst INTEGER NOT NULL,
    retired_at_kst INTEGER,
    -- audit row id 와 1:1 매핑 (PRD-ROT-03)
    audit_row_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_vault_kek_rotations_idx ON vault_kek_rotations(derivation_index);
