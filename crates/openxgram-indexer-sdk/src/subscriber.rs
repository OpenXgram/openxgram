//! 4.2.1.1 — EAS 이벤트 구독. 본 SDK 는 두 모드 지원:
//! - DB poll: 같은 머신의 `eas_attestations` 테이블을 watermark 로 폴링 (테스트/단일 노드)
//! - (future) onchain log subscribe: alloy provider 의 `getLogs` (다음 PR)

use chrono::{DateTime, FixedOffset};
use openxgram_db::Db;
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationEvent {
    pub id: String,
    pub schema_uid: String,
    pub kind: String,
    pub fields_json: String,
    pub data_hash: String,
    pub created_at: DateTime<FixedOffset>,
}

pub struct AttestationSubscriber<'a> {
    db: &'a mut Db,
    /// 마지막으로 본 created_at (RFC3339). 빈 문자열 == 처음.
    pub watermark: String,
}

impl<'a> AttestationSubscriber<'a> {
    pub fn new(db: &'a mut Db) -> Self {
        Self {
            db,
            watermark: String::new(),
        }
    }

    /// watermark 보다 이후의 attestation 을 반환 (created_at 오름차순).
    /// 호출 후 watermark 자동 갱신.
    pub fn poll(&mut self) -> Result<Vec<AttestationEvent>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, schema_uid, kind, fields_json, data_hash, created_at
             FROM eas_attestations
             WHERE created_at > ?1
             ORDER BY created_at",
        )?;
        let rows = stmt.query_map([&self.watermark], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, schema_uid, kind, fields_json, data_hash, ts) = row?;
            let parsed = DateTime::parse_from_rfc3339(&ts)
                .map_err(|e| crate::IndexerError::Resolver(e.to_string()))?;
            out.push(AttestationEvent {
                id,
                schema_uid,
                kind,
                fields_json,
                data_hash,
                created_at: parsed,
            });
        }
        if let Some(last) = out.last() {
            self.watermark = last.created_at.to_rfc3339();
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_db::DbConfig;
    use openxgram_eas::{Attestation, AttestationData, AttestationKind, AttestationStore};
    use serde_json::json;

    fn open_db() -> Db {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut db = Db::open(DbConfig {
            path: tmp.path().to_path_buf(),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        std::mem::forget(tmp);
        db
    }

    #[test]
    fn poll_returns_new_attestations_then_advances_watermark() {
        let mut db = open_db();
        // ensure schema first
        AttestationStore::new(&mut db).ensure_schema().unwrap();
        let mut sub = AttestationSubscriber::new(&mut db);
        assert_eq!(sub.poll().unwrap().len(), 0, "초기엔 비어있음");

        // insert 2 attestations
        AttestationStore::new(sub.db)
            .insert(&Attestation::new(AttestationData {
                kind: AttestationKind::Message,
                fields: json!({}),
            }))
            .unwrap();
        AttestationStore::new(sub.db)
            .insert(&Attestation::new(AttestationData {
                kind: AttestationKind::Payment,
                fields: json!({}),
            }))
            .unwrap();

        let events = sub.poll().unwrap();
        assert_eq!(events.len(), 2);
        // 두 번째 poll 은 빈 결과
        assert_eq!(sub.poll().unwrap().len(), 0, "watermark 가 advance");
    }
}
