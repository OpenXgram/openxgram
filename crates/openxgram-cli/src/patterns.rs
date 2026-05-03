//! xgram patterns — L3 행동/발화 분류 CLI (observe / list).
//!
//! Phase 1: 빈도 기반 분류 (NEW=1, RECURRING=2~4, ROUTINE=5+). 시간 간격 기반 ROUTINE
//! 임계값 조정·embedder 클러스터링은 후속.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{Classification, PatternStore};

#[derive(Debug, Clone)]
pub enum PatternsAction {
    Observe { text: String },
    List { classification: Classification },
}

pub fn run_patterns(data_dir: &Path, action: PatternsAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let mut store = PatternStore::new(&mut db);

    match action {
        PatternsAction::Observe { text } => {
            let p = store.observe(&text)?;
            println!("✓ pattern observe");
            println!("  id            : {}", p.id);
            println!("  text          : {}", p.pattern_text);
            println!("  frequency     : {}", p.frequency);
            println!("  classification: {}", p.classification.as_str());
            println!("  last_seen     : {}", p.last_seen);
        }
        PatternsAction::List { classification } => {
            let patterns = store.list_by_classification(classification)?;
            if patterns.is_empty() {
                println!("patterns ({}) 비어있음.", classification.as_str());
                return Ok(());
            }
            println!(
                "patterns ({}, {} 개)",
                classification.as_str(),
                patterns.len()
            );
            for p in &patterns {
                println!(
                    "  [{}x] {} (last={})",
                    p.frequency, p.pattern_text, p.last_seen
                );
            }
        }
    }
    Ok(())
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    if !path.exists() {
        bail!("DB 미존재 ({}). `xgram init` 먼저 실행.", path.display());
    }
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}
