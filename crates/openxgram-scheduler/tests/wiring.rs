//! scheduler wiring 통합 테스트 — build + add + start/shutdown 라이프사이클.

use openxgram_scheduler::{add_reflection_job, build_scheduler, NIGHTLY_REFLECTION_CRON};
use tempfile::tempdir;

#[tokio::test(flavor = "multi_thread")]
async fn build_and_register_nightly_job() {
    let tmp = tempdir().unwrap();
    let mut scheduler = build_scheduler().await.unwrap();
    let _id = add_reflection_job(
        &mut scheduler,
        NIGHTLY_REFLECTION_CRON,
        tmp.path().join("test.db"),
    )
    .await
    .unwrap();
    scheduler.start().await.unwrap();
    scheduler.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_cron_expression_raises() {
    let tmp = tempdir().unwrap();
    let mut scheduler = build_scheduler().await.unwrap();
    let result = add_reflection_job(&mut scheduler, "not a cron", tmp.path().join("x.db")).await;
    assert!(result.is_err());
}
