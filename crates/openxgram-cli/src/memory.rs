//! xgram memory — L2 memories CLI (add/list/pin/unpin).
//!
//! Phase 1: 간단 CRUD. 회상 점수·임베딩 통합·NEW/RECURRING/ROUTINE
//! 분류기는 후속 PR.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{
    default_embedder, export_claude, import_claude, parse_claude, MemoryKind, MemoryStore,
    MessageStore,
};

/// memory export/import 포맷.
#[derive(Debug, Clone, Copy)]
pub enum MemoryExportFmt {
    /// Claude 호환 markdown (카테고리별 entry, single code block).
    Claude,
}

#[derive(Debug, Clone)]
pub enum MemoryAction {
    Add {
        kind: MemoryKind,
        content: String,
        session_id: Option<String>,
    },
    List {
        /// None 이면 모든 kind (fact/decision/reference/rule) 출력.
        kind: Option<MemoryKind>,
    },
    Pin {
        id: String,
    },
    Unpin {
        id: String,
    },
    /// 같은 conversation_id 로 묶인 모든 메시지 (timestamp 오름차순) 출력.
    ShowConversation {
        id: String,
    },
}

pub fn run_memory(data_dir: &Path, action: MemoryAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    if let MemoryAction::ShowConversation { id } = &action {
        return show_conversation(&mut db, id);
    }
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
            println!(
                "  session   : {}",
                m.session_id.as_deref().unwrap_or("(none)")
            );
            println!("  created_at: {}", m.created_at);
        }
        MemoryAction::List { kind } => {
            let kinds: Vec<MemoryKind> = match kind {
                Some(k) => vec![k],
                None => vec![
                    MemoryKind::Fact,
                    MemoryKind::Decision,
                    MemoryKind::Reference,
                    MemoryKind::Rule,
                ],
            };
            let mut total = 0usize;
            for k in kinds {
                let memories = store.list_by_kind(k)?;
                if memories.is_empty() {
                    continue;
                }
                total += memories.len();
                println!("{k} memories ({})", memories.len());
                for m in &memories {
                    let pin = if m.pinned { "📌" } else { "  " };
                    println!(
                        "  {pin} {} — {} (acc={}, last={})",
                        m.id, m.content, m.access_count, m.last_accessed
                    );
                }
            }
            if total == 0 {
                println!("memory 없음.");
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
        MemoryAction::ShowConversation { .. } => unreachable!("handled above"),
    }
    Ok(())
}

fn show_conversation(db: &mut Db, conversation_id: &str) -> Result<()> {
    let embedder = default_embedder().context("embedder 초기화")?;
    let msgs = MessageStore::new(db, embedder.as_ref())
        .list_for_conversation(conversation_id)
        .context("conversation 조회")?;
    if msgs.is_empty() {
        println!("conversation {conversation_id}: 메시지 없음.");
        return Ok(());
    }
    println!("conversation {conversation_id} ({} messages)", msgs.len());
    for m in &msgs {
        let preview = m.body.lines().next().unwrap_or("").chars().take(160).collect::<String>();
        println!(
            "  [{}] {} {} → {}",
            m.timestamp.format("%Y-%m-%d %H:%M:%S"),
            m.session_id,
            m.sender,
            preview
        );
    }
    Ok(())
}

/// L2 memories + L4 traits 를 Claude 호환 markdown 으로 export.
/// `output` 이 Some 이면 파일에 기록, None 이면 stdout.
pub fn run_export(data_dir: &Path, output: Option<&Path>, fmt: MemoryExportFmt) -> Result<()> {
    let mut db = open_db(data_dir)?;
    match fmt {
        MemoryExportFmt::Claude => {
            let exp = export_claude(&mut db).context("Claude export 실패")?;
            let md = exp.render_markdown();
            match output {
                Some(p) => {
                    fs::write(p, &md)
                        .with_context(|| format!("export 파일 쓰기 실패 ({})", p.display()))?;
                    println!("✓ export 완료 → {}", p.display());
                }
                None => {
                    print!("{md}");
                }
            }
        }
    }
    Ok(())
}

/// Claude 호환 markdown 파일을 읽어 memories/traits 를 import.
pub fn run_import(data_dir: &Path, input: &Path, fmt: MemoryExportFmt) -> Result<()> {
    if !input.exists() {
        bail!("import 입력 파일 미존재 ({})", input.display());
    }
    let text = fs::read_to_string(input)
        .with_context(|| format!("import 파일 읽기 실패 ({})", input.display()))?;
    let mut db = open_db(data_dir)?;
    match fmt {
        MemoryExportFmt::Claude => {
            let parsed = parse_claude(&text);
            let summary = import_claude(&mut db, &parsed).context("Claude import 실패")?;
            println!("✓ import 완료");
            println!("  memories  : {}", summary.memories_inserted);
            println!("  traits    : {}", summary.traits_inserted);
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
