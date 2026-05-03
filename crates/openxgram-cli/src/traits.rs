//! xgram traits — L4 정체성·성향 CLI (set / get / list).
//!
//! Phase 1: manual source 만 CLI 노출. derived 자동 도출은 후속 (PatternStore + 야간 reflection).

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{TraitSource, TraitStore};

#[derive(Debug, Clone)]
pub enum TraitsAction {
    Set {
        name: String,
        value: String,
        source: TraitSource,
        refs: Vec<String>,
    },
    Get {
        name: String,
    },
    List,
}

pub fn run_traits(data_dir: &Path, action: TraitsAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let mut store = TraitStore::new(&mut db);

    match action {
        TraitsAction::Set {
            name,
            value,
            source,
            refs,
        } => {
            let t = store.insert_or_update(&name, &value, source, &refs)?;
            println!("✓ trait 저장");
            println!("  id        : {}", t.id);
            println!("  name      : {}", t.name);
            println!("  value     : {}", t.value);
            println!("  source    : {}", t.source.as_str());
            println!("  refs      : {:?}", t.source_refs);
            println!("  updated_at: {}", t.updated_at);
        }
        TraitsAction::Get { name } => {
            match store.get_by_name(&name)? {
                Some(t) => {
                    println!("trait: {}", t.name);
                    println!("  id        : {}", t.id);
                    println!("  value     : {}", t.value);
                    println!("  source    : {}", t.source.as_str());
                    println!("  refs      : {:?}", t.source_refs);
                    println!("  created_at: {}", t.created_at);
                    println!("  updated_at: {}", t.updated_at);
                }
                None => bail!("trait 없음: {name}"),
            }
        }
        TraitsAction::List => {
            let traits = store.list()?;
            if traits.is_empty() {
                println!("traits 비어있음.");
                return Ok(());
            }
            println!("traits ({})", traits.len());
            for t in &traits {
                println!(
                    "  {} = {} (source={}, updated={})",
                    t.name,
                    t.value,
                    t.source.as_str(),
                    t.updated_at
                );
            }
        }
    }
    Ok(())
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    if !path.exists() {
        bail!(
            "DB 미존재 ({}). `xgram init` 먼저 실행.",
            path.display()
        );
    }
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}
