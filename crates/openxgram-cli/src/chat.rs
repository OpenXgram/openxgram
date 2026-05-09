//! `xgram chat` — 30초 녹화용 REPL 데모 진입점.
//!
//! 설계 원칙 (마스터 룰):
//! - prompt 0: data_dir / 패스워드 / alias 다 자동
//! - 외부 의존 0: Anthropic 키 없으면 echo, Tailscale 없으면 localhost
//! - 한 명령으로 즉시 대화 가능
//!
//! REPL 내부 명령:
//!   `> 안녕`              → 봇 응답
//!   `> recall <query>`    → 메모리 회상
//!   `> sessions`          → 세션 목록
//!   `> exit` / Ctrl-D     → 종료
//!
//! 첫 가동 시 자동 init (data_dir + master keypair). 패스워드는 `~/.openxgram/.chat-password` (chmod 0600)
//! 에 저장 — 데모 단순함 우선. master 가 진지한 사용으로 전환 시 패스워드 재설정 권장.

use anyhow::{Context, Result};
use openxgram_core::paths::{db_path, default_data_dir, keystore_dir, MASTER_KEY_NAME};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::response::Generator;

const CHAT_SESSION_TITLE: &str = "chat";
const PASSWORD_FILE: &str = ".chat-password";

pub async fn run_chat(data_dir: Option<PathBuf>) -> Result<()> {
    let dir = match data_dir {
        Some(p) => p,
        None => default_data_dir().context("default_data_dir")?,
    };
    std::fs::create_dir_all(&dir).ok();

    // 1) 미초기화면 자동 init (alias=me, 패스워드 자동 생성/저장)
    let manifest = openxgram_core::paths::manifest_path(&dir);
    if !manifest.exists() {
        ensure_init(&dir)?;
    }

    // 2) DB open + chat session ensure
    let mut db = Db::open(DbConfig {
        path: db_path(&dir),
        ..Default::default()
    })
    .context("DB open")?;
    db.migrate().context("DB migrate")?;
    let session = SessionStore::new(&mut db)
        .ensure_by_title(CHAT_SESSION_TITLE, "chat")
        .context("chat session ensure")?;
    let session_id = session.id.clone();

    // 3) Generator (Anthropic 키 있으면 LLM, 없으면 echo)
    let generator = Generator::from_env();
    let alias = std::env::var("XGRAM_AGENT_ALIAS").unwrap_or_else(|_| "Starian".into());

    println!();
    println!("xgram chat — '{}' 와 대화 (generator: {})", alias, generator.label());
    println!("내부 명령: recall <query> / sessions / exit");
    println!();

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("http client")?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let embedder = default_embedder().context("embedder")?;

    loop {
        print!("> ");
        stdout.flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input == "exit" || input == "quit" || input == ":q" {
            break;
        }

        // 내부 명령
        if let Some(q) = input.strip_prefix("recall ") {
            recall_print(&mut db, embedder.as_ref(), q.trim())?;
            continue;
        }
        if input == "sessions" {
            for s in SessionStore::new(&mut db).list()? {
                println!("  - {} ({})", s.title, s.id);
            }
            continue;
        }

        // user 메시지 저장
        let user_msg = MessageStore::new(&mut db, embedder.as_ref())
            .insert(&session_id, "me", input, "chat", None)
            .context("user message insert")?;

        // 같은 conversation 의 history (LLM 컨텍스트)
        let history = MessageStore::new(&mut db, embedder.as_ref())
            .list_for_conversation(&user_msg.conversation_id)
            .unwrap_or_default();

        // 응답 생성
        let out = generator.generate(&http, &alias, input, &history).await?;
        println!("< {}", out.body);

        // assistant 메시지 저장 (같은 conversation)
        MessageStore::new(&mut db, embedder.as_ref())
            .insert(
                &session_id,
                &alias,
                &out.body,
                out.signature,
                Some(&user_msg.conversation_id),
            )
            .context("assistant message insert")?;
    }

    println!();
    println!("(끝 — 메모리 보존됨: {})", db_path(&dir).display());
    Ok(())
}

fn recall_print(
    db: &mut Db,
    embedder: &dyn openxgram_memory::Embedder,
    query: &str,
) -> Result<()> {
    let hits = MessageStore::new(db, embedder).recall_top_k(query, 5)?;
    if hits.is_empty() {
        println!("(회상 결과 없음)");
        return Ok(());
    }
    for (i, h) in hits.iter().enumerate() {
        let preview = h.message.body.lines().next().unwrap_or("").chars().take(80).collect::<String>();
        println!(
            "  [{}] dist={:.3} {} → {}",
            i + 1,
            h.distance,
            h.message.sender,
            preview
        );
    }
    Ok(())
}

/// 첫 가동 시 자동 init — alias=me, 패스워드 자동 생성 후 0600 파일에 저장.
/// master 가 진지한 사용으로 전환 시 `xgram init --force` 로 재설정 권장.
fn ensure_init(data_dir: &Path) -> Result<()> {
    use crate::init::{run_init, InitOpts};
    use openxgram_manifest::MachineRole;

    // 이미 init 된 경우 — 패스워드만 환경에 set 하고 return.
    let manifest = openxgram_core::paths::manifest_path(data_dir);
    let pw_file = data_dir.join(PASSWORD_FILE);
    if manifest.exists() && pw_file.exists() {
        let p = std::fs::read_to_string(&pw_file)?.trim().to_string();
        unsafe {
            std::env::set_var("XGRAM_KEYSTORE_PASSWORD", &p);
        }
        return Ok(());
    }
    if manifest.exists() && !pw_file.exists() {
        // 누군가 다른 방법으로 init 함 — 사용자 패스워드 알아야. 안내.
        anyhow::bail!(
            "init 은 됐는데 chat 패스워드 파일 없음 ({}). XGRAM_KEYSTORE_PASSWORD 직접 export 후 재시도",
            pw_file.display()
        );
    }

    let password = {
        let p = generate_random_password();
        std::fs::create_dir_all(data_dir).ok();
        std::fs::write(&pw_file, &p)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&pw_file)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&pw_file, perms)?;
        }
        p
    };
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", &password);
        std::env::set_var("XGRAM_SKIP_PORT_PRECHECK", "1");
    }
    eprintln!("[chat] 첫 가동 — 자동 init (alias=me, password={})", pw_file.display());
    run_init(&InitOpts {
        alias: "me".into(),
        role: MachineRole::Primary,
        data_dir: data_dir.to_path_buf(),
        force: false,
        dry_run: false,
        import: false,
    })
    .context("자동 init")?;
    eprintln!("[chat] init 완료. 패스워드 파일은 0600 — 진지한 사용 시 변경 권장.");
    Ok(())
}

fn generate_random_password() -> String {
    use getrandom::fill;
    let mut bytes = [0u8; 16];
    fill(&mut bytes).expect("getrandom");
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_init_writes_manifest_and_password_file() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        unsafe {
            std::env::remove_var("XGRAM_SEED");
            std::env::remove_var("XGRAM_KEYSTORE_PASSWORD");
        }
        ensure_init(dir).unwrap();
        assert!(openxgram_core::paths::manifest_path(dir).exists());
        assert!(dir.join(PASSWORD_FILE).exists());
        // 두 번째 호출 — 기존 password 재사용
        ensure_init(dir).unwrap();
    }

    #[test]
    fn random_password_is_hex_32() {
        let p = generate_random_password();
        assert_eq!(p.len(), 32);
        assert!(p.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
