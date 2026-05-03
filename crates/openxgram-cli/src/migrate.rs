//! xgram migrate — DB 마이그레이션 실행 + 적용 상태 출력.
//!
//! Phase 1: latest 까지 일괄 적용 (db crate 가 idempotent). target 버전
//! 지정 (--target) 은 후속 PR — 현재는 무시하고 latest 만.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};

#[derive(Debug, Clone)]
pub struct MigrateOpts {
    pub data_dir: PathBuf,
    pub target: Option<String>,
}

pub fn run_migrate(opts: &MigrateOpts) -> Result<()> {
    let path = db_path(&opts.data_dir);
    if !path.exists() {
        bail!(
            "DB 파일 미존재 ({}). `xgram init --alias <NAME>` 먼저 실행.",
            path.display()
        );
    }
    if let Some(t) = &opts.target {
        println!("⚠ --target {t} 은 Phase 1.5 에서 지원. 현재는 latest 까지 적용합니다.");
    }

    println!("xgram migrate");
    println!("  DB: {}", path.display());

    let mut db = Db::open(DbConfig {
        path: path.clone(),
        ..Default::default()
    })
    .context("DB open 실패")?;

    let before: Vec<u32> = db
        .list_applied_migrations()
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.version)
        .collect();
    db.migrate().context("migrate 실패")?;
    let after = db.list_applied_migrations()?;

    let new_versions: Vec<u32> = after
        .iter()
        .map(|r| r.version)
        .filter(|v| !before.contains(v))
        .collect();
    println!();
    if new_versions.is_empty() {
        let latest = after.iter().map(|r| r.version).max().unwrap_or(0);
        println!("✓ 모든 마이그레이션 이미 적용됨 (latest version: {latest})");
    } else {
        println!("✓ 신규 적용 {} 건: {:?}", new_versions.len(), new_versions);
    }
    println!("  적용된 마이그레이션 ({}):", after.len());
    for r in &after {
        println!("    [{}] {} — {}", r.version, r.name, r.applied_at);
    }
    Ok(())
}
