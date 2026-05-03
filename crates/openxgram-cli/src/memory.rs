//! xgram memory — L2 memories CLI (add/list/pin/unpin).
//!
//! Phase 1: 간단 CRUD. 회상 점수·임베딩 통합·NEW/RECURRING/ROUTINE
//! 분류기는 후속 PR.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{MemoryKind, MemoryStore};

#[derive(Debug, Clone)]
pub enum MemoryAction {
    Add {
        kind: MemoryKind,
        content: String,
        session_id: Option<String>,
    },
    List {
        kind: MemoryKind,
    },
    Pin {
        id: String,
    },
    Unpin {
        id: String,
    },
}

pub fn run_memory(data_dir: &Path, action: MemoryAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let mut store = MemoryStore::new(&mut db);
    match action {
        MemoryAction::Add {
            kind,
            content,
            session_id,
        } => {
            let m = store.insert(session_id.as_deref(), kind, &content)?;
            println!("✓ memory 저장");
            println!("  id        : {}", m.id);
            println!("  kind      : {}", m.kind);
            println!("  session   : {}", m.session_id.as_deref().unwrap_or("(none)"));
            println!("  created_at: {}", m.created_at);
        }
        MemoryAction::List { kind } => {
            let memories = store.list_by_kind(kind)?;
            if memories.is_empty() {
                println!("{kind} memory 없음.");
                return Ok(());
            }
            println!("{kind} memories ({})", memories.len());
            for m in &memories {
                let pin = if m.pinned { "📌" } else { "  " };
                println!(
                    "  {pin} {} — {} (acc={}, last={})",
                    m.id, m.content, m.access_count, m.last_accessed
                );
            }
        }
        MemoryAction::Pin { id } => {
            store.set_pinned(&id, true)?;
            println!("✓ pinned: {id}");
        }
        MemoryAction::Unpin { id } => {
            store.set_pinned(&id, false)?;
            println!("✓ unpinned: {id}");
        }
    }
    Ok(())
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    if !path.exists() {
        bail!(
            "DB 파일 미존재 ({}). `xgram init --alias <NAME>` 먼저 실행.",
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
