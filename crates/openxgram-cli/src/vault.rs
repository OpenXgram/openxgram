//! xgram vault — 암호화 자격증명 CLI (set / get / list / delete).
//!
//! Phase 1: keystore 패스워드(XGRAM_KEYSTORE_PASSWORD) 로 ChaCha20 암호화.
//! ACL · daily 한도 · MFA 정책은 후속 PR.

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::env::require_password;
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_vault::{AclAction, AclPolicy, VaultStore};

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
    AclSet {
        key_pattern: String,
        agent: String,
        actions: Vec<AclAction>,
        daily_limit: i64,
        policy: AclPolicy,
    },
    AclList,
    AclDelete {
        key_pattern: String,
        agent: String,
    },
    Pending,
    Approve {
        id: String,
    },
    Deny {
        id: String,
    },
    MfaIssue {
        agent: String,
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
        VaultAction::AclSet {
            key_pattern,
            agent,
            actions,
            daily_limit,
            policy,
        } => {
            let acl = store.upsert_acl(&key_pattern, &agent, &actions, daily_limit, policy)?;
            println!("✓ vault ACL 저장");
            println!("  id           : {}", acl.id);
            println!("  key_pattern  : {}", acl.key_pattern);
            println!("  agent        : {}", acl.agent);
            println!(
                "  actions      : {}",
                acl.allowed_actions
                    .iter()
                    .map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            println!("  daily_limit  : {}", acl.daily_limit);
            println!("  policy       : {}", acl.policy.as_str());
        }
        VaultAction::AclList => {
            let entries = store.list_acl()?;
            if entries.is_empty() {
                println!("vault ACL 비어있음.");
                return Ok(());
            }
            println!("vault ACL ({})", entries.len());
            for e in &entries {
                let actions: Vec<&str> = e.allowed_actions.iter().map(|a| a.as_str()).collect();
                println!(
                    "  {}/{} → [{}] (limit={}, policy={})",
                    e.key_pattern,
                    e.agent,
                    actions.join(","),
                    e.daily_limit,
                    e.policy.as_str()
                );
            }
        }
        VaultAction::AclDelete { key_pattern, agent } => {
            store.delete_acl(&key_pattern, &agent)?;
            println!("✓ vault ACL 삭제: {key_pattern}/{agent}");
        }
        VaultAction::Pending => {
            let pending = store.list_pending()?;
            if pending.is_empty() {
                println!("vault pending 비어있음.");
                return Ok(());
            }
            println!("vault pending ({})", pending.len());
            for p in &pending {
                println!(
                    "  {} — {} {}/{} (요청 {})",
                    p.id,
                    p.action.as_str(),
                    p.key,
                    p.agent,
                    p.requested_at
                );
                println!("     `xgram vault approve {}` 또는 `xgram vault deny {}`", p.id, p.id);
            }
        }
        VaultAction::Approve { id } => {
            store.approve_confirmation(&id)?;
            println!("✓ vault confirm 승인: {id} (1회 소비, agent 재호출 시 통과)");
        }
        VaultAction::Deny { id } => {
            store.deny_confirmation(&id)?;
            println!("✓ vault confirm 거부: {id}");
        }
        VaultAction::MfaIssue { agent } => {
            let secret = store.issue_mfa_secret(&agent)?;
            println!("✓ TOTP secret 발급 (agent={agent})");
            println!();
            println!("authenticator 앱(Google Authenticator / 1Password / Authy) 에 등록:");
            println!("  secret (base32): {secret}");
            println!("  algorithm      : SHA1");
            println!("  digits         : 6");
            println!("  period         : 30s");
            println!("  issuer         : OpenXgram");
            println!("  account        : {agent}");
            println!();
            println!("이후 agent 가 vault_get/_set/_delete 호출 시 현재 TOTP 코드 동봉 필수.");
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
