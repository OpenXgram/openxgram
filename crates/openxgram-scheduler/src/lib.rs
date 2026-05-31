//! openxgram-scheduler — cron 기반 백그라운드 작업.
//!
//! Phase 1 first PR: tokio-cron-scheduler wiring + nightly reflection job
//! 등록 함수. 데몬 통합·다른 cron job (cold backup auto, doctor self-check)
//! 은 후속.
//!
//! silent error 게이트: cron job 내부 panic 은 tracing::error 로 기록하고
//! 다음 트리거에 영향 주지 않도록 catch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};

/// 매일 자정 KST = UTC 15:00 — sec min hour DOM month DOW
pub const NIGHTLY_REFLECTION_CRON: &str = "0 0 15 * * *";

pub async fn build_scheduler() -> Result<JobScheduler, JobSchedulerError> {
    JobScheduler::new().await
}

/// scheduler 에 reflection job 을 cron 표현식으로 등록.
/// db_path 는 매 트리거마다 새로 open + migrate (job 격리).
pub async fn add_reflection_job(
    scheduler: &mut JobScheduler,
    cron_expr: &str,
    db_path: PathBuf,
) -> Result<uuid::Uuid, JobSchedulerError> {
    let path = Arc::new(db_path);
    let job = Job::new_async(cron_expr, move |_uuid, _l| {
        let path = path.clone();
        Box::pin(async move {
            if let Err(e) = run_reflection_pass(&path) {
                tracing::error!(error = %e, "nightly reflection failed");
            }
        })
    })?;
    scheduler.add(job).await
}

fn run_reflection_pass(db_path: &Path) -> anyhow::Result<()> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_memory::{derive_traits_from_patterns, reflect_all};
    let mut db = Db::open(DbConfig {
        path: db_path.to_path_buf(),
        ..Default::default()
    })?;
    db.migrate()?;
    let episodes = reflect_all(&mut db)?;
    let traits = derive_traits_from_patterns(&mut db)?;
    // rc.216 — L3 patterns → L2 wiki_pages 격상 (Karpathy 패턴 본질 fix).
    let promoted = promote_patterns_to_wiki(&mut db).unwrap_or(0);
    tracing::info!(
        episodes = episodes.len(),
        derived_traits = traits.len(),
        wiki_promoted = promoted,
        "nightly reflection completed"
    );
    Ok(())
}

/// rc.216 — L3 patterns (RECURRING/ROUTINE) → L2 wiki_pages 자동 격상.
/// 멱등 upsert. NEW(freq=1) 은 격상 안 함. daemon_gui::promote_patterns_to_wiki 와 동일 논리.
fn promote_patterns_to_wiki(db: &mut openxgram_db::Db) -> anyhow::Result<i64> {
    use sha2::{Digest, Sha256};
    let conn = db.conn();
    let rows: Vec<(String, String, i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, pattern_text, frequency, first_seen, last_seen \
             FROM patterns WHERE frequency >= 2 ORDER BY frequency DESC, last_seen DESC LIMIT 500",
        )?;
        let it = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        it.flatten().collect()
    };

    let mut promoted = 0i64;
    for (pid, ptxt, freq, first_seen, last_seen) in rows {
        let class = if freq >= 5 { "routine" } else { "recurring" };
        let page_id = format!("pattern-{}", &pid[..pid.len().min(36)]);
        let file_path = format!("entity/{}.md", page_id);
        let title: String = ptxt.chars().take(80).collect();
        let content = format!(
            "# {title}\n\n- classification: {class}\n- frequency: {freq}\n- first_seen: {first_seen}\n- last_seen: {last_seen}\n- source_pattern_id: {pid}\n\n원본 pattern_text:\n\n> {ptxt}\n",
        );
        let content_hash = format!("{:x}", Sha256::new().chain_update(content.as_bytes()).finalize());
        let now = chrono::Utc::now().timestamp();
        let tags = serde_json::json!([class, "auto-promoted"]).to_string();
        let r = conn.execute(
            "INSERT INTO wiki_pages (id, file_path, page_type, title, content_hash, embedding_hash, created_at, updated_at, category_path, tags, authors) \
             VALUES (?1, ?2, 'entity', ?3, ?4, ?4, ?5, ?5, 'patterns', ?6, '[\"reflection_pass\"]') \
             ON CONFLICT(id) DO UPDATE SET \
                title = excluded.title, \
                content_hash = excluded.content_hash, \
                embedding_hash = excluded.embedding_hash, \
                updated_at = excluded.updated_at, \
                tags = excluded.tags",
            rusqlite::params![page_id, file_path, title, content_hash, now, tags],
        );
        match r {
            Ok(n) if n > 0 => promoted += 1,
            Ok(_) => {}
            Err(e) => tracing::warn!("wiki upsert 실패 ({}): {e}", page_id),
        }
    }
    Ok(promoted)
}
