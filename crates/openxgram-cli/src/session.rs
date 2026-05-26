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
    backfill_memory_embeddings, backfill_message_embeddings, default_embedder, export_session,
    import_session, reflect_all, reflect_session, EpisodeStore, MessageStore, SessionStore,
    TextPackage,
};

#[derive(Debug, Clone)]
pub enum SessionAction {
    New {
        title: String,
    },
    List,
    Show {
        id: String,
    },
    Message {
        session_id: String,
        sender: String,
        body: String,
    },
    Reflect {
        session_id: String,
    },
    Recall {
        query: String,
        k: usize,
    },
    Export {
        session_id: String,
        out: Option<std::path::PathBuf>,
    },
    Import {
        input: Option<std::path::PathBuf>,
        verify: bool,
    },
    Delete {
        id: String,
    },
    ReflectAll,
    /// step 5/6 — 다른 LLM 앱에 복붙 가능한 평문 transcript 출력.
    /// `--format` claude / chatgpt / gemini / plain (기본 plain).
    Transcript {
        session_id: String,
        format: String,
    },
    /// 임베딩이 없는 기존 메시지를 일괄 임베딩 (FastEmbedder 사용).
    /// `force=true` 이면 기존 임베딩 맵 전체 삭제 후 재임베딩 (prefix 변경 후 필요).
    BackfillEmbeddings { force: bool },
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
        SessionAction::Export { session_id, out } => {
            cmd_export(&mut db, data_dir, &session_id, out.as_deref())
        }
        SessionAction::Import { input, verify } => cmd_import(&mut db, input.as_deref(), verify),
        SessionAction::Delete { id } => cmd_delete(&mut db, &id),
        SessionAction::ReflectAll => cmd_reflect_all(&mut db),
        SessionAction::Transcript { session_id, format } => {
            cmd_transcript(&mut db, &session_id, &format)
        }
        SessionAction::BackfillEmbeddings { force } => cmd_backfill_embeddings(&mut db, force),
    }
}

/// step 5/6 — session 의 모든 메시지를 평문 transcript 로 출력.
/// `--format` 별로 헤더/footer 가 다름 — claude/chatgpt/gemini 가 paste 시 친화적.
fn cmd_transcript(db: &mut Db, session_id: &str, format: &str) -> Result<()> {
    use openxgram_memory::{default_embedder, MessageStore};

    let session = SessionStore::new(db)
        .get_by_id(session_id)?
        .ok_or_else(|| anyhow!("session 없음: {session_id}"))?;
    let embedder = default_embedder()?;
    let messages = MessageStore::new(db, embedder.as_ref()).list_for_session(session_id)?;

    let header = match format {
        "claude" => "다음은 Claude 와 진행 중이던 대화입니다. 이어서 답해주세요.\n\n",
        "chatgpt" => "Below is an in-progress conversation. Please continue.\n\n",
        "gemini" => "다음 대화 컨텍스트를 이어가 주세요.\n\n",
        "plain" | _ => "",
    };
    print!("{header}");
    for m in &messages {
        let ts = m.timestamp.format("%Y-%m-%d %H:%M");
        println!("[{ts}] {}: {}", m.sender, m.body);
    }
    if !matches!(format, "plain" | "") {
        println!();
        println!("(끝 — OpenXgram session_id={session_id})");
    }
    Ok(())
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

    let embedder = default_embedder()?;
    let msg = MessageStore::new(db, embedder.as_ref()).insert(
        session_id,
        sender,
        body,
        &signature_hex,
        None,
    )?;
    println!("✓ 메시지 저장 (서명: secp256k1 ECDSA, master)");
    println!("  id              : {}", msg.id);
    println!("  session         : {}", msg.session_id);
    println!("  conversation_id : {}", msg.conversation_id);
    println!("  sender          : {}", msg.sender);
    println!(
        "  signature : {}…{}",
        &signature_hex[..16],
        &signature_hex[signature_hex.len() - 16..]
    );
    println!("  timestamp : {}", msg.timestamp);
    Ok(())
}

fn cmd_recall(db: &mut Db, query: &str, k: usize) -> Result<()> {
    let embedder = default_embedder()?;
    let hits = MessageStore::new(db, embedder.as_ref()).recall_top_k(query, k)?;
    if hits.is_empty() {
        println!("일치하는 메시지 없음.");
        return Ok(());
    }
    println!("recall top-{} for {:?}", hits.len(), query);
    for (i, hit) in hits.iter().enumerate() {
        println!(
            "  [{:>2}] dist={:.4} source={} session={} sender={} ts={}",
            i + 1,
            hit.distance,
            hit.source,
            hit.message.session_id,
            hit.message.sender,
            hit.message.timestamp,
        );
        println!("       body: {}", hit.message.body);
    }
    Ok(())
}

