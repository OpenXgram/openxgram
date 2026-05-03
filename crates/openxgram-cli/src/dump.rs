//! xgram dump <kind> — JSON 출력 통합. 사람용 명령은 색상·텍스트, 도구 통합은
//! 단일 dump 진입점으로 단순화. Tauri/스크립트/Prometheus exporter 모두 친화.
//!
//! 지원 kind:
//!   sessions / messages / episodes / memories / patterns / traits
//!   vault    / acl      / pending  / peers    / payments / mcp-tokens

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{MemoryStore, PatternStore, SessionStore, TraitStore};
use openxgram_payment::PaymentStore;
use openxgram_peer::PeerStore;
use openxgram_vault::VaultStore;
use serde_json::{json, Value};

use crate::mcp_tokens;

pub fn run_dump(data_dir: &Path, kind: &str) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let value = match kind {
        "sessions" => sessions(&mut db)?,
        "episodes" => episodes(&mut db)?,
        "memories" => memories(&mut db)?,
        "patterns" => patterns(&mut db)?,
        "traits" => traits(&mut db)?,
        "vault" => vault_entries(&mut db)?,
        "acl" => vault_acl(&mut db)?,
        "pending" => vault_pending(&mut db)?,
        "peers" => peers(&mut db)?,
        "payments" => payments(&mut db)?,
        "mcp-tokens" => mcp_tokens_dump(&mut db)?,
        other => bail!(
            "지원하지 않는 kind: {other}. 가능한 값: sessions/episodes/memories/patterns/traits/vault/acl/pending/peers/payments/mcp-tokens"
        ),
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn sessions(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = SessionStore::new(db)
        .list()?
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "title": s.title,
                "home_machine": s.home_machine,
                "created_at": s.created_at.to_rfc3339(),
                "last_active": s.last_active.to_rfc3339(),
            })
        })
        .collect();
    Ok(json!({"kind": "sessions", "count": list.len(), "items": list}))
}

fn episodes(db: &mut Db) -> Result<Value> {
    // 글로벌 episode 카운트만 (session 별 list 는 session show 가 담당)
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))?;
    Ok(json!({"kind": "episodes", "count": count}))
}

fn memories(db: &mut Db) -> Result<Value> {
    use openxgram_memory::MemoryKind;
    let mut all = Vec::new();
    for kind in [
        MemoryKind::Fact,
        MemoryKind::Decision,
        MemoryKind::Reference,
        MemoryKind::Rule,
    ] {
        for m in MemoryStore::new(db).list_by_kind(kind)? {
            all.push(json!({
                "id": m.id,
                "kind": m.kind.as_str(),
                "content": m.content,
                "pinned": m.pinned,
                "importance": m.importance,
                "access_count": m.access_count,
            }));
        }
    }
    Ok(json!({"kind": "memories", "count": all.len(), "items": all}))
}

fn patterns(db: &mut Db) -> Result<Value> {
    use openxgram_memory::Classification;
    let mut all = Vec::new();
    for c in [
        Classification::New,
        Classification::Recurring,
        Classification::Routine,
    ] {
        for p in PatternStore::new(db).list_by_classification(c)? {
            all.push(json!({
                "id": p.id,
                "text": p.pattern_text,
                "frequency": p.frequency,
                "classification": p.classification.as_str(),
                "first_seen": p.first_seen.to_rfc3339(),
                "last_seen": p.last_seen.to_rfc3339(),
            }));
        }
    }
    Ok(json!({"kind": "patterns", "count": all.len(), "items": all}))
}

fn traits(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = TraitStore::new(db)
        .list()?
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "value": t.value,
                "source": t.source.as_str(),
                "refs": t.source_refs,
                "created_at": t.created_at.to_rfc3339(),
                "updated_at": t.updated_at.to_rfc3339(),
            })
        })
        .collect();
    Ok(json!({"kind": "traits", "count": list.len(), "items": list}))
}

fn vault_entries(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = VaultStore::new(db)
        .list()?
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "key": e.key,
                "tags": e.tags,
                "created_at": e.created_at.to_rfc3339(),
                "last_accessed": e.last_accessed.to_rfc3339(),
            })
        })
        .collect();
    Ok(json!({"kind": "vault", "count": list.len(), "items": list}))
}

fn vault_acl(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = VaultStore::new(db)
        .list_acl()?
        .iter()
        .map(|a| {
            let actions: Vec<&str> = a.allowed_actions.iter().map(|x| x.as_str()).collect();
            json!({
                "id": a.id,
                "key_pattern": a.key_pattern,
                "agent": a.agent,
                "allowed_actions": actions,
                "daily_limit": a.daily_limit,
                "policy": a.policy.as_str(),
                "created_at": a.created_at.to_rfc3339(),
            })
        })
        .collect();
    Ok(json!({"kind": "acl", "count": list.len(), "items": list}))
}

fn vault_pending(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = VaultStore::new(db)
        .list_pending()?
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "key": p.key,
                "agent": p.agent,
                "action": p.action.as_str(),
                "status": p.status.as_str(),
                "requested_at": p.requested_at.to_rfc3339(),
                "decided_at": p.decided_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    Ok(json!({"kind": "pending", "count": list.len(), "items": list}))
}

fn peers(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = PeerStore::new(db)
        .list()?
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "alias": p.alias,
                "public_key_hex": p.public_key_hex,
                "address": p.address,
                "role": p.role.as_str(),
                "last_seen": p.last_seen.map(|t| t.to_rfc3339()),
                "created_at": p.created_at.to_rfc3339(),
                "notes": p.notes,
            })
        })
        .collect();
    Ok(json!({"kind": "peers", "count": list.len(), "items": list}))
}

fn payments(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = PaymentStore::new(db)
        .list()?
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "amount_usdc_micro": p.amount_usdc_micro,
                "amount_display": p.amount_display(),
                "chain": p.chain,
                "payee_address": p.payee_address,
                "memo": p.memo,
                "nonce": p.nonce,
                "state": p.state.as_str(),
                "submitted_tx_hash": p.submitted_tx_hash,
                "created_at": p.created_at.to_rfc3339(),
            })
        })
        .collect();
    Ok(json!({"kind": "payments", "count": list.len(), "items": list}))
}

fn mcp_tokens_dump(db: &mut Db) -> Result<Value> {
    let list: Vec<Value> = mcp_tokens::list_tokens(db)?
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "agent": t.agent,
                "label": t.label,
                "created_at": t.created_at,
                "last_used": t.last_used,
            })
        })
        .collect();
    Ok(json!({"kind": "mcp-tokens", "count": list.len(), "items": list}))
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
