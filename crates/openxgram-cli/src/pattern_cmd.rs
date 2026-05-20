//! xgram pattern — 패턴 매칭 엔진 CLI (match/suggest/confirm/record).
//!
//! PRD-OpenXgram §4.3. W 규칙 2.
//! 기존 `xgram patterns` (L3 빈도 분류) 과는 별개 — 본 명령은 action_sequence + 성공률.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_patterns::{mcp::PatternTools, pattern::ActionStep};

#[derive(Debug, Clone)]
pub enum PatternAction {
    Match {
        new_action: String,
        k: Option<usize>,
        min_similarity: Option<f64>,
    },
    Suggest {
        current_state: String,
    },
    Confirm {
        pattern_id: String,
        modifications: Option<Vec<ActionStep>>,
    },
    Record {
        pattern_id: String,
        success: bool,
        duration_ms: Option<i64>,
    },
}

pub fn run_pattern(data_dir: &Path, action: PatternAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let conn: &rusqlite::Connection = db.conn();
    let tools = PatternTools::new(conn);

    match action {
        PatternAction::Match {
            new_action,
            k,
            min_similarity,
        } => {
            let r = tools
                .match_pattern(&new_action, k.unwrap_or(5), min_similarity.unwrap_or(0.0))
                .context("match_action_pattern 실패")?;
            println!("input: {}", r.input);
            println!("count: {}", r.count);
            if r.patterns.is_empty() {
                println!("(매칭 없음)");
                return Ok(());
            }
            for p in &r.patterns {
                let rate = p
                    .success_rate
                    .map(|v| format!("{:.2}", v))
                    .unwrap_or_else(|| "-".into());
                let dur = p
                    .avg_duration_ms
                    .map(|v| format!("{}ms", v))
                    .unwrap_or_else(|| "-".into());
                println!("  [{}] success={} avg={}", p.id, rate, dur);
                println!("    seq: {}", p.sequence);
            }
        }
        PatternAction::Suggest { current_state } => {
            let suggestions = tools
                .suggest_next(&current_state)
                .context("suggest_next_steps 실패")?;
            if suggestions.is_empty() {
                println!("(제안 없음)");
                return Ok(());
            }
            println!("suggestions: {}", suggestions.len());
            for s in &suggestions {
                let rate = s
                    .success_rate
                    .map(|v| format!("{:.2}", v))
                    .unwrap_or_else(|| "-".into());
                let tool = s.tool.clone().unwrap_or_else(|| "-".into());
                println!(
                    "  [{}] success={} tool={} step={}",
                    s.pattern_id, rate, tool, s.step
                );
            }
        }
        PatternAction::Confirm {
            pattern_id,
            modifications,
        } => {
            let r = tools
                .confirm(&pattern_id, modifications)
                .context("confirm_pattern_execution 실패")?;
            println!("pattern_id: {}", r.pattern_id);
            println!("plan ({} steps):", r.plan.len());
            for (i, step) in r.plan.iter().enumerate() {
                let tool = step.tool.clone().unwrap_or_else(|| "-".into());
                println!("  {}. {} [tool={}]", i + 1, step.step, tool);
            }
        }
        PatternAction::Record {
            pattern_id,
            success,
            duration_ms,
        } => {
            tools
                .record(&pattern_id, success, duration_ms)
                .context("record_pattern_outcome 실패")?;
            println!(
                "✓ recorded {} success={} duration_ms={:?}",
                pattern_id, success, duration_ms
            );
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
