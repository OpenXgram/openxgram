//! Retention 정책 (PRD-RET-01, 02, 03).
//!
//! 레이어별 정책 (PRD §5.2):
//! - L0 messages: 90일 후 episode summary 압축 → 원본 cold backup → 삭제
//! - L2 memories: pinned 무기한 / unpinned 180일 + LRU access_count
//! - L3 patterns: 영구
//! - L4 traits: 영구
//! - vault_audit: 1년 hot SQLite + 영구 cold (age NDJSON)
//!
//! preview = read-only SELECT COUNT, apply = 마스터 confirm 후 DELETE + audit row 기록

use std::path::Path;

use anyhow::{Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    L0,
    L2,
}

impl Layer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L2 => "L2",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub layer: Layer,
    pub older_than_days: i64,
}

pub const DEFAULT_L0_RETENTION_DAYS: i64 = 90;
pub const DEFAULT_L2_RETENTION_DAYS: i64 = 180;

#[derive(Debug, Clone)]
pub struct PreviewReport {
    pub layer: Layer,
    pub older_than_days: i64,
    pub candidate_count: i64,
    pub cutoff_kst: String,
}

/// preview — read-only count. db read connection 만 사용.
pub fn preview(data_dir: &Path, policy: RetentionPolicy) -> Result<PreviewReport> {
    let mut db = open_db(data_dir)?;
    let cutoff_ts =
        openxgram_core::time::kst_now() - chrono::Duration::days(policy.older_than_days);
    let cutoff_iso = cutoff_ts.to_rfc3339();

    let count = match policy.layer {
        Layer::L0 => count_old_messages(&mut db, &cutoff_iso)?,
        Layer::L2 => count_old_memories(&mut db, &cutoff_iso)?,
    };
    Ok(PreviewReport {
        layer: policy.layer,
        older_than_days: policy.older_than_days,
        candidate_count: count,
        cutoff_kst: cutoff_iso,
    })
}

fn count_old_messages(db: &mut Db, cutoff_iso: &str) -> Result<i64> {
    let conn = db.conn();
    conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE timestamp < ?1",
        rusqlite::params![cutoff_iso],
        |r| r.get::<_, i64>(0),
    )
    .with_context(|| "L0 messages 카운트 실패")
}

fn count_old_memories(db: &mut Db, cutoff_iso: &str) -> Result<i64> {
    let conn = db.conn();
    // pinned=0 unpinned 만 — pinned 컬럼 존재 가정
    conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE created_at < ?1 AND COALESCE(pinned, 0) = 0",
        rusqlite::params![cutoff_iso],
        |r| r.get::<_, i64>(0),
    )
    .with_context(|| "L2 memories 카운트 실패")
}

#[derive(Debug, Clone)]
pub struct ApplyReport {
    pub layer: Layer,
    pub deleted_count: i64,
    pub cutoff_kst: String,
}

/// apply — 마스터 confirm 후 실제 DELETE. audit row 자동 기록.
/// dry_run=true 시 SELECT 만, false 시 DELETE.
pub fn apply(data_dir: &Path, policy: RetentionPolicy, dry_run: bool) -> Result<ApplyReport> {
    let mut db = open_db(data_dir)?;
    let cutoff_ts =
        openxgram_core::time::kst_now() - chrono::Duration::days(policy.older_than_days);
    let cutoff_iso = cutoff_ts.to_rfc3339();

    let count = match policy.layer {
        Layer::L0 => count_old_messages(&mut db, &cutoff_iso)?,
        Layer::L2 => count_old_memories(&mut db, &cutoff_iso)?,
    };

    if dry_run {
        return Ok(ApplyReport {
            layer: policy.layer,
            deleted_count: count,
            cutoff_kst: cutoff_iso,
        });
    }

    // 실 삭제 — audit row 기록 우선
    record_retention_audit(&mut db, policy.layer, count, &cutoff_iso)?;

    let conn = db.conn();
    let deleted = match policy.layer {
        Layer::L0 => conn
            .execute(
                "DELETE FROM messages WHERE timestamp < ?1",
                rusqlite::params![cutoff_iso],
            )
            .context("L0 DELETE 실패")?,
        Layer::L2 => conn
            .execute(
                "DELETE FROM memories WHERE created_at < ?1 AND COALESCE(pinned, 0) = 0",
                rusqlite::params![cutoff_iso],
            )
            .context("L2 DELETE 실패")?,
    };
    Ok(ApplyReport {
        layer: policy.layer,
        deleted_count: deleted as i64,
        cutoff_kst: cutoff_iso,
    })
}

fn record_retention_audit(db: &mut Db, layer: Layer, count: i64, cutoff_iso: &str) -> Result<()> {
    use openxgram_vault::audit_chain::{chain_hash, next_seq_and_prev, AuditEntry};
    let id = uuid::Uuid::new_v4().to_string();
    let ts = openxgram_core::time::kst_now().to_rfc3339();
    let key = format!("retention/{}", layer.as_str());
    let reason = format!("RETENTION_APPLY count={count} cutoff={cutoff_iso}");
    let entry = AuditEntry {
        id: id.clone(),
        key: key.clone(),
        agent: "master".into(),
        action: "delete".into(),
        allowed: true,
        reason: Some(reason.clone()),
        timestamp: ts.clone(),
    };
    let (seq, prev) = next_seq_and_prev(db).context("audit chain 조회 실패")?;
    let h = chain_hash(&prev, &entry);
    db.conn()
        .execute(
            "INSERT INTO vault_audit (id, key, agent, action, allowed, reason, timestamp, prev_hash, entry_hash, seq)
             VALUES (?1, ?2, 'master', 'delete', 1, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, key, reason, ts, &prev[..], &h[..], seq],
        )
        .context("retention audit row insert 실패")?;
    Ok(())
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}

