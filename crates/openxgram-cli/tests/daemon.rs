//! daemon 통합 테스트 — scheduler + transport 서버 wiring 만 검증.
//!
//! ctrl_c signal 시뮬레이션이 어려워, run_daemon 전체 라이프사이클은
//! 별도 e2e (수동 실행 또는 systemd 환경)에서 검증한다. 여기서는
//! component 들이 daemon 모듈에 정상 wired 되어 있는지만.

use openxgram_cli::daemon::DaemonOpts;
use std::path::PathBuf;

#[test]
fn daemon_opts_constructable() {
    let _opts = DaemonOpts {
        data_dir: PathBuf::from("/tmp/x"),
        bind_addr: Some("127.0.0.1:7300".parse().unwrap()),
        reflection_cron: Some("0 0 15 * * *".to_string()),
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_components_init_without_blocking() {
    // scheduler 와 transport 가 tokio 환경에서 분리 init 가능한지만 빠르게 검증.
    use openxgram_scheduler::{add_reflection_job, build_scheduler, NIGHTLY_REFLECTION_CRON};
    use openxgram_transport::spawn_server;
    use tempfile::tempdir;

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

    let server = spawn_server("127.0.0.1:0".parse().unwrap()).await.unwrap();
    assert!(server.bound_addr.port() > 0);

    scheduler.shutdown().await.unwrap();
    server.shutdown();
}
