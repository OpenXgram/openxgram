//! Self-host relay — `xgram relay serve` (PRD-NOSTR-06).
//!
//! nostr-relay-builder LocalRelay 위에 얇은 어댑터. NIP-13 PoW 옵션, bind addr
//! 설정. 데몬 라이프사이클 통합 (shutdown handle 노출).

use crate::{NostrError, Result};
use nostr_relay_builder::{LocalRelay, RelayBuilder};
use std::net::IpAddr;

pub const DEFAULT_RELAY_PORT: u16 = 7400;

#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub addr: IpAddr,
    pub port: u16,
    pub min_pow: Option<u8>, // NIP-13 PoW difficulty (0~32)
    pub max_connections: Option<usize>,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1".parse().unwrap(),
            port: DEFAULT_RELAY_PORT,
            min_pow: None,
            max_connections: None,
        }
    }
}

/// 자체 호스팅 nostr relay. shutdown 호출까지 백그라운드에서 실행.
#[derive(Debug, Clone)]
pub struct NostrRelay {
    inner: LocalRelay,
}

impl NostrRelay {
    pub async fn serve(config: RelayConfig) -> Result<Self> {
        let mut builder = RelayBuilder::default().addr(config.addr).port(config.port);
        if let Some(d) = config.min_pow {
            builder = builder.min_pow(d);
        }
        if let Some(m) = config.max_connections {
            builder = builder.max_connections(m);
        }
        let inner = LocalRelay::run(builder)
            .await
            .map_err(|e| NostrError::Nostr(e.to_string()))?;
        Ok(Self { inner })
    }

    pub fn url(&self) -> String {
        self.inner.url()
    }

    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{keys_from_master, NostrKind, NostrSink};
    use openxgram_keystore::{FsKeystore, Keystore};
    use tempfile::tempdir;

    fn make_keys() -> nostr::Keys {
        let tmp = tempdir().unwrap();
        let ks = FsKeystore::new(tmp.path());
        let _ = ks.create("t", "p").unwrap();
        let m = ks.load("t", "p").unwrap();
        keys_from_master(&m).unwrap()
    }

    #[tokio::test]
    async fn serve_localhost_random_port_publish_round_trip() {
        let cfg = RelayConfig {
            addr: "127.0.0.1".parse().unwrap(),
            port: 0, // OS 가 빈 포트 배정
            min_pow: None,
            max_connections: None,
        };
        let relay = NostrRelay::serve(cfg).await.unwrap();
        let url = relay.url();
        assert!(url.starts_with("ws://127.0.0.1:"));

        // 자체 relay 에 publish 가능 검증
        let sink = NostrSink::new(make_keys());
        sink.add_relays([url]).await.unwrap();
        let id = sink
            .publish(NostrKind::L0Message, "self-host", Some("d-1"), vec![])
            .await
            .unwrap();
        assert!(!id.to_hex().is_empty());

        sink.shutdown().await;
        relay.shutdown();
    }

    #[tokio::test]
    async fn serve_with_pow_threshold_starts() {
        let cfg = RelayConfig {
            addr: "127.0.0.1".parse().unwrap(),
            port: 0,
            min_pow: Some(0), // 0 = 사실상 비활성, 시작만 검증
            max_connections: Some(10),
        };
        let relay = NostrRelay::serve(cfg).await.unwrap();
        assert!(relay.url().contains("127.0.0.1"));
        relay.shutdown();
    }
}
