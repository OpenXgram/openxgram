//! xgram audit verify — chain 무결성 + 체크포인트 서명 검증 CLI (PRD-AUDIT-03).

use std::path::Path;

use anyhow::{Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_vault::audit_chain::{verify_chain, verify_checkpoints};

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
            let n = openxgram_vault::audit_chain::backfill_chain(&mut db)
                .context("backfill 실패")?;
            Ok(VerifyReport::Backfilled(n))
        }
        AuditAction::Checkpoint => {
            // password prompt 는 CLI binding 측 — 여기서는 master 미지정 시 skip
            Ok(VerifyReport::CheckpointRequiresMaster)
        }
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
        assert!(format!("{}", VerifyReport::Failed(vec!["x".into()]))
            .contains("실패"));
    }
}
