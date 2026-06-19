use crate::{PeerError, Result};
use openxgram_db::Db;

/// 정본 신원 매핑 저장소. PeerStore 패턴 미러 (`db: &mut Db`).
pub struct IdentityStore<'a> {
    db: &'a mut Db,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalGroup {
    pub canonical_address: String,
    pub primary_alias: Option<String>,
    pub aliases: Vec<String>,
    pub quarantined: bool,
}

impl<'a> IdentityStore<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self { db }
    }

    /// 변형 alias -> 정본 주소. 매핑 없으면 None.
    pub fn resolve(&mut self, alias: &str) -> Result<Option<String>> {
        let r = self.db.conn().query_row(
            "SELECT canonical_address FROM identity_aliases WHERE alias = ?1",
            [alias],
            |row| row.get::<_, String>(0),
        );
        match r {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// peers 를 스캔해 정본 신원으로 분류·매핑한다.
    /// 그룹핑 키: session_identifier(있으면) -> eth_address -> 둘 다 없으면 격리.
    /// 정본 주소: 그룹 내 role='primary' 의 eth_address (없으면 첫 행의 eth_address, 그것도 없으면 sid:<session>).
    pub fn reconcile(&mut self, now_rfc3339: &str) -> Result<()> {
        struct Row {
            alias: String,
            eth: Option<String>,
            sid: Option<String>,
            role: String,
        }
        let rows: Vec<Row> = {
            let mut stmt = self.db.conn().prepare(
                "SELECT alias, eth_address, session_identifier, role FROM peers ORDER BY created_at ASC",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok(Row {
                    alias: r.get(0)?,
                    eth: r.get(1)?,
                    sid: r.get(2)?,
                    role: r.get(3)?,
                })
            })?;
            let mut out = Vec::new();
            for r in mapped {
                out.push(r?);
            }
            out
        };

        use std::collections::BTreeMap;
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut quarantine: Vec<usize> = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let key = if let Some(sid) = &row.sid {
                Some(format!("sid:{sid}"))
            } else {
                row.eth.clone()
            };
            match key {
                Some(k) => groups.entry(k).or_default().push(i),
                None => quarantine.push(i),
            }
        }

        for (_key, idxs) in &groups {
            let primary_idx = idxs
                .iter()
                .copied()
                .find(|&i| rows[i].role == "primary")
                .unwrap_or(idxs[0]);
            let canonical_address = rows[primary_idx]
                .eth
                .clone()
                .or_else(|| idxs.iter().filter_map(|&i| rows[i].eth.clone()).next())
                .unwrap_or_else(|| {
                    rows[primary_idx]
                        .sid
                        .clone()
                        .map(|s| format!("sid:{s}"))
                        .unwrap_or_else(|| format!("alias:{}", rows[primary_idx].alias))
                });
            for &i in idxs {
                let is_primary = i == primary_idx;
                self.upsert_alias(&rows[i].alias, &canonical_address, is_primary, "active", now_rfc3339)?;
            }
        }

        for &i in &quarantine {
            let canon = format!("alias:{}", rows[i].alias);
            self.upsert_alias(&rows[i].alias, &canon, false, "quarantined", now_rfc3339)?;
        }

        Ok(())
    }

    /// 정본 주소별 그룹 목록 (현황 그리드 P2 용).
    pub fn groups(&mut self) -> Result<Vec<CanonicalGroup>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT canonical_address, alias, is_primary_alias, status
             FROM identity_aliases ORDER BY canonical_address, alias",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? != 0,
                r.get::<_, String>(3)?,
            ))
        })?;
        use std::collections::BTreeMap;
        let mut map: BTreeMap<String, CanonicalGroup> = BTreeMap::new();
        for row in rows {
            let (canon, alias, is_primary, status) = row?;
            let g = map.entry(canon.clone()).or_insert_with(|| CanonicalGroup {
                canonical_address: canon.clone(),
                primary_alias: None,
                aliases: Vec::new(),
                quarantined: false,
            });
            if is_primary {
                g.primary_alias = Some(alias.clone());
            }
            if status == "quarantined" {
                g.quarantined = true;
            }
            g.aliases.push(alias);
        }
        Ok(map.into_values().collect())
    }

    /// 정본 alias 재지정: 그룹 내 다른 행의 primary 해제 후 지정 alias 를 primary 로.
    /// 대상 alias 가 그룹에 없으면 아무것도 변경하지 않고 NotFound 반환(기존 primary 보존).
    pub fn set_primary_alias(&mut self, canonical_address: &str, alias: &str) -> Result<()> {
        // 존재 확인을 먼저 — 그래야 없는 alias 호출 시 기존 primary 를 날리지 않는다.
        let exists: bool = self.db.conn().query_row(
            "SELECT COUNT(*) FROM identity_aliases WHERE canonical_address = ?1 AND alias = ?2",
            rusqlite::params![canonical_address, alias],
            |r| r.get::<_, i64>(0),
        )? > 0;
        if !exists {
            return Err(PeerError::NotFound(format!(
                "alias '{alias}' 가 정본 '{canonical_address}' 그룹에 없음"
            )));
        }
        self.db.conn().execute(
            "UPDATE identity_aliases SET is_primary_alias = 0 WHERE canonical_address = ?1",
            [canonical_address],
        )?;
        self.db.conn().execute(
            "UPDATE identity_aliases SET is_primary_alias = 1 WHERE canonical_address = ?1 AND alias = ?2",
            rusqlite::params![canonical_address, alias],
        )?;
        Ok(())
    }

    /// 별칭 upsert. created_at 은 RFC3339 호출자 주입.
    pub fn upsert_alias(
        &mut self,
        alias: &str,
        canonical_address: &str,
        is_primary: bool,
        status: &str,
        created_at: &str,
    ) -> Result<()> {
        self.db.conn().execute(
            "INSERT INTO identity_aliases (alias, canonical_address, is_primary_alias, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(alias) DO UPDATE SET
               canonical_address = excluded.canonical_address,
               is_primary_alias  = excluded.is_primary_alias,
               status            = excluded.status",
            rusqlite::params![alias, canonical_address, is_primary as i64, status, created_at],
        )?;
        Ok(())
    }
}
