//! attestation row 영속 — onchain 제출 전/후 모두 추적.
//! 4.1.2.1 / 4.1.2.2 — 메시지·결제 발생 시 자동 INSERT (호출자 책임).

use openxgram_db::Db;
use rusqlite::params;

use crate::attest::{Attestation, AttestationKind};
use crate::Result;

/// `attestations` 테이블이 없으면 생성 (idempotent — migration 미관리, 본 crate 가 ensure).
const ENSURE_SQL: &str = "CREATE TABLE IF NOT EXISTS eas_attestations (
    id TEXT PRIMARY KEY,
    schema_uid TEXT NOT NULL,
    kind TEXT NOT NULL,
    fields_json TEXT NOT NULL,
    data_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    tx_hash TEXT
);
CREATE INDEX IF NOT EXISTS idx_eas_kind ON eas_attestations(kind, created_at);
CREATE INDEX IF NOT EXISTS idx_eas_data_hash ON eas_attestations(data_hash);
";

pub struct AttestationStore<'a> {
    db: &'a mut Db,
}

impl<'a> AttestationStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    pub fn ensure_schema(&mut self) -> Result<()> {
        self.db.conn().execute_batch(ENSURE_SQL)?;
        Ok(())
    }

    pub fn insert(&mut self, attestation: &Attestation) -> Result<()> {
        self.ensure_schema()?;
        self.db.conn().execute(
            "INSERT INTO eas_attestations
             (id, schema_uid, kind, fields_json, data_hash, created_at, tx_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                attestation.id,
                attestation.schema_uid,
                attestation.kind.as_str(),
                attestation.fields_json,
                attestation.data_hash,
                attestation.created_at.to_rfc3339(),
                attestation.tx_hash,
            ],
        )?;
        Ok(())
    }

    pub fn count(&mut self) -> Result<i64> {
        self.ensure_schema()?;
        Ok(self
            .db
            .conn()
            .query_row("SELECT COUNT(*) FROM eas_attestations", [], |r| r.get(0))?)
    }

    pub fn count_by_kind(&mut self, kind: AttestationKind) -> Result<i64> {
        self.ensure_schema()?;
        Ok(self.db.conn().query_row(
            "SELECT COUNT(*) FROM eas_attestations WHERE kind = ?1",
            [kind.as_str()],
            |r| r.get(0),
        )?)
    }

    pub fn mark_submitted(&mut self, id: &str, tx_hash: &str) -> Result<()> {
        self.ensure_schema()?;
        self.db.conn().execute(
            "UPDATE eas_attestations SET tx_hash = ?1 WHERE id = ?2",
            params![tx_hash, id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attest::{Attestation, AttestationData, AttestationKind};
    use openxgram_db::DbConfig;
    use serde_json::json;

    fn open_db() -> Db {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut db = Db::open(DbConfig {
            path: tmp.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        std::mem::forget(tmp);
        db
    }

    #[test]
    fn ensure_schema_is_idempotent() {
        let mut db = open_db();
        let mut s = AttestationStore::new(&mut db);
        s.ensure_schema().unwrap();
        s.ensure_schema().unwrap();
        assert_eq!(s.count().unwrap(), 0);
    }

    #[test]
    fn insert_and_count_by_kind() {
        let mut db = open_db();
        let mut s = AttestationStore::new(&mut db);
        let m = Attestation::new(AttestationData {
            kind: AttestationKind::Message,
            fields: json!({"from": "0xa"}),
        });
        let p = Attestation::new(AttestationData {
            kind: AttestationKind::Payment,
            fields: json!({"sender": "0xa", "recipient": "0xb"}),
        });
        s.insert(&m).unwrap();
        s.insert(&p).unwrap();
        assert_eq!(s.count().unwrap(), 2);
        assert_eq!(s.count_by_kind(AttestationKind::Message).unwrap(), 1);
        assert_eq!(s.count_by_kind(AttestationKind::Payment).unwrap(), 1);
        assert_eq!(s.count_by_kind(AttestationKind::Endorsement).unwrap(), 0);
    }

    #[test]
    fn mark_submitted_sets_tx_hash() {
        let mut db = open_db();
        let mut s = AttestationStore::new(&mut db);
        let m = Attestation::new(AttestationData {
            kind: AttestationKind::Endorsement,
            fields: json!({"endorser": "0xa", "endorsee": "0xb", "tag": "rust"}),
        });
        let id = m.id.clone();
        s.insert(&m).unwrap();
        s.mark_submitted(&id, "0xabc123").unwrap();
        let tx: String = db
            .conn()
            .query_row(
                "SELECT tx_hash FROM eas_attestations WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tx, "0xabc123");
    }
}
