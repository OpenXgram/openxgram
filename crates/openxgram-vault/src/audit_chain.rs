//! Hash chain + Merkle checkpoint (PRD-AUDIT-01, 02, 03, 04).
//!
//! Vault audit row 마다 prev_hash + entry_hash + seq 를 자동 산출.
//! 1시간마다 audit_checkpoint(seq, merkle_root, signature) 추가 — master keypair 로 서명.
//! verify: chain 연속성 + 체크포인트 서명 + merkle root 재계산 비교.
//!
//! Canonical row serialization: deterministic JSON (key 정렬). entry_hash = SHA256(prev_hash || canonical_bytes).

use openxgram_db::Db;
use openxgram_keystore::Keypair;
use rs_merkle::algorithms::Sha256 as Sha256Algo;
use rs_merkle::MerkleTree;
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum AuditChainError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid chain at seq {seq}: {reason}")]
    Broken { seq: i64, reason: String },
    #[error("checkpoint signature invalid at seq {seq}")]
    BadSignature { seq: i64 },
    #[error("missing checkpoint signer pubkey")]
    MissingSigner,
    #[error("hex decode: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("k256 ecdsa: {0}")]
    Ecdsa(String),
}

pub type Result<T> = std::result::Result<T, AuditChainError>;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: String,
    pub key: String,
    pub agent: String,
    pub action: String,
    pub allowed: bool,
    pub reason: Option<String>,
    pub timestamp: String,
}

/// row 의 canonical JSON bytes — key 정렬 보장 (BTreeMap).
pub fn canonical_bytes(e: &AuditEntry) -> Vec<u8> {
    use std::collections::BTreeMap;
    let mut m: BTreeMap<&str, serde_json::Value> = BTreeMap::new();
    m.insert("id", serde_json::Value::String(e.id.clone()));
    m.insert("key", serde_json::Value::String(e.key.clone()));
    m.insert("agent", serde_json::Value::String(e.agent.clone()));
    m.insert("action", serde_json::Value::String(e.action.clone()));
    m.insert("allowed", serde_json::Value::Bool(e.allowed));
    m.insert(
        "reason",
        e.reason
            .clone()
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    m.insert("timestamp", serde_json::Value::String(e.timestamp.clone()));
    serde_json::to_vec(&m).expect("BTreeMap serialize 가능")
}

/// SHA256(prev_hash || canonical_bytes(entry)) — 32 bytes.
pub fn chain_hash(prev: &[u8], entry: &AuditEntry) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(canonical_bytes(entry));
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

/// 다음 seq 와 prev_hash 를 조회 — 빈 chain 이면 (1, [0;32]).
pub fn next_seq_and_prev(db: &mut Db) -> Result<(i64, [u8; 32])> {
    let conn = db.conn();
    let row = conn
        .query_row(
            "SELECT seq, entry_hash FROM vault_audit
             WHERE seq IS NOT NULL ORDER BY seq DESC LIMIT 1",
            [],
            |r| {
                let seq: Option<i64> = r.get(0)?;
                let h: Option<Vec<u8>> = r.get(1)?;
                Ok((seq, h))
            },
        )
        .ok();
    match row {
        Some((Some(seq), Some(h))) => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&h[..32.min(h.len())]);
            Ok((seq + 1, arr))
        }
        _ => Ok((1, [0u8; 32])),
    }
}

/// 기존 audit row 에 prev_hash/entry_hash/seq 채움. 이미 채워진 row 는 건너뜀.
/// PRD-AUDIT-01 의 "INSERT 시 자동 채움" 의 batch 변형 — 마이그레이션 후 backfill 용도.
pub fn backfill_chain(db: &mut Db) -> Result<usize> {
    let conn = db.conn();
    let mut rows: Vec<AuditEntry> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, key, agent, action, allowed, reason, timestamp
             FROM vault_audit WHERE seq IS NULL ORDER BY timestamp ASC",
        )?;
        let mapped = stmt.query_map([], |r| {
            Ok(AuditEntry {
                id: r.get(0)?,
                key: r.get(1)?,
                agent: r.get(2)?,
                action: r.get(3)?,
                allowed: r.get::<_, i64>(4)? != 0,
                reason: r.get(5)?,
                timestamp: r.get(6)?,
            })
        })?;
        for row in mapped {
            rows.push(row?);
        }
    }
    let (mut next_seq, mut prev_hash) = next_seq_and_prev(db)?;
    let conn = db.conn();
    for e in &rows {
        let h = chain_hash(&prev_hash, e);
        conn.execute(
            "UPDATE vault_audit SET prev_hash = ?1, entry_hash = ?2, seq = ?3 WHERE id = ?4",
            params![&prev_hash[..], &h[..], next_seq, e.id],
        )?;
        prev_hash = h;
        next_seq += 1;
    }
    Ok(rows.len())
}

