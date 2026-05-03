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
    use openxgram_memory::reflect_all;
    let mut db = Db::open(DbConfig {
        path: db_path.to_path_buf(),
        ..Default::default()
    })?;
    db.migrate()?;
    let episodes = reflect_all(&mut db)?;
    tracing::info!(count = episodes.len(), "nightly reflection completed");
    Ok(())
}
