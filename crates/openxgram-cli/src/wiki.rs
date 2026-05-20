//! xgram wiki — L2 위키 CLI (read/write/link/search/list).
//!
//! PRD-OpenXgram §4.1. openxgram-wiki 의 WikiTools 를 CLI 표면에 노출.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_wiki::{mcp::WikiTools, WikiFs};

#[derive(Debug, Clone)]
pub enum WikiAction {
    Read {
        topic: String,
    },
    Write {
        topic: String,
        content: String,
        page_type: Option<String>,
    },
    Link {
        from: String,
        to: String,
        reason: Option<String>,
    },
    Search {
        query: String,
        k: Option<usize>,
    },
    List {
        page_type: Option<String>,
    },
}

pub async fn run_wiki(data_dir: &Path, action: WikiAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let wiki_root = data_dir.join("wiki");
    let fs = WikiFs::new(&wiki_root);
    fs.ensure_dirs().await.context("wiki dirs 생성 실패")?;
    let conn: &rusqlite::Connection = db.conn();
    let tools = WikiTools::new(&fs, conn);

    match action {
        WikiAction::Read { topic } => {
            let r = tools.read(&topic).await.context("read_wiki_page 실패")?;
            println!("# {}  ({})", r.title, r.id);
            println!("type        : {}", r.page_type);
            println!("content_hash: {}", r.content_hash);
            if !r.related.is_empty() {
                println!("related     : {}", r.related.join(", "));
            }
            if !r.source_refs.is_empty() {
                println!("source_refs : {}", r.source_refs.join(", "));
            }
            println!("---");
            println!("{}", r.body);
        }
        WikiAction::Write {
            topic,
            content,
            page_type,
        } => {
            let r = tools
                .write(&topic, &content, page_type.as_deref(), None)
                .await
                .context("write_wiki_page 실패")?;
            println!(
                "✓ {} {} (hash={})",
                if r.created { "created" } else { "updated" },
                r.id,
                r.content_hash
            );
        }
        WikiAction::Link { from, to, reason } => {
            let r = tools
                .link(&from, &to, reason.as_deref())
                .await
                .context("link_concepts 실패")?;
            println!("✓ linked {} → {}", r.from, r.to);
        }
        WikiAction::Search { query, k } => {
            let hits = tools.search(&query, k).context("search_wiki 실패")?;
            if hits.is_empty() {
                println!("(매칭 없음)");
                return Ok(());
            }
            println!("hits: {}", hits.len());
            for h in &hits {
                println!("  [{:.3}] {}  — {}", h.score, h.id, h.title);
            }
        }
        WikiAction::List { page_type } => {
            let entries = tools.list(page_type.as_deref()).context("list_wiki 실패")?;
            if entries.is_empty() {
                println!("(페이지 없음)");
                return Ok(());
            }
            println!("pages: {}", entries.len());
            for e in &entries {
                println!("  [{}] {}  — {}", e.page_type, e.id, e.title);
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
