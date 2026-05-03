//! xgram vault — 암호화 자격증명 CLI (set / get / list / delete).
//!
//! Phase 1: keystore 패스워드(XGRAM_KEYSTORE_PASSWORD) 로 ChaCha20 암호화.
//! ACL · daily 한도 · MFA 정책은 후속 PR.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::env::require_password;
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_vault::VaultStore;

#[derive(Debug, Clone)]
pub enum VaultAction {
    Set {
        key: String,
        value: String,
        tags: Vec<String>,
    },
    Get {
        key: String,
    },
    List,
    Delete {
        key: String,
    },
}

pub fn run_vault(data_dir: &Path, action: VaultAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let password = require_password()?;
    let mut store = VaultStore::new(&mut db);

    match action {
        VaultAction::Set { key, value, tags } => {
            let entry = store.set(&key, value.as_bytes(), &password, &tags)?;
            println!("✓ vault entry 저장");
            println!("  id        : {}", entry.id);
            println!("  key       : {}", entry.key);
            println!("  tags      : {:?}", entry.tags);
            println!("  created_at: {}", entry.created_at);
        }
        VaultAction::Get { key } => {
            let bytes = store.get(&key, &password)?;
            // bytes 가 UTF-8 이면 출력, 아니면 hex
            match std::str::from_utf8(&bytes) {
                Ok(s) => println!("{s}"),
                Err(_) => println!("{}", hex::encode(bytes)),
            }
        }
        VaultAction::List => {
            let entries = store.list()?;
            if entries.is_empty() {
                println!("vault 비어있음.");
                return Ok(());
            }
            println!("vault entries ({})", entries.len());
            for e in &entries {
                println!(
                    "  {} — {} (tags={:?}, last={})",
                    e.id, e.key, e.tags, e.last_accessed
                );
            }
        }
        VaultAction::Delete { key } => {
            store.delete(&key)?;
            println!("✓ vault entry 삭제: {key}");
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
