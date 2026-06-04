//! openxgram-peer — peer registry baseline.
//!
//! transport 종류 (HTTP/Tailscale/XMTP) 와 무관하게 peer 메타데이터 통합 관리.
//! cross-machine 메시지 push/pull 의 어드레스북 + ECDSA 서명 검증의 기준.
//!
//! Phase 2 baseline: CRUD + last_seen 갱신. 실제 push/pull 흐름은 transport 통합 PR.

use chrono::{DateTime, FixedOffset};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PeerError {
    #[error("db error: {0}")]
    Db(#[from] DbError),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("peer not found: {0}")]
    NotFound(String),

    #[error("invalid timestamp: {0}")]
    InvalidTimestamp(String),

    #[error("invalid role: {0}")]
    InvalidRole(String),
}

pub type Result<T> = std::result::Result<T, PeerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerRole {
    Primary,
    Secondary,
    Worker,
}

impl PeerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Worker => "worker",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "primary" => Self::Primary,
            "secondary" => Self::Secondary,
            "worker" => Self::Worker,
            other => return Err(PeerError::InvalidRole(other.into())),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Peer {
    pub id: String,
    pub alias: String,
    /// secp256k1 압축 공개키 hex (66자, 0x02/0x03 prefix + 32 byte X)
    pub public_key_hex: String,
    /// transport 주소 (http://host:port, xmtp://addr 등)
    pub address: String,
    /// EIP-55 ETH 주소 (envelope.from 매칭용). public_key 로부터 derive 또는 별도 등록.
    pub eth_address: Option<String>,
    pub role: PeerRole,
    pub last_seen: Option<DateTime<FixedOffset>>,
    pub created_at: DateTime<FixedOffset>,
    pub notes: Option<String>,
}

pub struct PeerStore<'a> {
    db: &'a mut Db,
}

