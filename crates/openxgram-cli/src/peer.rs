//! xgram peer — peer registry CLI (add/list/show/touch/delete).
//!
//! Phase 2 baseline: CRUD. push/pull 흐름은 transport 통합 PR.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use k256::ecdsa::VerifyingKey;
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::AgentAddress;
use openxgram_peer::{PeerRole, PeerStore};

/// 압축 secp256k1 공개키 hex (66자) → EIP-55 EVM 주소.
/// `peer add` 시 항상 eth_address 를 채워야 inbound envelope.from 매칭이 작동한다.
fn eth_address_from_pubkey_hex(public_key_hex: &str) -> Result<String> {
    let bytes = hex::decode(public_key_hex)
        .map_err(|e| anyhow!("public_key hex decode 실패: {e}"))?;
    let vk = VerifyingKey::from_sec1_bytes(&bytes)
        .map_err(|e| anyhow!("public_key sec1 파싱 실패: {e}"))?;
    Ok(AgentAddress::from_verifying_key(&vk).0)
}

#[derive(Debug, Clone)]
pub enum PeerAction {
    Add {
        alias: String,
        public_key_hex: String,
        address: String,
        role: PeerRole,
        notes: Option<String>,
    },
    List,
    Show {
        alias: String,
    },
    Touch {
        alias: String,
    },
    Delete {
        alias: String,
    },
}

pub fn run_peer(data_dir: &Path, action: PeerAction) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let mut store = PeerStore::new(&mut db);

    match action {
        PeerAction::Add {
            alias,
            public_key_hex,
            address,
            role,
            notes,
        } => {
            let eth_addr = eth_address_from_pubkey_hex(&public_key_hex)?;
            let p = store.add_with_eth(
                &alias,
                &public_key_hex,
                &address,
                Some(&eth_addr),
                role,
                notes.as_deref(),
            )?;
            println!("✓ peer 등록");
            println!("  id          : {}", p.id);
            println!("  alias       : {}", p.alias);
            println!("  role        : {}", p.role.as_str());
            println!("  address     : {}", p.address);
            println!("  eth_address : {eth_addr}");
            println!(
                "  public_key  : {}…{}",
                &p.public_key_hex[..8],
                &p.public_key_hex[p.public_key_hex.len() - 8..]
            );
        }
        PeerAction::List => {
            let peers = store.list()?;
            if peers.is_empty() {
                println!("등록된 peer 없음.");
                return Ok(());
            }
            println!("peers ({})", peers.len());
            for p in &peers {
                let last = p
                    .last_seen
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "(미연결)".into());
                println!(
                    "  {} — {} [{}] last_seen={}",
                    p.alias,
                    p.address,
                    p.role.as_str(),
                    last
                );
            }
        }
        PeerAction::Show { alias } => {
            let p = store
                .get_by_alias(&alias)?
                .ok_or_else(|| anyhow::anyhow!("peer 없음: {alias}"))?;
            println!("peer: {}", p.alias);
            println!("  id        : {}", p.id);
            println!("  role      : {}", p.role.as_str());
            println!("  address   : {}", p.address);
            println!("  public_key: {}", p.public_key_hex);
            if let Some(n) = &p.notes {
                println!("  notes     : {n}");
            }
            println!("  created_at: {}", p.created_at);
            if let Some(ls) = p.last_seen {
                println!("  last_seen : {ls}");
            } else {
                println!("  last_seen : (미연결)");
            }
        }
        PeerAction::Touch { alias } => {
            store.touch(&alias)?;
            println!("✓ peer last_seen 갱신: {alias}");
        }
        PeerAction::Delete { alias } => {
            store.delete(&alias)?;
            println!("✓ peer 삭제: {alias}");
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
