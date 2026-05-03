-- Hash chain + Merkle checkpoint (PRD-AUDIT-01, 02).
-- vault_audit 에 prev_hash + entry_hash + seq 추가.
-- audit_checkpoint 신규 — 1시간마다 Merkle root + 서명.

ALTER TABLE vault_audit ADD COLUMN prev_hash BLOB;
ALTER TABLE vault_audit ADD COLUMN entry_hash BLOB;
ALTER TABLE vault_audit ADD COLUMN seq INTEGER;

CREATE INDEX IF NOT EXISTS idx_vault_audit_seq ON vault_audit(seq);

CREATE TABLE audit_checkpoint (
    seq INTEGER PRIMARY KEY,
    merkle_root BLOB NOT NULL,
    signature BLOB NOT NULL,
    signer_pubkey_hex TEXT NOT NULL,
    signed_at_kst INTEGER NOT NULL
);
