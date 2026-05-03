//! NostrSink — relay 로 event 발행 (PRD-NOSTR-03).
//!
//! 단일 NostrSink 가 여러 relay 동시 publish. nostr-sdk Client 기반.
//! relay 추가 → connect → publish 의 3-step.

use crate::{NostrError, NostrKind, Result};
use nostr::{EventBuilder, EventId, Keys, Kind, Tag};
use nostr_sdk::Client;

#[derive(Debug, Clone)]
pub struct NostrSink {
    client: Client,
}

impl NostrSink {
    /// keys 로 서명자 등록한 client 생성. relay 는 별도 추가 필요.
    pub fn new(keys: Keys) -> Self {
        Self {
            client: Client::new(keys),
        }
    }

    /// relay URL 추가 + 연결. 실패 시 첫 에러 반환.
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

    /// kind + content + addressable id + tags 로 EventBuilder 빌드 후
    /// 모든 WRITE relay 에 broadcast. 1개라도 성공하면 EventId 반환.
    pub async fn publish(
        &self,
        kind: NostrKind,
        content: &str,
        addressable_id: Option<&str>,
        extra_tags: Vec<Tag>,
    ) -> Result<EventId> {
        let mut tags = Vec::new();
        if let Some(d) = addressable_id {
            tags.push(Tag::identifier(d));
        }
        tags.extend(extra_tags);
        let builder = EventBuilder::new(Kind::from(kind), content).tags(tags);
        let output = self
            .client
            .send_event_builder(builder)
            .await
            .map_err(|e| NostrError::Nostr(e.to_string()))?;
        Ok(*output.id())
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
    use crate::keys_from_master;
    use nostr_relay_builder::MockRelay;
    use openxgram_keystore::{FsKeystore, Keystore};
    use tempfile::tempdir;

    fn make_keys() -> Keys {
        let tmp = tempdir().unwrap();
        let ks = FsKeystore::new(tmp.path());
        let _ = ks.create("t", "p").unwrap();
        let m = ks.load("t", "p").unwrap();
        keys_from_master(&m).unwrap()
    }

    #[tokio::test]
    async fn publish_to_mock_relay_succeeds() {
        let relay = MockRelay::run().await.unwrap();
        let keys = make_keys();
        let sink = NostrSink::new(keys);
        sink.add_relays([relay.url()]).await.unwrap();
        let id = sink
            .publish(NostrKind::L0Message, "hello mock", Some("sess-1"), vec![])
            .await
            .unwrap();
        assert!(!id.to_hex().is_empty());
        sink.shutdown().await;
    }

    #[tokio::test]
    async fn publish_to_multiple_relays_succeeds() {
        let r1 = MockRelay::run().await.unwrap();
        let r2 = MockRelay::run().await.unwrap();
        let keys = make_keys();
        let sink = NostrSink::new(keys);
        sink.add_relays([r1.url(), r2.url()]).await.unwrap();
        let id = sink
            .publish(NostrKind::L4Trait, "trait body", Some("trait-1"), vec![])
            .await
            .unwrap();
        assert!(!id.to_hex().is_empty());
        sink.shutdown().await;
    }

    #[tokio::test]
    async fn publish_without_relay_returns_error() {
        let keys = make_keys();
        let sink = NostrSink::new(keys);
        // relay 미추가 — send_event_builder 는 실패해야 함
        let res = sink
            .publish(NostrKind::L0Message, "no relay", None, vec![])
            .await;
        assert!(res.is_err());
    }
}
