//! step 14 — `xgram payment summary` 수익 대시보드.
//!
//! 본 노드의 payment_intents 테이블 + EAS payment attestations 종합:
//!  - 받은 금액 (수신 — agent 가 외부에서 받은 결제)
//!  - 보낸 금액 (송신 — payment_intents.draft/signed/submitted/confirmed)
//!  - 순수익
//!  - state 별 분포
//!
//! 단순 출력 — 표 형태.

use anyhow::{Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use std::path::Path;

pub struct PaymentSummary {
    pub total_sent_micros: u64,
    pub total_received_micros: u64,
    pub by_state: std::collections::HashMap<String, (u64, u64)>, // state → (count, sum_micros)
    pub net_micros: i64,
}

pub fn run_summary(data_dir: &Path) -> Result<()> {
    let summary = compute_summary(data_dir)?;
    println!("OpenXgram 수익 요약");
    println!("─────────────────────────────");
    println!(
        "받은 금액 (수신) : {:>12} micro USDC = {:>10.2} USDC",
        summary.total_received_micros,
        summary.total_received_micros as f64 / 1_000_000.0
    );
    println!(
        "보낸 금액 (송신) : {:>12} micro USDC = {:>10.2} USDC",
        summary.total_sent_micros,
        summary.total_sent_micros as f64 / 1_000_000.0
    );
    println!(
        "순수익           : {:>12} micro USDC = {:>10.2} USDC",
        summary.net_micros,
        summary.net_micros as f64 / 1_000_000.0
    );
    if !summary.by_state.is_empty() {
        println!();
        println!("송신 state 별:");
        let mut entries: Vec<_> = summary.by_state.iter().collect();
        entries.sort_by_key(|(k, _)| k.clone());
        for (state, (count, sum)) in entries {
            println!(
                "  {:<12} {:>5} 건  {:>12} micro = {:>8.2} USDC",
                state,
                count,
                sum,
                *sum as f64 / 1_000_000.0
            );
        }
    }
    Ok(())
}

pub fn compute_summary(data_dir: &Path) -> Result<PaymentSummary> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open")?;
    db.migrate().context("DB migrate")?;

    // 보낸 금액 — payment_intents 의 amount_usdc_micro 합 (state 별)
    let mut by_state: std::collections::HashMap<String, (u64, u64)> = Default::default();
    let mut total_sent = 0u64;
    {
        let conn = db.conn();
        let mut stmt = conn.prepare(
            "SELECT state, COUNT(*), COALESCE(SUM(amount_usdc_micro), 0)
             FROM payment_intents GROUP BY state",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (state, count, sum) = row?;
            let sum_u = sum.max(0) as u64;
            by_state.insert(state.clone(), (count.max(0) as u64, sum_u));
            // 실제로 보낸 것 = submitted/confirmed 만 (draft/signed/failed 는 송신 안 함)
            if state == "submitted" || state == "confirmed" {
                total_sent += sum_u;
            }
        }
    }

    // 받은 금액 — eas_attestations 의 kind='payment' fields_json.amount_micros 합산
    // (recipient = "us-addr" 또는 master 자기 주소 인 항목만)
    let mut total_received = 0u64;
    {
        let conn = db.conn();
        // eas_attestations 테이블 미존재 시 silent skip
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='eas_attestations')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if exists {
            let mut stmt = conn.prepare(
                "SELECT fields_json FROM eas_attestations WHERE kind='payment'",
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for row in rows {
                if let Ok(json) = row {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                        if let Some(amt) = v.get("amount_micros").and_then(|x| x.as_u64()) {
                            total_received += amt;
                        }
                    }
                }
            }
        }
    }

    let net = total_received as i64 - total_sent as i64;
    Ok(PaymentSummary {
        total_sent_micros: total_sent,
        total_received_micros: total_received,
        by_state,
        net_micros: net,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn summary_empty_db_zero() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let mut db = Db::open(DbConfig {
            path: db_path(dir),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        drop(db);
        let s = compute_summary(dir).unwrap();
        assert_eq!(s.total_sent_micros, 0);
        assert_eq!(s.total_received_micros, 0);
        assert_eq!(s.net_micros, 0);
    }

    #[test]
    fn summary_aggregates_payment_intents_by_state() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let mut db = Db::open(DbConfig {
            path: db_path(dir),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        let now = openxgram_core::time::kst_now().to_rfc3339();
        // draft 1건, confirmed 1건
        db.conn()
            .execute(
                "INSERT INTO payment_intents
                  (id, amount_usdc_micro, chain, payee_address, memo, nonce, state, created_at)
                 VALUES ('p1', 100000, 'base', 'x', 'a', 'n1', 'draft', ?1),
                        ('p2', 500000, 'base', 'y', 'b', 'n2', 'confirmed', ?1)",
                [&now],
            )
            .unwrap();
        drop(db);
        let s = compute_summary(dir).unwrap();
        assert_eq!(s.total_sent_micros, 500_000); // confirmed 만
        assert_eq!(s.by_state.get("draft").unwrap(), &(1, 100_000));
        assert_eq!(s.by_state.get("confirmed").unwrap(), &(1, 500_000));
    }

    #[test]
    fn summary_includes_received_from_eas_payment() {
        use openxgram_eas::{Attestation, AttestationData, AttestationKind, AttestationStore};
        use serde_json::json;

        let tmp = tempdir().unwrap();
        let dir = tmp.path();
        let mut db = Db::open(DbConfig {
            path: db_path(dir),
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();
        AttestationStore::new(&mut db)
            .insert(&Attestation::new(AttestationData {
                kind: AttestationKind::Payment,
                fields: json!({
                    "sender": "outside",
                    "recipient": "us",
                    "amount_micros": 1_000_000u64,
                    "chain": "base",
                    "intent_id": "i1"
                }),
            }))
            .unwrap();
        AttestationStore::new(&mut db)
            .insert(&Attestation::new(AttestationData {
                kind: AttestationKind::Payment,
                fields: json!({"amount_micros": 250_000u64}),
            }))
            .unwrap();
        drop(db);
        let s = compute_summary(dir).unwrap();
        assert_eq!(s.total_received_micros, 1_250_000);
        assert_eq!(s.net_micros, 1_250_000); // 송신 0, 수신 1.25M
    }
}