/// /v1/metrics 추가 노출 — preview 카운트 게이지 (PRD-RET-03).
pub fn metrics_exposition(data_dir: &Path) -> String {
    let mut out = String::new();
    for layer in [Layer::L0, Layer::L2] {
        let days = match layer {
            Layer::L0 => DEFAULT_L0_RETENTION_DAYS,
            Layer::L2 => DEFAULT_L2_RETENTION_DAYS,
        };
        let n = preview(
            data_dir,
            RetentionPolicy {
                layer,
                older_than_days: days,
            },
        )
        .map(|r| r.candidate_count)
        .unwrap_or(0);
        out.push_str(&format!(
            "# HELP openxgram_retention_candidates_total {} 레이어 retention 후보 수\n# TYPE openxgram_retention_candidates_total gauge\nopenxgram_retention_candidates_total{{layer=\"{}\"}} {}\n",
            layer.as_str(), layer.as_str(), n
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::tempdir;

    fn fixture_with_messages(n_old: i64, n_new: i64) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let path = db_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut db = Db::open(DbConfig {
            path,
            ..Default::default()
        })
        .unwrap();
        db.migrate().unwrap();

        // session 1개 생성 (messages 가 session_id 외래키 가정)
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, title, created_at, last_active, home_machine)
             VALUES ('s1', 'test', '2025-01-01T00:00:00+09:00', '2025-01-01T00:00:00+09:00', 'test-machine')",
            [],
        )
        .ok();

        // 오래된 messages
        for i in 0..n_old {
            let id = format!("old-{i}");
            conn.execute(
                "INSERT INTO messages (id, session_id, sender, body, signature, timestamp)
                 VALUES (?1, 's1', 'user', 'old', '00', '2025-01-01T00:00:00+09:00')",
                params![id],
            )
            .ok();
        }
        // 최근 messages
        let now = chrono::Utc::now()
            .with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap())
            .to_rfc3339();
        for i in 0..n_new {
            let id = format!("new-{i}");
            conn.execute(
                "INSERT INTO messages (id, session_id, sender, body, signature, timestamp)
                 VALUES (?1, 's1', 'user', 'new', '00', ?2)",
                params![id, now],
            )
            .ok();
        }
        dir
    }

    #[test]
    fn preview_counts_only_old_messages() {
        let dir = fixture_with_messages(3, 2);
        let r = preview(
            dir.path(),
            RetentionPolicy {
                layer: Layer::L0,
                older_than_days: 30,
            },
        )
        .unwrap();
        assert_eq!(r.candidate_count, 3);
    }

    #[test]
    fn preview_short_threshold_includes_recent() {
        let dir = fixture_with_messages(2, 1);
        let r = preview(
            dir.path(),
            RetentionPolicy {
                layer: Layer::L0,
                older_than_days: 0, // 모든 created_at < cutoff (cutoff = now)
            },
        )
        .unwrap();
        // now timestamp 가 created_at 와 정확히 동일이면 < 가 아님 — 0~3 까지 가능
        assert!(r.candidate_count >= 2);
    }

    #[test]
    fn apply_dry_run_does_not_delete() {
        let dir = fixture_with_messages(3, 2);
        let r = apply(
            dir.path(),
            RetentionPolicy {
                layer: Layer::L0,
                older_than_days: 30,
            },
            true, // dry_run
        )
        .unwrap();
        assert_eq!(r.deleted_count, 3);
        // 다시 카운트 — 실 row 변동 없음
        let r2 = preview(
            dir.path(),
            RetentionPolicy {
                layer: Layer::L0,
                older_than_days: 30,
            },
        )
        .unwrap();
        assert_eq!(r2.candidate_count, 3, "dry-run 후에도 row 수 불변");
    }

    #[test]
    fn apply_real_deletes_and_records_audit() {
        let dir = fixture_with_messages(3, 2);
        let r = apply(
            dir.path(),
            RetentionPolicy {
                layer: Layer::L0,
                older_than_days: 30,
            },
            false,
        )
        .unwrap();
        assert_eq!(r.deleted_count, 3);
        // 재 카운트 → 0
        let r2 = preview(
            dir.path(),
            RetentionPolicy {
                layer: Layer::L0,
                older_than_days: 30,
            },
        )
        .unwrap();
        assert_eq!(r2.candidate_count, 0);

        // audit row 기록 확인
        let mut db = open_db(dir.path()).unwrap();
        let n: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM vault_audit WHERE key LIKE 'retention/%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "audit row 1개 기록");
    }

    #[test]
    fn metrics_exposition_contains_both_layers() {
        let dir = fixture_with_messages(0, 0);
        let m = metrics_exposition(dir.path());
        assert!(m.contains("layer=\"L0\""));
        assert!(m.contains("layer=\"L2\""));
        assert!(m.contains("openxgram_retention_candidates_total"));
    }
}
