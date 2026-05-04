//! ratchet 1주 회전 cron — kind 30050 announce publish + metric (PRD-NOSTR-12).
//!
//! 정책:
//! - period_secs 마다 ratchet.rotate_now → build_announce → NostrSink.publish
//! - prometheus counter (ratchet_rotation_total) + gauge (ratchet_last_rotated_unix_ts)
//! - audit chain 기록은 PRD-AUDIT 후 추가 — 지금은 tracing::info 로 placeholder

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use openxgram_keystore::Keypair;
use openxgram_nostr::{keys_from_master, NostrSink, Ratchet};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};

/// 매 일요일 03:00 KST = UTC 토요일 18:00. sec min hour DOM month DOW
pub const WEEKLY_ROTATION_CRON: &str = "0 0 18 * * Sat";

/// 회전 누적 횟수 (Prometheus exposition 시 daemon.rs gather_db_metrics 와 같은 패턴으로 노출).
pub static RATCHET_ROTATION_TOTAL: AtomicU64 = AtomicU64::new(0);

/// 마지막 회전 unix ts. 미회전 시 0.
pub static RATCHET_LAST_ROTATED_UNIX_TS: AtomicI64 = AtomicI64::new(0);

/// /v1/metrics 추가 노출 — daemon 측에서 이어 붙임.
pub fn metrics_exposition() -> String {
    let total = RATCHET_ROTATION_TOTAL.load(Ordering::Relaxed);
    let last = RATCHET_LAST_ROTATED_UNIX_TS.load(Ordering::Relaxed);
    format!(
        "# HELP openxgram_ratchet_rotation_total ratchet 회전 누적 횟수\n# TYPE openxgram_ratchet_rotation_total counter\nopenxgram_ratchet_rotation_total {total}\n\
# HELP openxgram_ratchet_last_rotated_unix_ts 마지막 ratchet 회전 unix ts (KST)\n# TYPE openxgram_ratchet_last_rotated_unix_ts gauge\nopenxgram_ratchet_last_rotated_unix_ts {last}\n"
    )
}

/// 1회 회전 — ratchet.rotate_now + build_announce + sink.publish.
/// metrics counter+gauge 갱신 + tracing audit log.
pub async fn rotate_once(
    ratchet: &Arc<Mutex<Ratchet>>,
    master: &Keypair,
    relays: &[String],
) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock 비정상")?
        .as_secs();

    let nostr_keys = keys_from_master(master)
        .map_err(|e| anyhow::anyhow!("master → nostr keys 변환 실패: {e}"))?;

    // ratchet rotate + announce build (lock 잠금 시간 최소화)
    let announce_event = {
        let mut r = ratchet.lock().await;
        r.rotate_now(now);
        r.build_announce(&nostr_keys, now)
            .map_err(|e| anyhow::anyhow!("build_announce 실패: {e}"))?
    };

    // publish
    let sink = NostrSink::new(nostr_keys);
    sink.add_relays(relays.iter().cloned())
        .await
        .map_err(|e| anyhow::anyhow!("relay 추가 실패: {e}"))?;
    sink.client()
        .send_event(&announce_event)
        .await
        .map_err(|e| anyhow::anyhow!("announce publish 실패: {e}"))?;
    sink.shutdown().await;

    RATCHET_ROTATION_TOTAL.fetch_add(1, Ordering::Relaxed);
    RATCHET_LAST_ROTATED_UNIX_TS.store(now as i64, Ordering::Relaxed);
    tracing::info!(unix_ts = now, "ratchet rotated + announce published (audit row deferred to PRD-AUDIT)");
    Ok(())
}

/// scheduler 에 weekly ratchet rotation job 등록.
/// master 는 Arc 로 공유 (Keypair 자체는 Clone 미구현).
pub async fn add_ratchet_rotation_job(
    scheduler: &mut JobScheduler,
    cron_expr: &str,
    ratchet: Arc<Mutex<Ratchet>>,
    master: Arc<Keypair>,
    relays: Vec<String>,
) -> Result<uuid::Uuid, JobSchedulerError> {
    let job = Job::new_async(cron_expr, move |_uuid, _l| {
        let ratchet = ratchet.clone();
        let master = master.clone();
        let relays = relays.clone();
        Box::pin(async move {
            if let Err(e) = rotate_once(&ratchet, &master, &relays).await {
                tracing::error!(error = %e, "ratchet weekly rotation failed");
            }
        })
    })?;
    scheduler.add(job).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_keystore::{FsKeystore, Keystore};

    #[tokio::test]
    async fn rotate_once_publishes_announce_and_increments_metric() {
        let relay = nostr_relay_builder::MockRelay::run().await.unwrap();
        let url = relay.url();

        let tmp = tempfile::tempdir().unwrap();
        let ks = FsKeystore::new(tmp.path());
        ks.create("rotate-test", "pw").unwrap();
        let master = ks.load("rotate-test", "pw").unwrap();

        let ratchet = Arc::new(Mutex::new(Ratchet::default()));
        let before = RATCHET_ROTATION_TOTAL.load(Ordering::Relaxed);

        rotate_once(&ratchet, &master, &[url]).await.unwrap();

        let after = RATCHET_ROTATION_TOTAL.load(Ordering::Relaxed);
        assert_eq!(after, before + 1);
        assert!(RATCHET_LAST_ROTATED_UNIX_TS.load(Ordering::Relaxed) > 0);

        // 회전 후 retained periods 1개 이상
        let retained = ratchet.lock().await.retained_periods();
        assert!(!retained.is_empty());
    }

    #[test]
    fn weekly_cron_is_valid() {
        // tokio-cron-scheduler 가 파싱 가능한지 (런타임 검증)
        let job = Job::new_async(WEEKLY_ROTATION_CRON, |_, _| Box::pin(async {}));
        assert!(job.is_ok(), "weekly cron 표현식 파싱 실패");
    }
}
