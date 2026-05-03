//! xgram session — 대화 컨테이너 CRUD + 메시지 추가 + reflection.
//!
//! Phase 1 first PR: new / list / show / message / reflect.
//! recall (KNN 검색)·delete 는 후속.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::env::require_password;
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_memory::{
    export_session, import_session, reflect_all, reflect_session, DummyEmbedder, EpisodeStore,
    MessageStore, SessionStore, TextPackage,
};

#[derive(Debug, Clone)]
pub enum SessionAction {
    New { title: String },
    List,
    Show { id: String },
    Message { session_id: String, sender: String, body: String },
    Reflect { session_id: String },
    Recall { query: String, k: usize },
    Export { session_id: String, out: Option<std::path::PathBuf> },
    Import { input: Option<std::path::PathBuf> },
    Delete { id: String },
    ReflectAll,
}

pub fn run_session(data_dir: &Path, action: SessionAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    match action {
        SessionAction::New { title } => cmd_new(&mut db, &title),
        SessionAction::List => cmd_list(&mut db),
        SessionAction::Show { id } => cmd_show(&mut db, &id),
        SessionAction::Message {
            session_id,
            sender,
            body,
        } => cmd_message(&mut db, data_dir, &session_id, &sender, &body),
        SessionAction::Reflect { session_id } => cmd_reflect(&mut db, &session_id),
        SessionAction::Recall { query, k } => cmd_recall(&mut db, &query, k),
        SessionAction::Export { session_id, out } => cmd_export(&mut db, &session_id, out.as_deref()),
        SessionAction::Import { input } => cmd_import(&mut db, input.as_deref()),
        SessionAction::Delete { id } => cmd_delete(&mut db, &id),
        SessionAction::ReflectAll => cmd_reflect_all(&mut db),
    }
}

fn cmd_delete(db: &mut Db, id: &str) -> Result<()> {
    if SessionStore::new(db).get_by_id(id)?.is_none() {
        bail!("session 없음: {id}");
    }
    SessionStore::new(db).delete(id)?;
    println!("✓ session 삭제 (messages·episodes CASCADE, memories.session_id NULL): {id}");
    Ok(())
}

