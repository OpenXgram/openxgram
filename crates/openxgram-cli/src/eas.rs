//! step 18 — EAS attestation CLI (`xgram eas list/count`).
//!
//! 자동 attestation hook (MessageStore.insert / PaymentStore.mark_confirmed 후 자동) 은 별도 작업.
//! 본 모듈은 조회/수동 attest 만.

use anyhow::{Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_eas::{Attestation, AttestationData, AttestationKind, AttestationStore};
use std::path::Path;

pub fn run_list(data_dir: &Path, limit: usize) -> Result<()> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open")?;
    db.migrate().context("DB migrate")?;
    AttestationStore::new(&mut db).ensure_schema()?;

    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, schema_uid, kind, fields_json, created_at, tx_hash
         FROM eas_attestations ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut count = 0;
    println!("{:<14} {:<14} {:<26} {}", "kind", "uid(prefix)", "created_at", "fields");
    for row in rows {
        let (_id, uid, kind, fields, ts, _tx) = row?;
        let preview: String = fields.chars().take(60).collect();
        println!(
            "{:<14} {:<14} {:<26} {}",
            kind,
            &uid[..14],
            ts.chars().take(26).collect::<String>(),
            preview
        );
        count += 1;
    }
    if count == 0 {
        println!("(attestation 없음)");
    }
    Ok(())
}

pub fn run_count(data_dir: &Path) -> Result<()> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let mut store = AttestationStore::new(&mut db);
    store.ensure_schema()?;
    let total = store.count()?;
    println!("EAS attestations 총 {total} 건");
    for kind in [
        AttestationKind::Message,
        AttestationKind::Payment,
        AttestationKind::Endorsement,
    ] {
        let n = store.count_by_kind(kind)?;
        println!("  {:<12} {n}", kind.as_str());
    }
    Ok(())
}

pub fn run_attest(
    data_dir: &Path,
    kind: AttestationKind,
    fields_json: &str,
) -> Result<()> {
    let fields: serde_json::Value =
        serde_json::from_str(fields_json).context("fields JSON 파싱")?;
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;
    let att = Attestation::new(AttestationData { kind, fields });
    AttestationStore::new(&mut db).insert(&att)?;
    println!("✓ attest 완료 — id={} kind={} uid={}", att.id, att.kind.as_str(), att.schema_uid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn count_empty_returns_zero() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let mut db = Db::open(DbConfig { path: db_path(dir), ..Default::default() }).unwrap();
        db.migrate().unwrap();
        AttestationStore::new(&mut db).ensure_schema().unwrap();
        drop(db);
        run_count(dir).unwrap();
    }

    #[test]
    fn attest_then_count_increments() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let mut db = Db::open(DbConfig { path: db_path(dir), ..Default::default() }).unwrap();
        db.migrate().unwrap();
        drop(db);
        run_attest(dir, AttestationKind::Message, r#"{"from":"a","to":"b"}"#).unwrap();
        run_attest(dir, AttestationKind::Endorsement, r#"{"endorser":"a","endorsee":"b","tag":"trust"}"#).unwrap();
        let mut db = Db::open(DbConfig { path: db_path(dir), ..Default::default() }).unwrap();
        db.migrate().unwrap();
        let mut store = AttestationStore::new(&mut db);
        assert_eq!(store.count().unwrap(), 2);
        assert_eq!(store.count_by_kind(AttestationKind::Message).unwrap(), 1);
        assert_eq!(store.count_by_kind(AttestationKind::Endorsement).unwrap(), 1);
    }
}
