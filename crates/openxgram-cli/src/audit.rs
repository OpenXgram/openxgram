//! xgram audit verify — chain 무결성 + 체크포인트 서명 검증 CLI (PRD-AUDIT-03).

use std::path::Path;

use anyhow::{Context, Result};
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_vault::audit_chain::{create_checkpoint, verify_chain, verify_checkpoints};

#[derive(Debug, Clone)]
pub enum AuditAction {
    Verify,
    Backfill,
    Checkpoint,
}

pub fn run_audit(data_dir: &Path, action: AuditAction) -> Result<VerifyReport> {
    let mut db = open_db(data_dir)?;
    match action {
        AuditAction::Verify => verify(&mut db),
        AuditAction::Backfill => {
            let n =
                openxgram_vault::audit_chain::backfill_chain(&mut db).context("backfill 실패")?;
            Ok(VerifyReport::Backfilled(n))
        }
        AuditAction::Checkpoint => {
            // password prompt 는 CLI binding 측 — 여기서는 master 미지정 시 skip
            Ok(VerifyReport::CheckpointRequiresMaster)
        }
    }
}

/// master 패스워드를 받아 즉시 체크포인트를 생성한다 (PRD-AUDIT-03).
///
/// 빈 chain 이거나 새 entry 가 없으면 `CheckpointSkipped`,
/// 생성 성공 시 `CheckpointCreated(seq)` 를 반환한다.
pub fn run_audit_checkpoint(data_dir: &Path, password: &str) -> Result<VerifyReport> {
    let mut db = open_db(data_dir)?;
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let master = ks
        .load(MASTER_KEY_NAME, password)
        .context("master 키 로드 실패")?;
    match create_checkpoint(&mut db, &master).context("checkpoint 생성 실패")? {
        Some(seq) => Ok(VerifyReport::CheckpointCreated(seq)),
        None => Ok(VerifyReport::CheckpointSkipped),
    }
}

pub fn verify(db: &mut Db) -> Result<VerifyReport> {
    let chain_result = verify_chain(db);
    let cp_result = verify_checkpoints(db);
    let mut errs: Vec<String> = Vec::new();
    if let Err(e) = chain_result {
        errs.push(format!("chain: {e}"));
    }
    if let Err(e) = cp_result {
        errs.push(format!("checkpoint: {e}"));
    }
    if errs.is_empty() {
        Ok(VerifyReport::Ok)
    } else {
        Ok(VerifyReport::Failed(errs))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyReport {
    Ok,
    Failed(Vec<String>),
    Backfilled(usize),
    CheckpointRequiresMaster,
    CheckpointCreated(i64),
    CheckpointSkipped,
}

impl std::fmt::Display for VerifyReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => write!(f, "✓ audit chain + checkpoints 모두 정상"),
            Self::Failed(errs) => {
                writeln!(f, "✗ audit verify 실패 ({} 항목)", errs.len())?;
                for (i, e) in errs.iter().enumerate() {
                    writeln!(f, "  {}. {}", i + 1, e)?;
                }
                Ok(())
            }
            Self::Backfilled(n) => write!(f, "✓ {n} 개 audit row 에 hash chain backfill 완료"),
            Self::CheckpointRequiresMaster => {
                write!(f, "checkpoint 생성은 master 패스워드가 필요 — `xgram audit checkpoint --password=…`")
            }
            Self::CheckpointCreated(seq) => {
                write!(f, "✓ checkpoint 생성 완료 (seq={seq})")
            }
            Self::CheckpointSkipped => {
                write!(
                    f,
                    "ℹ checkpoint 생략 — 새 audit entry 가 없거나 chain 이 비어있음"
                )
            }
        }
    }
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::tempdir;

    fn fixture_dir() -> tempfile::TempDir {
        tempdir().unwrap()
    }

    fn insert_row(db: &mut Db, id: &str, ts: &str) {
        db.conn()
            .execute(
                "INSERT INTO vault_audit (id, key, agent, action, allowed, reason, timestamp)
                 VALUES (?1, 'k', 'a', 'get', 1, NULL, ?2)",
                params![id, ts],
            )
            .unwrap();
    }

    #[test]
    fn backfill_then_verify_returns_ok() {
        let dir = fixture_dir();
        // db_path 가 가리키는 파일에 직접 init
        let path = db_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut db = Db::open(DbConfig {
            path: path.clone(),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        insert_row(&mut db, "1", "2026-05-04T00:00:00+09:00");
        insert_row(&mut db, "2", "2026-05-04T00:00:01+09:00");
        drop(db);

        let r1 = run_audit(dir.path(), AuditAction::Backfill).unwrap();
        assert!(matches!(r1, VerifyReport::Backfilled(2)));
        let r2 = run_audit(dir.path(), AuditAction::Verify).unwrap();
        assert!(matches!(r2, VerifyReport::Ok));
    }

    #[test]
    fn report_display_formats() {
        assert!(format!("{}", VerifyReport::Ok).contains("정상"));
        assert!(format!("{}", VerifyReport::Backfilled(3)).contains("3"));
        assert!(format!("{}", VerifyReport::Failed(vec!["x".into()])).contains("실패"));
    }
}
