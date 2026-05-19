//! xgram mistake — 실수 레지스트리 CLI (check/log/find/resolve).
//!
//! PRD-OpenXgram §4.2. W 규칙 1.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_mistakes::{mcp::MistakeTools, NewMistake};

#[derive(Debug, Clone)]
pub enum MistakeAction {
    Check {
        planned: String,
        k: Option<usize>,
    },
    Log {
        session_id: String,
        intended: String,
        outcome: String,
        reason: String,
        lesson: String,
        severity: Option<u8>,
        related_wiki: Option<String>,
    },
    Find {
        situation: String,
        k: Option<usize>,
    },
    Resolve {
        id: String,
        resolution: String,
    },
}

pub fn run_mistake(data_dir: &Path, action: MistakeAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let conn: &rusqlite::Connection = db.conn();
    let tools = MistakeTools::new(conn);

    match action {
        MistakeAction::Check { planned, k } => {
            let r = tools
                .check(&planned, k.unwrap_or(5))
                .context("check_for_mistakes 실패")?;
            println!("planned: {}", r.planned_action);
            println!("similar_count: {}", r.similar_count);
            if r.hits.is_empty() {
                println!("(과거 유사 실수 없음 — 안전 진행 가능)");
                return Ok(());
            }
            for w in &r.warnings {
                println!("{}", w);
            }
        }
        MistakeAction::Log {
            session_id,
            intended,
            outcome,
            reason,
            lesson,
            severity,
            related_wiki,
        } => {
            let input = NewMistake {
                session_id,
                intended_action: intended,
                actual_outcome: outcome,
                failure_reason: reason,
                lesson,
                severity,
                related_wiki,
            };
            let r = tools.log(input).context("log_mistake 실패")?;
            println!(
                "✓ logged {} (severity={}, occurred_at={})",
                r.id, r.severity, r.occurred_at
            );
        }
        MistakeAction::Find { situation, k } => {
            let hits = tools
                .find_similar(&situation, k.unwrap_or(5))
                .context("find_similar_failures 실패")?;
            if hits.is_empty() {
                println!("(매칭 없음)");
                return Ok(());
            }
            println!("matches: {}", hits.len());
            for h in &hits {
                println!(
                    "  [{}] sev={} resolved={} — {}",
                    h.id, h.severity, h.resolved, h.intended_action
                );
                println!("    lesson: {}", h.lesson);
            }
        }
        MistakeAction::Resolve { id, resolution } => {
            tools
                .resolve(&id, &resolution)
                .context("resolve_mistake 실패")?;
            println!("✓ resolved {}", id);
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
