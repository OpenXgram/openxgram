//! NIP-65 relay list (kind 10002) — peer 의 read/write relay 자가 광고 (PRD-NOSTR-08).
//!
//! - publish_relay_list: 자기 master keys 로 kind 10002 event 발행.
//! - fetch_peer_relays: peer 의 pubkey 로 kind 10002 최신 event 조회 → URL 목록 반환.
//! - dedup_relays: 기존 + 신규 합치면서 중복 제거.

use crate::{NostrError, Result};
use nostr::nips::nip65;
use nostr::{EventBuilder, Filter, Keys, Kind, PublicKey, RelayUrl};
use nostr_sdk::Client;
use std::collections::HashSet;
use std::time::Duration;

pub use nostr::nips::nip65::RelayMetadata;

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// (url, Some(Read)) | (url, Some(Write)) | (url, None) — None 은 read+write 의미.
pub type RelayEntry = (RelayUrl, Option<RelayMetadata>);

/// kind 10002 event 발행. addressable 이 아닌 replaceable 이라 d-tag 불필요.
pub async fn publish_relay_list(
    keys: &Keys,
    relays: Vec<RelayEntry>,
    publish_to: &[String],
) -> Result<nostr::EventId> {
    let client = Client::new(keys.clone());
    for r in publish_to {
        client
            .add_relay(r)
            .await
            .map_err(|e| NostrError::Nostr(e.to_string()))?;
    }
    client.connect().await;
    let builder = EventBuilder::relay_list(relays);
    let out = client
        .send_event_builder(builder)
        .await
        .map_err(|e| NostrError::Nostr(e.to_string()))?;
    let id = *out.id();
    let _ = client.shutdown().await;
    Ok(id)
}

/// peer 의 가장 최신 kind 10002 event 1개 조회 → 파싱된 RelayEntry 목록.
pub async fn fetch_peer_relays(
    author: &PublicKey,
    query_relays: &[String],
) -> Result<Vec<RelayEntry>> {
    let client = Client::new(Keys::generate());
    for r in query_relays {
        client
            .add_relay(r)
            .await
            .map_err(|e| NostrError::Nostr(e.to_string()))?;
    }
    client.connect().await;
    let filter = Filter::new().kind(Kind::RelayList).author(*author).limit(1);
    let events = client
        .fetch_events(filter, FETCH_TIMEOUT)
        .await
        .map_err(|e| NostrError::Nostr(e.to_string()))?;
    let _ = client.shutdown().await;
    let event = match events.first() {
        Some(e) => e,
        None => return Ok(Vec::new()),
    };
    let list: Vec<RelayEntry> = nip65::extract_relay_list(event)
        .map(|(u, m)| (u.clone(), *m))
        .collect();
    Ok(list)
}

/// 기존 relay 집합에 신규를 합치면서 url 기준 중복 제거.
/// metadata 충돌 시 신규 우선.
pub fn dedup_relays(existing: Vec<RelayEntry>, incoming: Vec<RelayEntry>) -> Vec<RelayEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<RelayEntry> = Vec::new();
    // 신규 우선
    for (url, md) in incoming.into_iter().chain(existing) {
        let key = url.to_string();
        if seen.insert(key) {
            out.push((url, md));
        }
    }
    out
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

    #[test]
    fn dedup_keeps_first_occurrence() {
        let url1 = RelayUrl::parse("wss://a.example.com").unwrap();
        let url2 = RelayUrl::parse("wss://b.example.com").unwrap();
        let existing = vec![
            (url1.clone(), Some(RelayMetadata::Read)),
            (url2.clone(), None),
        ];
        let incoming = vec![
            (url1.clone(), Some(RelayMetadata::Write)), // 동일 URL — 신규 metadata 우선
            (
                RelayUrl::parse("wss://c.example.com").unwrap(),
                Some(RelayMetadata::Read),
            ),
        ];
        let merged = dedup_relays(existing, incoming);
        assert_eq!(merged.len(), 3);
        // url1 의 metadata 가 신규(Write) 인지 확인
        let m1 = merged.iter().find(|(u, _)| u == &url1).unwrap().1;
        assert_eq!(m1, Some(RelayMetadata::Write));
    }

    #[tokio::test]
    async fn publish_then_fetch_round_trip() {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url();

        let keys = make_keys();
        let entries = vec![
            (
                RelayUrl::parse("wss://relay.a.example.com").unwrap(),
                Some(RelayMetadata::Read),
            ),
            (
                RelayUrl::parse("wss://relay.b.example.com").unwrap(),
                Some(RelayMetadata::Write),
            ),
        ];
        let _id = publish_relay_list(&keys, entries.clone(), std::slice::from_ref(&url))
            .await
            .unwrap();

        let fetched = fetch_peer_relays(&keys.public_key(), std::slice::from_ref(&url))
            .await
            .unwrap();
        assert_eq!(fetched.len(), 2);
        let urls: HashSet<String> = fetched.iter().map(|(u, _)| u.to_string()).collect();
        assert!(urls.contains("wss://relay.a.example.com"));
        assert!(urls.contains("wss://relay.b.example.com"));
    }

    #[tokio::test]
    async fn fetch_returns_empty_when_no_event() {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url();
        let keys = make_keys();
        let fetched = fetch_peer_relays(&keys.public_key(), std::slice::from_ref(&url))
            .await
            .unwrap();
        assert!(fetched.is_empty());
    }
}
