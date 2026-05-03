//! MCP HTTP Bearer 토큰 — agent 인증.
//!
//! 발급:
//!   - 64자 hex 랜덤 토큰 생성 (`rand::rngs::OsRng` 32 bytes → hex)
//!   - SHA-256 해시만 DB 저장 → 유출 시 복구 불가
//!   - 평문 토큰은 발급 직후 1회 마스터에게 표시
//!
//! 검증:
//!   - HTTP 요청 Authorization 헤더 → "Bearer <token>" 파싱 → SHA-256 → DB lookup
//!   - 매칭되는 row 의 agent 반환 + last_used 갱신
//!   - 매칭 없음 → None (호출자가 401 처리 또는 master 폴백)

use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct McpTokenEntry {
    pub id: String,
    pub agent: String,
    pub label: Option<String>,
    pub created_at: String,
    pub last_used: Option<String>,
}

pub fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

/// 새 64자 hex 토큰 생성 (32 bytes OS RNG via getrandom).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS RNG (getrandom) 실패");
    hex::encode(bytes)
}

/// (id, plain_token) 반환. plain_token 은 발급 직후 1회만 노출.
pub fn create_token(db: &mut Db, agent: &str, label: Option<&str>) -> Result<(String, String)> {
    let id = Uuid::new_v4().to_string();
    let token = generate_token();
    let token_hash = hash_token(&token);
    let now = kst_now().to_rfc3339();
    let affected = db.conn().execute(
        "INSERT INTO mcp_tokens (id, token_hash, agent, label, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, token_hash, agent, label, now],
    )?;
    if affected != 1 {
        bail!("토큰 저장 실패 (affected={affected})");
    }
    Ok((id, token))
}

pub fn list_tokens(db: &mut Db) -> Result<Vec<McpTokenEntry>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, agent, label, created_at, last_used
         FROM mcp_tokens ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(McpTokenEntry {
            id: r.get(0)?,
            agent: r.get(1)?,
            label: r.get(2)?,
            created_at: r.get(3)?,
            last_used: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn revoke_token(db: &mut Db, id: &str) -> Result<()> {
    let affected = db
        .conn()
        .execute("DELETE FROM mcp_tokens WHERE id = ?1", [id])?;
    if affected != 1 {
        bail!("토큰 미존재: {id}");
    }
    Ok(())
}

/// Bearer 토큰 검증. 매칭 시 agent 반환 + last_used 갱신.
/// 매칭 없으면 Ok(None) — 호출자가 거부/폴백 결정.
pub fn verify_token(db: &mut Db, token: &str) -> Result<Option<String>> {
    let token_hash = hash_token(token);
    let res = db.conn().query_row(
        "SELECT id, agent FROM mcp_tokens WHERE token_hash = ?1",
        [&token_hash],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    );
    match res {
        Ok((id, agent)) => {
            let now = kst_now().to_rfc3339();
            db.conn().execute(
                "UPDATE mcp_tokens SET last_used = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )?;
            Ok(Some(agent))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn open_db(data_dir: &Path) -> Result<Db> {
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
