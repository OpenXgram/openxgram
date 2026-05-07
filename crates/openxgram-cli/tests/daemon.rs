//! daemon 통합 테스트 — scheduler + transport 서버 wiring 만 검증.
//!
//! ctrl_c signal 시뮬레이션이 어려워, run_daemon 전체 라이프사이클은
//! 별도 e2e (수동 실행 또는 systemd 환경)에서 검증한다. 여기서는
//! component 들이 daemon 모듈에 정상 wired 되어 있는지만.

use openxgram_cli::daemon::DaemonOpts;
use std::path::PathBuf;

#[test]
#[serial_test::file_serial]
fn daemon_opts_constructable() {
    let _opts = DaemonOpts {
        data_dir: PathBuf::from("/tmp/x"),
        bind_addr: Some("127.0.0.1:47300".parse().unwrap()),
        gui_bind: None,
        reflection_cron: Some("0 0 15 * * *".to_string()),
        tailscale: false,
    };
}

#[test]
#[serial_test::file_serial]
fn tailscale_module_callable_without_panic() {
    use openxgram_transport::tailscale;
    // is_running 은 항상 bool 반환 — tailscale 미설치 환경에서도 panic 안 함
    let _ = tailscale::is_running();
    // local_ipv4 는 Result — 에러여도 panic 안 함
    let _ = tailscale::local_ipv4();
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
