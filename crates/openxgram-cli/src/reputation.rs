//! 5.3 평판 기반 랭킹 — 본 노드의 메모리/결제/엔도스먼트 카운트로 IdentityScore 집계.
//!
//! 5.2 외부 거래 데모는 `tests/external_trade_demo.rs` 가 담당 — 본 모듈은 score 만.

use anyhow::Result;
use std::collections::HashMap;

use openxgram_db::{Db, DbConfig};
use openxgram_indexer_sdk::{DefaultRanker, IdentityScore, Rank};

use openxgram_core::paths::db_path;
use std::path::Path;

/// 본 노드의 DB 에서 identity 별 카운트 집계 → IdentityScore 리스트.
/// 식별자: peer.alias (peer 테이블 기준).
pub fn aggregate_local_scores(data_dir: &Path) -> Result<Vec<IdentityScore>> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })?;
    db.migrate()?;

    let mut by_id: HashMap<String, IdentityScore> = HashMap::new();

    // 1) 메시지 카운트 — sender 가 `peer:<alias>` 인 inbox 메시지를 endorser/endorsee 인덱스로
    let message_counts: Vec<(String, i64)> = {
        let conn = db.conn();
        let mut stmt = conn.prepare(
            "SELECT REPLACE(sender, 'peer:', ''), COUNT(*)
             FROM messages
             WHERE sender LIKE 'peer:%'
             GROUP BY sender",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (alias, count) in message_counts {
        let s = by_id.entry(alias.clone()).or_insert_with(|| IdentityScore {
            identity: alias,
            ..Default::default()
        });
        s.messages += count as u64;
    }

    // 2) 결제 받음 카운트 — payment_intents 테이블 (status=confirmed) recipient 별
    let payment_counts: Vec<(String, i64)> = {
        let conn = db.conn();
        match conn.prepare(
            "SELECT payee_address, COUNT(*) FROM payment_intents
             WHERE state='confirmed' GROUP BY payee_address",
        ) {
            Ok(mut stmt) => {
                match stmt.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                }) {
                    Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                    Err(_) => Vec::new(),
                }
            }
            Err(_) => Vec::new(),
        }
    };
    for (id, count) in payment_counts {
        let s = by_id.entry(id.clone()).or_insert_with(|| IdentityScore {
            identity: id,
            ..Default::default()
        });
        s.payments_received += count as u64;
    }

    // 3) endorsement — eas_attestations 의 kind='endorsement' fields_json 의 endorsee
    let endorsement_jsons: Vec<String> = {
        let conn = db.conn();
        match conn.prepare(
            "SELECT fields_json FROM eas_attestations WHERE kind='endorsement'",
        ) {
            Ok(mut stmt) => {
                match stmt.query_map([], |r| r.get::<_, String>(0)) {
                    Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                    Err(_) => Vec::new(),
                }
            }
            Err(_) => Vec::new(),
        }
    };
    for json in endorsement_jsons {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            if let Some(endorsee) = v.get("endorsee").and_then(|x| x.as_str()) {
                let s = by_id.entry(endorsee.to_string()).or_insert_with(|| {
                    IdentityScore {
                        identity: endorsee.to_string(),
                        ..Default::default()
                    }
                });
                s.endorsements_received += 1;
            }
        }
    }

    Ok(DefaultRanker::default().rank(by_id.into_values().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_eas::{Attestation, AttestationData, AttestationKind, AttestationStore};
    use serde_json::json;
    use tempfile::tempdir;

    fn open_db(dir: &Path) -> Db {
        let mut db = Db::open(DbConfig {
            path: db_path(dir),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        db
    }

    fn insert_session(db: &mut Db, id: &str, title: &str) {
        let now = openxgram_core::time::kst_now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, last_active, home_machine)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, title, &now, &now, "test-host"],
            )
            .unwrap();
    }

    fn insert_message(db: &mut Db, session_id: &str, sender: &str, body: &str) {
        let id = uuid::Uuid::new_v4().to_string();
        let now = openxgram_core::time::kst_now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO messages
                  (id, session_id, sender, body, signature, timestamp, conversation_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    id, session_id, sender, body, "sig", &now, "conv-1"
                ],
            )
            .unwrap();
    }

    #[test]
    fn aggregate_counts_messages_and_endorsements() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir).unwrap();
        let mut db = open_db(dir);
        insert_session(&mut db, "s1", "inbox-from-alice");
        insert_session(&mut db, "s2", "inbox-from-bob");
        insert_message(&mut db, "s1", "peer:alice", "hi");
        insert_message(&mut db, "s1", "peer:alice", "again");
        insert_message(&mut db, "s2", "peer:bob", "hey");

        // bob endorsed
        AttestationStore::new(&mut db)
            .insert(&Attestation::new(AttestationData {
                kind: AttestationKind::Endorsement,
                fields: json!({"endorser": "alice", "endorsee": "bob", "tag": "trustworthy"}),
            }))
            .unwrap();
        drop(db);

        let scores = aggregate_local_scores(dir).unwrap();
        let alice = scores.iter().find(|s| s.identity == "alice").unwrap();
        let bob = scores.iter().find(|s| s.identity == "bob").unwrap();
        assert_eq!(alice.messages, 2);
        assert_eq!(bob.messages, 1);
        assert_eq!(bob.endorsements_received, 1);
        // bob 의 raw_score 가 alice 보다 큼 (endorsement 가중치)
        assert!(bob.raw_score > alice.raw_score);
    }

    #[test]
    fn empty_db_returns_no_scores() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir).unwrap();
        let _db = open_db(dir);
        // ensure_schema for eas
        let mut db = open_db(dir);
        AttestationStore::new(&mut db).ensure_schema().unwrap();
        drop(db);
        let scores = aggregate_local_scores(dir).unwrap();
        assert!(scores.is_empty());
    }
}
