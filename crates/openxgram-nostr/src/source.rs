//! NostrSource — relay subscribe + 이벤트 콜백 (PRD-NOSTR-04).
//!
//! 단일 NostrSource 가 여러 relay 동시 subscribe. nostr-sdk Client 기반.
//! 이벤트 수신 → callback (sync). 데몬 polling task 가 spawn_listener 결과를
//! L0 message store 또는 process_inbound 로 라우팅.

use crate::{NostrError, Result};
use nostr::{Event, Filter, SubscriptionId};
use nostr_sdk::{Client, Keys, RelayPoolNotification};
use std::sync::Arc;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct NostrSource {
    client: Client,
}

impl NostrSource {
    pub fn new(keys: Keys) -> Self {
        Self {
            client: Client::new(keys),
        }
    }

    pub async fn add_relays<I, S>(&self, urls: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for u in urls {
            self.client
                .add_relay(u.as_ref())
                .await
                .map_err(|e| NostrError::Nostr(e.to_string()))?;
        }
        self.client.connect().await;
        Ok(())
    }

    pub async fn subscribe(&self, filter: Filter) -> Result<SubscriptionId> {
        let out = self
            .client
            .subscribe(filter, None)
            .await
            .map_err(|e| NostrError::Nostr(e.to_string()))?;
        Ok(out.val)
    }

    /// 백그라운드 task 로 이벤트 수신. Shutdown notification 까지 실행.
    /// callback 은 sync — 무거운 처리는 자체 spawn 권장.
    pub fn spawn_listener<F>(&self, callback: F) -> JoinHandle<()>
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let cb = Arc::new(callback);
        let mut rx = self.client.notifications();
        tokio::spawn(async move {
            while let Ok(notif) = rx.recv().await {
                match notif {
                    RelayPoolNotification::Event { event, .. } => cb(*event),
                    RelayPoolNotification::Shutdown => break,
                    _ => {}
                }
            }
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn shutdown(self) {
        let _ = self.client.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keys_from_master, NostrKind, NostrSink};
    use nostr::{EventBuilder, Kind};
    use nostr_relay_builder::MockRelay;
    use openxgram_keystore::{FsKeystore, Keystore};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;
    use tokio::time::{timeout, Duration};

    fn make_keys() -> Keys {
        let tmp = tempdir().unwrap();
        let ks = FsKeystore::new(tmp.path());
        let _ = ks.create("t", "p").unwrap();
        let m = ks.load("t", "p").unwrap();
        keys_from_master(&m).unwrap()
    }

    #[tokio::test]
    async fn subscribe_and_receive_event() {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url();

        // publisher
        let sink = NostrSink::new(make_keys());
        sink.add_relays([url.clone()]).await.unwrap();

        // subscriber — 다른 keypair 로
        let source = NostrSource::new(Keys::generate());
        source.add_relays([url.clone()]).await.unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let handle = source.spawn_listener(move |_event| {
            c.fetch_add(1, Ordering::SeqCst);
        });

        // L0Message kind 만 구독
        let filter = Filter::new().kind(Kind::from(30500u16));
        source.subscribe(filter).await.unwrap();

        // publish
        sink.publish(NostrKind::L0Message, "ping", Some("s1"), vec![])
            .await
            .unwrap();

        // 콜백 트리거 대기 — 최대 3초
        let _ = timeout(Duration::from_secs(3), async {
            while counter.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        assert!(counter.load(Ordering::SeqCst) >= 1);

        sink.shutdown().await;
        source.shutdown().await;
        let _ = handle.await;
    }

    #[tokio::test]
    async fn shutdown_stops_listener() {
        let relay = MockRelay::run().await.unwrap();
        let source = NostrSource::new(Keys::generate());
        source.add_relays([relay.url()]).await.unwrap();
        let handle = source.spawn_listener(|_| {});
        source.shutdown().await;
        // listener 는 Shutdown notification 수신 후 종료
        let r = timeout(Duration::from_secs(3), handle).await;
        assert!(r.is_ok(), "listener did not exit on shutdown");
    }

    #[tokio::test]
    async fn filter_kind_excludes_other_kinds() {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url();

        let sink = NostrSink::new(make_keys());
        sink.add_relays([url.clone()]).await.unwrap();

        let source = NostrSource::new(Keys::generate());
        source.add_relays([url.clone()]).await.unwrap();

        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let handle = source.spawn_listener(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        });

        // 30500 (L0Message) 만 구독
        source
            .subscribe(Filter::new().kind(Kind::from(30500u16)))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        // 30100 publish — 구독 kind 가 아니므로 callback 호출 안 됨
        let pub_keys = sink.client().signer().await.unwrap();
        let other = EventBuilder::new(Kind::from(30100u16), "trait")
            .sign(&pub_keys)
            .await
            .unwrap();
        sink.client().send_event(&other).await.unwrap();

        // 1초 대기 — 콜백은 0회 유지
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        sink.shutdown().await;
        source.shutdown().await;
        let _ = handle.await;
    }
}
