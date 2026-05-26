//! xgram wiki — L2 위키 CLI (read/write/link/search/list).
//!
//! PRD-OpenXgram §4.1. openxgram-wiki 의 WikiTools 를 CLI 표면에 노출.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::embed::default_embedder;
use openxgram_wiki::{mcp::WikiTools, store::WikiStore, WikiFs};

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
    /// 미임베딩 페이지 일괄 임베딩
    Embed,
}

pub async fn run_wiki(data_dir: &Path, action: WikiAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let wiki_root = data_dir.join("wiki");
    let fs = WikiFs::new(&wiki_root);
    fs.ensure_dirs().await.context("wiki dirs 생성 실패")?;
    let conn: &rusqlite::Connection = db.conn();
    let tools = WikiTools::new(&fs, conn);

    let is_write = matches!(action, WikiAction::Write { .. } | WikiAction::Embed);

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
            // 임베더 주입 — 의미(벡터) 검색 활성화. default_embedder() 는
            // Arc<dyn Embedder+Send+Sync> → &dyn Embedder 로 upcast (Rust 1.86+ 안정).
            let emb_arc = openxgram_memory::default_embedder().ok();
            let emb_ref: Option<&dyn openxgram_memory::Embedder> =
                emb_arc.as_deref().map(|e| e as &dyn openxgram_memory::Embedder);
            let hits = tools
                .search(&query, k, emb_ref)
                .context("search_wiki 실패")?;
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
        WikiAction::Embed => {
            // is_write=true 이므로 match 이후 embed_missing_wiki_pages 가 실행됨
        }
    }

    // write / embed 후 미임베딩 페이지를 채운다
    if is_write {
        if let Err(e) = embed_missing_wiki_pages(conn, &wiki_root) {
            eprintln!("wiki 임베딩 실패: {e}");
            return Err(e);
        }
    }

    Ok(())
}

/// wiki_embeddings 에 없는 wiki_pages 를 embed_passage 로 임베딩하여 저장.
/// title + 디스크 본문(body)을 텍스트로 사용.
pub fn embed_missing_wiki_pages(conn: &rusqlite::Connection, wiki_root: &Path) -> Result<()> {
    let embedder = default_embedder().context("embedder 초기화 실패")?;
    let model_label = openxgram_memory::embed::embedder_mode_label();

    let store = WikiStore::new(conn);

    // wiki_embeddings 에 없는 page_id 목록 조회
    let mut stmt = conn.prepare(
        "SELECT wp.id, wp.title, wp.file_path
         FROM wiki_pages wp
         LEFT JOIN wiki_embeddings we ON wp.id = we.page_id
         WHERE we.page_id IS NULL",
    ).context("쿼리 준비 실패")?;

    #[derive(Debug)]
    struct Row { id: String, title: String, file_path: String }

    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                title: r.get(1)?,
                file_path: r.get(2)?,
            })
        })
        .context("쿼리 실행 실패")?
        .collect::<rusqlite::Result<_>>()
        .context("row 수집 실패")?;

    if rows.is_empty() {
        return Ok(());
    }

    println!("wiki 임베딩: {} 페이지 처리 중...", rows.len());

    for row in &rows {
        // 디스크 본문 읽기 (없으면 title만 사용)
        let disk_path = wiki_root.join(&row.file_path);
        let body = std::fs::read_to_string(&disk_path)
            .unwrap_or_else(|_| String::new());
        let text = if body.is_empty() {
            row.title.clone()
        } else {
            format!("{}\n\n{}", row.title, body)
        };

        let vec = embedder.embed_passage(&text);

        let page_id: openxgram_wiki::PageId = row.id.parse()
            .map_err(|e| anyhow::anyhow!("page_id 파싱 실패 {}: {e}", row.id))?;

        store.upsert_embedding(&page_id, &vec, model_label)
            .map_err(|e| anyhow::anyhow!("임베딩 저장 실패 {}: {e}", row.id))?;

        println!("  ✓ {}", row.id);
    }

    println!("wiki 임베딩 완료: {} 페이지", rows.len());
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