impl<'a> PeerStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// 새 peer 등록. alias / public_key 가 unique. eth_address 는 선택 (envelope.from 매칭용).
    pub fn add(
        &mut self,
        alias: &str,
        public_key_hex: &str,
        address: &str,
        role: PeerRole,
        notes: Option<&str>,
    ) -> Result<Peer> {
        self.add_with_eth(alias, public_key_hex, address, None, role, notes)
    }

    pub fn add_with_eth(
        &mut self,
        alias: &str,
        public_key_hex: &str,
        address: &str,
        eth_address: Option<&str>,
        role: PeerRole,
        notes: Option<&str>,
    ) -> Result<Peer> {
        let id = Uuid::new_v4().to_string();
        let now_rfc = kst_now().to_rfc3339();
        self.db.conn().execute(
            "INSERT INTO peers (id, alias, public_key_hex, address, role, created_at, notes, eth_address)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                alias,
                public_key_hex,
                address,
                role.as_str(),
                now_rfc,
                notes,
                eth_address
            ],
        )?;
        self.get_by_alias(alias)?
            .ok_or_else(|| PeerError::NotFound(format!("just-inserted: {alias}")))
    }

    /// rc.244 zero-touch — envelope sender hint 로 peer 자동 등록/갱신 (idempotent).
    ///   매번 수동으로 gui_address·중복·주소를 손보지 않도록, 신원(eth→pubkey) 기준으로:
    ///   - 이미 있으면 address/gui_address/last_seen UPDATE (중복 alias 생성 방지, role·alias 보존)
    ///   - 없으면 gui_address·role 포함해 신규 insert
    ///   gui_address 는 cross-machine 터미널 proxy 가 바로 쓰므로 자동 등록의 핵심.
    pub fn upsert_announce(
        &mut self,
        alias: &str,
        public_key_hex: &str,
        address: &str,
        gui_address: Option<&str>,
        eth_address: &str,
        default_role: PeerRole,
    ) -> Result<Peer> {
        let now_rfc = kst_now().to_rfc3339();
        // 1) 같은 eth 신원 존재 → UPDATE (alias·role 보존)
        if let Some(existing) = self.get_by_eth_address(eth_address)? {
            self.db.conn().execute(
                "UPDATE peers SET address = ?1, gui_address = COALESCE(?2, gui_address), last_seen = ?3 WHERE eth_address = ?4",
                rusqlite::params![address, gui_address, now_rfc, eth_address],
            )?;
            return self
                .get_by_alias(&existing.alias)?
                .ok_or_else(|| PeerError::NotFound(existing.alias.clone()));
        }
        // 2) 같은 pubkey 존재 (eth 누락 케이스) → UPDATE + eth 보강
        if let Some(existing) = self.get_by_public_key(public_key_hex)? {
            self.db.conn().execute(
                "UPDATE peers SET address = ?1, gui_address = COALESCE(?2, gui_address), eth_address = ?3, last_seen = ?4 WHERE public_key_hex = ?5",
                rusqlite::params![address, gui_address, eth_address, now_rfc, public_key_hex],
            )?;
            return self
                .get_by_alias(&existing.alias)?
                .ok_or_else(|| PeerError::NotFound(existing.alias.clone()));
        }
        // 3) 신규 — gui_address + role 포함 insert
        let id = Uuid::new_v4().to_string();
        self.db.conn().execute(
            "INSERT INTO peers (id, alias, public_key_hex, address, gui_address, role, created_at, last_seen, eth_address, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                id, alias, public_key_hex, address, gui_address, default_role.as_str(),
                now_rfc, now_rfc, eth_address, "auto-registered via envelope (zero-touch rc.244)"
            ],
        )?;
        self.get_by_alias(alias)?
            .ok_or_else(|| PeerError::NotFound(format!("just-inserted: {alias}")))
    }

    /// envelope.from 으로 inbound 매칭. 매칭된 row 수 반환 (0 = 미등록).
    pub fn touch_by_eth_address(&mut self, eth_address: &str) -> Result<usize> {
        let now_rfc = kst_now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE peers SET last_seen = ?1 WHERE eth_address = ?2",
            rusqlite::params![now_rfc, eth_address],
        )?;
        Ok(affected)
    }

    pub fn get_by_alias(&mut self, alias: &str) -> Result<Option<Peer>> {
        Self::row_to_opt(self.db.conn().query_row(
            "SELECT id, alias, public_key_hex, address, role, last_seen, created_at, notes, eth_address
             FROM peers WHERE alias = ?1",
            [alias],
            row_to_tuple,
        ))
    }

    pub fn get_by_public_key(&mut self, public_key_hex: &str) -> Result<Option<Peer>> {
        Self::row_to_opt(self.db.conn().query_row(
            "SELECT id, alias, public_key_hex, address, role, last_seen, created_at, notes, eth_address
             FROM peers WHERE public_key_hex = ?1",
            [public_key_hex],
            row_to_tuple,
        ))
    }

    pub fn list(&mut self) -> Result<Vec<Peer>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, alias, public_key_hex, address, role, last_seen, created_at, notes, eth_address
             FROM peers ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], row_to_tuple)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(tuple_to_peer(r?)?);
        }
        Ok(out)
    }

    pub fn delete(&mut self, alias: &str) -> Result<()> {
        let affected = self
            .db
            .conn()
            .execute("DELETE FROM peers WHERE alias = ?1", [alias])?;
        if affected != 1 {
            return Err(PeerError::NotFound(alias.into()));
        }
        Ok(())
    }

    /// peer 와 통신 성공 시 마지막 연결 시각 갱신.
    pub fn touch(&mut self, alias: &str) -> Result<()> {
        let now_rfc = kst_now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE peers SET last_seen = ?1 WHERE alias = ?2",
            rusqlite::params![now_rfc, alias],
        )?;
        if affected != 1 {
            return Err(PeerError::NotFound(alias.into()));
        }
        Ok(())
    }

    pub fn get_by_eth_address(&mut self, eth_address: &str) -> Result<Option<Peer>> {
        Self::row_to_opt(self.db.conn().query_row(
            "SELECT id, alias, public_key_hex, address, role, last_seen, created_at, notes, eth_address
             FROM peers WHERE eth_address = ?1",
            [eth_address],
            row_to_tuple,
        ))
    }

    /// inbound message 수신 후 호출 — public_key 로 peer 찾아 last_seen 갱신.
    /// 미등록 peer (anonymous) 는 0 반환 (에러 아님 — 등록 안 된 envelope 도 정상 도착 가능).
    /// 매칭 성공 시 1 반환.
    pub fn touch_by_public_key(&mut self, public_key_hex: &str) -> Result<usize> {
        let now_rfc = kst_now().to_rfc3339();
        let affected = self.db.conn().execute(
            "UPDATE peers SET last_seen = ?1 WHERE public_key_hex = ?2",
            rusqlite::params![now_rfc, public_key_hex],
        )?;
        Ok(affected)
    }

    #[allow(clippy::type_complexity)]
    fn row_to_opt(
        result: rusqlite::Result<(
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
        )>,
    ) -> Result<Option<Peer>> {
        match result {
            Ok(t) => Ok(Some(tuple_to_peer(t)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[allow(clippy::type_complexity)]
fn row_to_tuple(
    r: &rusqlite::Row,
) -> rusqlite::Result<(
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
)> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
    ))
}

#[allow(clippy::type_complexity)]
fn tuple_to_peer(
    t: (
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
    ),
) -> Result<Peer> {
    let (id, alias, public_key_hex, address, role, last_seen, created_at, notes, eth_address) = t;
    Ok(Peer {
        id,
        alias,
        public_key_hex,
        address,
        eth_address,
        role: PeerRole::parse(&role)?,
        // rc.188: invalid timestamp graceful fallback. 빈 string / invalid format → None / epoch.
        // 원인: peer add 시 created_at 빈 string 또는 SQL datetime('now') 의 non-RFC3339 format.
        // 이전엔 parse_ts fail → list() 전체 fail → process_inbound silent drop.
        last_seen: last_seen.as_deref().and_then(|s| parse_ts(s).ok()),
        created_at: parse_ts(&created_at).unwrap_or_else(|_| {
            // SQL datetime('now') format = 'YYYY-MM-DD HH:MM:SS' (no T, no TZ). 보완 시도.
            DateTime::parse_from_str(&format!("{}+00:00", created_at.replace(' ', "T")), "%Y-%m-%dT%H:%M:%S%z")
                .unwrap_or_else(|_| chrono::Utc::now().fixed_offset())
        }),
        notes,
    })
}

fn parse_ts(s: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).map_err(|e| PeerError::InvalidTimestamp(e.to_string()))
}
