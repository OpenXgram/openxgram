//! xgram daemon — Phase 1 first PR: scheduler + transport server foreground.
//!
//! Phase 1 first PR: 단일 binary 가 foreground 로 실행. 종료 신호(Ctrl-C)
//! 까지 대기. systemd unit·fork·pid 파일 등은 후속 PR.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use openxgram_core::paths::db_path;
use openxgram_scheduler::{add_reflection_job, build_scheduler, NIGHTLY_REFLECTION_CRON};
use openxgram_transport::spawn_server;

const DEFAULT_BIND: &str = "127.0.0.1:7300";

#[derive(Debug, Clone)]
pub struct DaemonOpts {
    pub data_dir: PathBuf,
    pub bind_addr: Option<SocketAddr>,
    pub reflection_cron: Option<String>,
    /// true 시 `tailscale ip --4` 결과를 기본 bind IP 로 사용. mTLS 는 WireGuard
    /// 터널이 네트워크 레이어에서 제공.
    pub tailscale: bool,
}

pub async fn run_daemon(opts: DaemonOpts) -> Result<()> {
    let bind = if let Some(addr) = opts.bind_addr {
        addr
    } else if opts.tailscale {
        let ip = openxgram_transport::tailscale::local_ipv4()
            .context("--tailscale 요청 — `tailscale ip --4` 실패")?;
        SocketAddr::new(std::net::IpAddr::V4(ip), 7300)
    } else {
        DEFAULT_BIND.parse().expect("DEFAULT_BIND parses")
    };
    let cron = opts
        .reflection_cron
        .unwrap_or_else(|| NIGHTLY_REFLECTION_CRON.to_string());

    println!("xgram daemon");
    println!("  data_dir         : {}", opts.data_dir.display());
    println!("  transport bind   : {bind}");
    println!("  reflection cron  : {cron}");

    let mut scheduler = build_scheduler().await.context("scheduler 생성 실패")?;
    add_reflection_job(&mut scheduler, &cron, db_path(&opts.data_dir))
        .await
        .context("reflection job 등록 실패")?;
    scheduler.start().await.context("scheduler 시작 실패")?;
    println!("  ✓ reflection scheduler started");

    let server = spawn_server(bind)
        .await
        .context("transport server bind 실패")?;
    println!("  ✓ transport server bound: http://{}", server.bound_addr);

    // inbound processor — 1초 주기로 server.drain_received() 한 후 envelope.from 매칭으로
    // peer.touch_by_eth_address. 매칭 실패는 silent (anonymous envelope 도 정상 도착).
    let data_dir_clone = opts.data_dir.clone();
    let received_arc = std::sync::Arc::new(server);
    let received_for_task = received_arc.clone();
    let processor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let envelopes = received_for_task.drain_received();
            if envelopes.is_empty() {
                continue;
            }
            if let Err(e) = process_inbound(&data_dir_clone, &envelopes) {
                tracing::warn!(error = %e, "inbound processor 처리 실패");
            }
        }
    });
    println!("  ✓ inbound processor running (1s interval)");
    println!();
    println!("Ctrl-C 로 종료.");

    tokio::signal::ctrl_c().await.context("signal 대기 실패")?;
    println!();
    println!("종료 신호 수신 — shutdown 중...");

    scheduler
        .shutdown()
        .await
        .context("scheduler shutdown 실패")?;
    processor.abort();
    if let Ok(server) = std::sync::Arc::try_unwrap(received_arc) {
        server.shutdown();
    }

    println!("✓ daemon stopped");
    Ok(())
}

/// drain 된 envelope 들을 한 번의 DB open 으로 처리 (silent error 패턴).
fn process_inbound(
    data_dir: &std::path::Path,
    envelopes: &[openxgram_transport::Envelope],
) -> Result<()> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_peer::PeerStore;
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open (inbound) 실패")?;
    db.migrate().context("DB migrate (inbound) 실패")?;
    let mut store = PeerStore::new(&mut db);
    for env in envelopes {
        match store.touch_by_eth_address(&env.from) {
            Ok(0) => {
                tracing::debug!(from = %env.from, "anonymous inbound (peer 미등록)");
            }
            Ok(_) => {
                tracing::info!(from = %env.from, "peer last_seen 갱신");
            }
            Err(e) => {
                tracing::warn!(error = %e, from = %env.from, "peer.touch 실패");
            }
        }
    }
    Ok(())
}