fn cmd_export(db: &mut Db, data_dir: &Path, session_id: &str, out: Option<&Path>) -> Result<()> {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
    // 환경변수 패스워드가 있으면 master public key 동봉 — 수신측이 검증 가능.
    let master_pk_hex = std::env::var(openxgram_core::env::PASSWORD_ENV)
        .ok()
        .and_then(|pw| {
            let ks = FsKeystore::new(keystore_dir(data_dir));
            ks.load(MASTER_KEY_NAME, &pw)
                .ok()
                .map(|kp| hex::encode(kp.public_key_bytes()))
        });
    let pkg = export_session(db, session_id, &host, master_pk_hex)?;
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

fn cmd_import(db: &mut Db, input: Option<&Path>, verify: bool) -> Result<()> {
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

    if verify {
        verify_package_signatures(&pkg)?;
        println!("✓ {} 메시지 signature 모두 검증 통과", pkg.messages.len());
    }

    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
    let summary = import_session(db, &pkg, &host)?;
    println!("✓ session import 완료");
    println!("  new session id    : {}", summary.session_id);
    println!("  messages_inserted : {}", summary.messages_inserted);
    println!("  episodes_inserted : {}", summary.episodes_inserted);
    println!("  memories_inserted : {}", summary.memories_inserted);
    Ok(())
}

fn verify_package_signatures(pkg: &TextPackage) -> Result<()> {
    use k256::ecdsa::{signature::Verifier, Signature, VerifyingKey};

    let pk_hex = pkg
        .master_public_key
        .as_deref()
        .ok_or_else(|| anyhow!("패키지에 master_public_key 없음 — 검증 불가"))?;
    let pk_bytes = hex::decode(pk_hex).context("master_public_key hex decode 실패")?;
    let vk = VerifyingKey::from_sec1_bytes(&pk_bytes)
        .map_err(|_| anyhow!("master_public_key 형식 오류 (33B SEC1 압축 기대)"))?;

    for msg in &pkg.messages {
        let sig_bytes = hex::decode(&msg.signature)
            .with_context(|| format!("msg {} signature hex decode 실패", msg.id))?;
        let sig = Signature::from_slice(&sig_bytes)
            .map_err(|_| anyhow!("msg {} signature 형식 오류", msg.id))?;
        vk.verify(msg.body.as_bytes(), &sig)
            .map_err(|_| anyhow!("msg {} signature 불일치", msg.id))?;
    }
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

fn cmd_backfill_embeddings(db: &mut Db, force: bool) -> Result<()> {
    let embedder = default_embedder()?;
    println!("임베딩 백필 시작 (embedder: FastEmbedder multilingual-e5-small, force={force})...");

    if force {
        // 기존 임베딩 맵 + 벡터 전체 삭제 후 재임베딩
        println!("  [force] 기존 message 임베딩 삭제 중...");
        let conn = db.conn();
        // message_embedding_map 삭제 → message_embeddings 는 orphan 이지만 vec0 는 직접 DELETE 지원
        let msg_map_deleted = conn.execute("DELETE FROM message_embedding_map", [])?;
        // vec0 가상 테이블 전체 삭제 — rowid 없이 DELETE * 사용
        conn.execute("DELETE FROM message_embeddings WHERE rowid > 0", [])?;
        println!("  [force] message_embedding_map {msg_map_deleted}건 삭제.");

        println!("  [force] 기존 memory 임베딩 삭제 중...");
        let mem_map_deleted = conn.execute("DELETE FROM memory_embedding_map", []).unwrap_or(0);
        conn.execute("DELETE FROM memory_embeddings WHERE rowid > 0", []).unwrap_or(0);
        println!("  [force] memory_embedding_map {mem_map_deleted}건 삭제.");
    }

    // messages 백필
    let (msg_done, msg_total) = backfill_message_embeddings(db, embedder.as_ref())?;
    if msg_total == 0 {
        println!("✓ 메시지 백필 불필요 — 미임베딩 메시지 없음.");
    } else {
        println!("✓ 메시지 백필 완료 — {msg_done}/{msg_total} 건 임베딩 생성.");
    }

    // memories 백필
    let (mem_done, mem_total) = backfill_memory_embeddings(db, embedder.as_ref())?;
    if mem_total == 0 {
        println!("✓ 메모리 백필 불필요 — 미임베딩 L2 memory 없음.");
    } else {
        println!("✓ 메모리 백필 완료 — {mem_done}/{mem_total} 건 임베딩 생성.");
    }

    Ok(())
}