/// chain 무결성 검증 — seq 연속 + entry_hash 재계산 일치.
pub fn verify_chain(db: &mut Db) -> Result<()> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, key, agent, action, allowed, reason, timestamp, prev_hash, entry_hash, seq
         FROM vault_audit WHERE seq IS NOT NULL ORDER BY seq ASC",
    )?;
    let mut iter = stmt.query([])?;
    let mut expected_seq = 1i64;
    let mut expected_prev = [0u8; 32];
    while let Some(row) = iter.next()? {
        let entry = AuditEntry {
            id: row.get(0)?,
            key: row.get(1)?,
            agent: row.get(2)?,
            action: row.get(3)?,
            allowed: row.get::<_, i64>(4)? != 0,
            reason: row.get(5)?,
            timestamp: row.get(6)?,
        };
        let prev_db: Vec<u8> = row.get(7)?;
        let entry_db: Vec<u8> = row.get(8)?;
        let seq_db: i64 = row.get(9)?;

        if seq_db != expected_seq {
            return Err(AuditChainError::Broken {
                seq: seq_db,
                reason: format!("seq 불연속: 기대 {expected_seq} 실제 {seq_db}"),
            });
        }
        if prev_db != expected_prev {
            return Err(AuditChainError::Broken {
                seq: seq_db,
                reason: "prev_hash 불일치".to_string(),
            });
        }
        let recomputed = chain_hash(&expected_prev, &entry);
        if entry_db != recomputed {
            return Err(AuditChainError::Broken {
                seq: seq_db,
                reason: "entry_hash 불일치 (변조 의심)".to_string(),
            });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&entry_db[..32.min(entry_db.len())]);
        expected_prev = arr;
        expected_seq += 1;
    }
    Ok(())
}

/// since_seq~latest 까지의 entry_hash 들을 leaf 로 Merkle root 계산.
pub fn merkle_root_since(db: &mut Db, since_seq: i64) -> Result<Option<[u8; 32]>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT entry_hash FROM vault_audit WHERE seq IS NOT NULL AND seq >= ?1 ORDER BY seq ASC",
    )?;
    let leaves: Vec<[u8; 32]> = stmt
        .query_map(params![since_seq], |r| {
            let v: Vec<u8> = r.get(0)?;
            let mut a = [0u8; 32];
            a.copy_from_slice(&v[..32.min(v.len())]);
            Ok(a)
        })?
        .filter_map(|x| x.ok())
        .collect();
    if leaves.is_empty() {
        return Ok(None);
    }
    let tree = MerkleTree::<Sha256Algo>::from_leaves(&leaves);
    Ok(tree.root())
}

/// master 로 Merkle root 서명 후 audit_checkpoint row 추가. seq = 마지막 chain seq.
pub fn create_checkpoint(db: &mut Db, master: &Keypair) -> Result<Option<i64>> {
    let conn = db.conn();
    // 마지막 seq
    let last_seq: Option<i64> = conn
        .query_row(
            "SELECT MAX(seq) FROM vault_audit WHERE seq IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let Some(last_seq) = last_seq else {
        return Ok(None);
    };

    // 마지막 체크포인트 이후 since_seq
    let since_seq: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM audit_checkpoint",
            [],
            |r| r.get(0),
        )
        .unwrap_or(1);
    if since_seq > last_seq {
        return Ok(None);
    }

    let root = merkle_root_since(db, since_seq)?.unwrap_or([0u8; 32]);
    let signature = master.sign(&root);
    let signer_pk_hex = hex::encode(master.public_key_bytes());
    let now_kst = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap())
        .timestamp();

    let conn = db.conn();
    conn.execute(
        "INSERT INTO audit_checkpoint (seq, merkle_root, signature, signer_pubkey_hex, signed_at_kst)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![last_seq, &root[..], &signature[..], signer_pk_hex, now_kst],
    )?;
    Ok(Some(last_seq))
}