fn cmd_reflect_all(db: &mut Db) -> Result<()> {
    let episodes = reflect_all(db)?;
    println!("✓ reflect-all 완료 — 새 episode {}개", episodes.len());
    for ep in &episodes {
        println!("  [{}] session={} {}", ep.id, ep.session_id, ep.summary);
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

fn cmd_new(db: &mut Db, title: &str) -> Result<()> {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
    let session = SessionStore::new(db).create(title, &host)?;
    println!("✓ session 생성");
    println!("  id            : {}", session.id);
    println!("  title         : {}", session.title);
    println!("  home_machine  : {}", session.home_machine);
    println!("  created_at    : {}", session.created_at);
    Ok(())
}

fn cmd_list(db: &mut Db) -> Result<()> {
    let sessions = SessionStore::new(db).list()?;
    if sessions.is_empty() {
        println!("session 없음. `xgram session new --title <TITLE>` 으로 생성하세요.");
        return Ok(());
    }
    println!("sessions ({})", sessions.len());
    for s in &sessions {
        println!(
            "  {} — {} ({}, last_active {})",
            s.id, s.title, s.home_machine, s.last_active
        );
    }
    Ok(())
}

fn cmd_show(db: &mut Db, id: &str) -> Result<()> {
    let session = SessionStore::new(db)
        .get_by_id(id)?
        .ok_or_else(|| anyhow!("session 없음: {id}"))?;
    let episodes = EpisodeStore::new(db).list_for_session(id)?;
    println!("session {}", session.id);
    println!("  title         : {}", session.title);
    println!("  home_machine  : {}", session.home_machine);
    println!("  created_at    : {}", session.created_at);
    println!("  last_active   : {}", session.last_active);
    println!("  episodes      : {}", episodes.len());
    for ep in &episodes {
        println!(
            "    [{}] {} ({} → {})",
            ep.id, ep.summary, ep.started_at, ep.ended_at
        );
    }
    Ok(())
}

fn cmd_message(
    db: &mut Db,
    data_dir: &Path,
    session_id: &str,
    sender: &str,
    body: &str,
) -> Result<()> {
    if SessionStore::new(db).get_by_id(session_id)?.is_none() {
        bail!("session 없음: {session_id}. `xgram session new` 으로 생성.");
    }

    // 마스터 키로 body 서명 — XGRAM_KEYSTORE_PASSWORD 환경변수 필수.
    let password = require_password()?;
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let kp = ks
        .load(MASTER_KEY_NAME, &password)
        .context("master 키 로드 실패 — keystore 패스워드 확인")?;
    let signature_hex = hex::encode(kp.sign(body.as_bytes()));

    let embedder = DummyEmbedder;
    let msg = MessageStore::new(db, &embedder).insert(session_id, sender, body, &signature_hex)?;
    println!("✓ 메시지 저장 (서명: secp256k1 ECDSA, master)");
    println!("  id        : {}", msg.id);
    println!("  session   : {}", msg.session_id);
    println!("  sender    : {}", msg.sender);
    println!("  signature : {}…{}", &signature_hex[..16], &signature_hex[signature_hex.len() - 16..]);
    println!("  timestamp : {}", msg.timestamp);
    Ok(())
}

fn cmd_recall(db: &mut Db, query: &str, k: usize) -> Result<()> {
    let embedder = DummyEmbedder;
    let hits = MessageStore::new(db, &embedder).recall_top_k(query, k)?;
    if hits.is_empty() {
        println!("일치하는 메시지 없음.");
        return Ok(());
    }
    println!("recall top-{} for {:?}", hits.len(), query);
    for (i, hit) in hits.iter().enumerate() {
        println!(
            "  [{:>2}] dist={:.4} session={} sender={} ts={}",
            i + 1,
            hit.distance,
            hit.message.session_id,
            hit.message.sender,
            hit.message.timestamp,
        );
        println!("       body: {}", hit.message.body);
    }
    Ok(())
}

fn cmd_export(
    db: &mut Db,
    session_id: &str,
    out: Option<&Path>,
) -> Result<()> {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
    let pkg = export_session(db, session_id, &host)?;
    let json = pkg.to_json()?;
    match out {
        Some(p) => {
            std::fs::write(p, &json).with_context(|| format!("저장 실패: {}", p.display()))?;
            println!("✓ session export → {}", p.display());
            println!("  messages : {}", pkg.messages.len());
            println!("  episodes : {}", pkg.episodes.len());
            println!("  memories : {}", pkg.memories.len());
        }
        None => {
            println!("{json}");
        }
    }
    Ok(())
}

fn cmd_import(db: &mut Db, input: Option<&Path>) -> Result<()> {
    use std::io::Read;
    let json = match input {
        Some(p) => std::fs::read_to_string(p)
            .with_context(|| format!("입력 파일 읽기 실패: {}", p.display()))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("stdin 읽기 실패")?;
            buf
        }
    };
    let pkg = TextPackage::from_json(&json)?;
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
    let summary = import_session(db, &pkg, &host)?;
    println!("✓ session import 완료");
    println!("  new session id    : {}", summary.session_id);
    println!("  messages_inserted : {}", summary.messages_inserted);
    println!("  episodes_inserted : {}", summary.episodes_inserted);
    println!("  memories_inserted : {}", summary.memories_inserted);
    Ok(())
}

fn cmd_reflect(db: &mut Db, session_id: &str) -> Result<()> {
    if SessionStore::new(db).get_by_id(session_id)?.is_none() {
        bail!("session 없음: {session_id}");
    }
    match reflect_session(db, session_id)? {
        Some(ep) => {
            println!("✓ reflection 완료");
            println!("  episode id    : {}", ep.id);
            println!("  message_count : {}", ep.message_count);
            println!("  summary       : {}", ep.summary);
        }
        None => println!("session 에 메시지가 없어 episode 미생성."),
    }
    Ok(())
}
