//! daemon nostr inbound processor — kind 30500 L0Message subscribe + NIP-44 unwrap +
//! envelope JSON deserialize → process_inbound 라우팅 (PRD-NOSTR-10).
//!
//! 활성 조건: XGRAM_NOSTR_RELAYS env 가 colon-separated 로 제공된 경우.
//! 기본 polling interval = 10초 (XGRAM_NOSTR_POLL_SECS env 로 override).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use openxgram_keystore::Keypair;
use openxgram_nostr::{
    keys_from_master, try_unwrap_with_warn, Filter, NostrKind, NostrKindRaw, NostrSource,
};
use openxgram_transport::Envelope;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const DEFAULT_POLL_SECS: u64 = 10;

#[derive(Debug, Clone)]
pub struct NostrInboundConfig {
    pub data_dir: PathBuf,
    pub relays: Vec<String>,
    pub poll_interval: Duration,
}

impl NostrInboundConfig {
    /// env 기반 자동 설정. relays 미설정 시 None 반환 (opt-in).
    pub fn from_env(data_dir: PathBuf) -> Option<Self> {
        let relays_env = std::env::var("XGRAM_NOSTR_RELAYS").ok()?;
        let relays: Vec<String> = relays_env
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if relays.is_empty() {
            return None;
        }
        let secs = std::env::var("XGRAM_NOSTR_POLL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_POLL_SECS);
        Some(Self {
            data_dir,
            relays,
            poll_interval: Duration::from_secs(secs),
        })
    }
}

/// nostr inbound processor 백그라운드 task spawn.
/// shutdown_rx 시그널 수신 시 graceful 종료.
pub async fn spawn_nostr_inbound_processor(
    cfg: NostrInboundConfig,
    master: Keypair,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<JoinHandle<()>> {
    let nostr_keys = keys_from_master(&master)
        .map_err(|e| anyhow::anyhow!("master → nostr keys 변환 실패: {e}"))?;
    let source = NostrSource::new(nostr_keys.clone());
    source
        .add_relays(cfg.relays.iter().cloned())
        .await
        .map_err(|e| anyhow::anyhow!("relay 추가 실패: {e}"))?;

    // (sender_pubkey, ciphertext) 채널 — listener → drain task
    let (tx, mut rx) = mpsc::unbounded_channel::<(openxgram_nostr::PublicKey, String)>();
    let _listener = source.spawn_listener(move |event| {
        let _ = tx.send((event.pubkey, event.content));
    });

    let filter = Filter::new().kind(NostrKindRaw::from(NostrKind::L0Message));
    source
        .subscribe(filter)
        .await
        .map_err(|e| anyhow::anyhow!("subscribe 실패: {e}"))?;

    let receiver_secret_arc = Arc::new(nostr_keys.secret_key().clone());
    let data_dir = cfg.data_dir.clone();
    let interval_secs = cfg.poll_interval;
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(interval_secs);
        // 첫 tick 즉시 — 누적 envelope batch 처리
        let mut batch: Vec<Envelope> = Vec::new();
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                _ = interval.tick() => {
                    drain_into_batch(&receiver_secret_arc, &mut rx, &mut batch);
                    if !batch.is_empty() {
                        let envelopes: Vec<Envelope> = std::mem::take(&mut batch);
                        if let Err(e) = crate::daemon::process_inbound(&data_dir, &envelopes) {
                            tracing::warn!(error = %e, "nostr inbound process_inbound 실패");
                        }
                    }
                }
            }
        }
        // 종료 직전 잔여 drain
        drain_into_batch(&receiver_secret_arc, &mut rx, &mut batch);
        if !batch.is_empty() {
            let _ = crate::daemon::process_inbound(&data_dir, &batch);
        }
        source.shutdown().await;
    });
    Ok(handle)
}

/// rx 에 누적된 (sender_pk, ciphertext) 들을 복호 + JSON deserialize → batch 추가.
/// 복호 실패 / JSON 파싱 실패는 try_unwrap_with_warn 가 WARN 로그 + drop.
fn drain_into_batch(
    receiver_secret: &openxgram_nostr::SecretKey,
    rx: &mut mpsc::UnboundedReceiver<(openxgram_nostr::PublicKey, String)>,
    batch: &mut Vec<Envelope>,
) {
    while let Ok((sender_pk, ciphertext)) = rx.try_recv() {
        let Some(plaintext) = try_unwrap_with_warn(receiver_secret, &sender_pk, &[], &ciphertext)
        else {
            continue;
        };
        match serde_json::from_str::<Envelope>(&plaintext) {
            Ok(env) => batch.push(env),
            Err(e) => {
                tracing::warn!(error = %e, "envelope JSON deserialize 실패 — drop");
            }
        }
    }
}

pub fn open_db_for_inbound(data_dir: &std::path::Path) -> Result<openxgram_db::Db> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_core::paths::db_path;
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 환경변수 조작 테스트 — 병렬 실행 시 race 발생하므로 단일 테스트로 통합 + Mutex.
    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn config_from_env_csv_default_and_none_paths() {
        let _g = ENV_LOCK.lock().unwrap();
        // 1. 미설정 → None
        std::env::remove_var("XGRAM_NOSTR_RELAYS");
        std::env::remove_var("XGRAM_NOSTR_POLL_SECS");
        assert!(NostrInboundConfig::from_env(PathBuf::from("/tmp")).is_none());

        // 2. CSV + custom poll
        std::env::set_var("XGRAM_NOSTR_RELAYS", "ws://a, ws://b ,, ws://c");
        std::env::set_var("XGRAM_NOSTR_POLL_SECS", "5");
        let cfg = NostrInboundConfig::from_env(PathBuf::from("/tmp")).unwrap();
        assert_eq!(cfg.relays, vec!["ws://a", "ws://b", "ws://c"]);
        assert_eq!(cfg.poll_interval, Duration::from_secs(5));

        // 3. default poll
        std::env::set_var("XGRAM_NOSTR_RELAYS", "ws://x");
        std::env::remove_var("XGRAM_NOSTR_POLL_SECS");
        let cfg = NostrInboundConfig::from_env(PathBuf::from("/tmp")).unwrap();
        assert_eq!(cfg.poll_interval, Duration::from_secs(DEFAULT_POLL_SECS));

        std::env::remove_var("XGRAM_NOSTR_RELAYS");
    }

    #[tokio::test]
    async fn shutdown_signal_terminates_processor() {
        use openxgram_keystore::Keystore;
        // MockRelay 구동 — 실제 publish 없이 shutdown 동작만 확인
        let relay = nostr_relay_builder::MockRelay::run().await.unwrap();
        let url = relay.url();

        let tmp = tempfile::tempdir().unwrap();
        let ks = openxgram_keystore::FsKeystore::new(tmp.path());
        ks.create("daemon", "pw").unwrap();
        let master = ks.load("daemon", "pw").unwrap();

        let (sd_tx, sd_rx) = tokio::sync::watch::channel(false);
        let cfg = NostrInboundConfig {
            data_dir: tmp.path().to_path_buf(),
            relays: vec![url],
            poll_interval: Duration::from_millis(100),
        };
        let handle = spawn_nostr_inbound_processor(cfg, master, sd_rx).await.unwrap();

        // 200ms 후 shutdown
        tokio::time::sleep(Duration::from_millis(200)).await;
        sd_tx.send(true).unwrap();

        // 1초 내 종료
        let r = tokio::time::timeout(Duration::from_secs(1), handle).await;
        assert!(r.is_ok(), "shutdown 신호 후 미종료");
    }
}