/// 모든 체크포인트의 서명을 검증 + merkle root 재계산 비교.
pub fn verify_checkpoints(db: &mut Db) -> Result<()> {
    let rows: Vec<(i64, Vec<u8>, Vec<u8>, String)> = {
        let conn = db.conn();
        let mut stmt = conn.prepare(
            "SELECT seq, merkle_root, signature, signer_pubkey_hex FROM audit_checkpoint ORDER BY seq ASC",
        )?;
        let mapped = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let collected: std::result::Result<Vec<_>, _> = mapped.collect();
        collected?
    };
    let mut prev_seq = 0i64;
    for (seq, root, sig, pk_hex) in rows {
        let pk_bytes = hex::decode(&pk_hex)?;
        openxgram_keystore::verify_with_pubkey(&pk_hex, &root, &sig)
            .map_err(|_| AuditChainError::BadSignature { seq })?;
        // pk_bytes 검증 — 33 bytes (compressed secp256k1)
        if pk_bytes.len() != 33 {
            return Err(AuditChainError::BadSignature { seq });
        }
        // 재계산
        let recomputed = merkle_root_since(db, prev_seq + 1)?.unwrap_or([0u8; 32]);
        if recomputed.as_slice() != root.as_slice() {
            return Err(AuditChainError::Broken {
                seq,
                reason: "merkle_root 재계산 불일치 (변조 의심)".to_string(),
            });
        }
        prev_seq = seq;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_db::DbConfig;
    use openxgram_keystore::{FsKeystore, Keystore};
    use tempfile::tempdir;

    fn open_db() -> Db {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("audit.db");
        let mut db = Db::open(DbConfig {
            path,
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        std::mem::forget(tmp);
        db
    }

    fn make_master() -> Keypair {
        let tmp = tempdir().unwrap();
        let ks = FsKeystore::new(tmp.path());
        let _ = ks.create("m", "p").unwrap();
        let m = ks.load("m", "p").unwrap();
        std::mem::forget(tmp);
        m
    }

    fn insert_audit_row(db: &mut Db, id: &str, key: &str, ts: &str) {
        let conn = db.conn();
        conn.execute(
            "INSERT INTO vault_audit (id, key, agent, action, allowed, reason, timestamp)
             VALUES (?1, ?2, 'agent-A', 'get', 1, NULL, ?3)",
            params![id, key, ts],
        )
        .unwrap();
    }

    #[test]
    fn canonical_bytes_deterministic() {
        let e1 = AuditEntry {
            id: "1".into(),
            key: "k".into(),
            agent: "a".into(),
            action: "get".into(),
            allowed: true,
            reason: None,
            timestamp: "2026-05-04T00:00:00+09:00".into(),
        };
        let b1 = canonical_bytes(&e1);
        let b2 = canonical_bytes(&e1);
        assert_eq!(b1, b2);
    }

    #[test]
    fn chain_hash_includes_prev() {
        let e = AuditEntry {
            id: "1".into(),
            key: "k".into(),
            agent: "a".into(),
            action: "get".into(),
            allowed: true,
            reason: None,
            timestamp: "2026-05-04T00:00:00+09:00".into(),
        };
        let h1 = chain_hash(&[0u8; 32], &e);
        let h2 = chain_hash(&[1u8; 32], &e);
        assert_ne!(h1, h2);
    }

    #[test]
    fn backfill_then_verify_passes() {
        let mut db = open_db();
        insert_audit_row(&mut db, "1", "k1", "2026-05-04T00:00:00+09:00");
        insert_audit_row(&mut db, "2", "k2", "2026-05-04T00:00:01+09:00");
        insert_audit_row(&mut db, "3", "k3", "2026-05-04T00:00:02+09:00");
        let n = backfill_chain(&mut db).unwrap();
        assert_eq!(n, 3);
        verify_chain(&mut db).unwrap();
    }

    #[test]
    fn delete_middle_row_breaks_verify() {
        let mut db = open_db();
        insert_audit_row(&mut db, "1", "k1", "2026-05-04T00:00:00+09:00");
        insert_audit_row(&mut db, "2", "k2", "2026-05-04T00:00:01+09:00");
        insert_audit_row(&mut db, "3", "k3", "2026-05-04T00:00:02+09:00");
        backfill_chain(&mut db).unwrap();
        // 직접 row 삭제 (PRD-AUDIT-04 fault injection)
        db.conn()
            .execute("DELETE FROM vault_audit WHERE id = '2'", [])
            .unwrap();
        let err = verify_chain(&mut db);
        assert!(err.is_err(), "삭제 후 verify 가 깨져야 함");
    }

    #[test]
    fn update_row_breaks_verify() {
        let mut db = open_db();
        insert_audit_row(&mut db, "1", "k1", "2026-05-04T00:00:00+09:00");
        insert_audit_row(&mut db, "2", "k2", "2026-05-04T00:00:01+09:00");
        backfill_chain(&mut db).unwrap();
        db.conn()
            .execute(
                "UPDATE vault_audit SET reason = 'tampered' WHERE id = '1'",
                [],
            )
            .unwrap();
        let err = verify_chain(&mut db);
        assert!(err.is_err(), "변조 후 verify 가 깨져야 함");
    }

    #[test]
    fn checkpoint_round_trip_verifies() {
        let mut db = open_db();
        let master = make_master();
        insert_audit_row(&mut db, "1", "k1", "2026-05-04T00:00:00+09:00");
        insert_audit_row(&mut db, "2", "k2", "2026-05-04T00:00:01+09:00");
        backfill_chain(&mut db).unwrap();
        let cp = create_checkpoint(&mut db, &master).unwrap();
        assert_eq!(cp, Some(2));
        verify_checkpoints(&mut db).unwrap();
    }

    #[test]
    fn checkpoint_signature_tampering_breaks() {
        let mut db = open_db();
        let master = make_master();
        insert_audit_row(&mut db, "1", "k1", "2026-05-04T00:00:00+09:00");
        backfill_chain(&mut db).unwrap();
        create_checkpoint(&mut db, &master).unwrap();
        // 임의 서명 변조
        db.conn()
            .execute(
                "UPDATE audit_checkpoint SET signature = ?1",
                params![&[0u8; 64][..]],
            )
            .unwrap();
        let err = verify_checkpoints(&mut db);
        assert!(err.is_err(), "서명 변조 후 verify 깨져야 함");
    }
}
