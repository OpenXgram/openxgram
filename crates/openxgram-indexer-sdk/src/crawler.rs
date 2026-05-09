//! 4.2.1.2 — ENS records 크롤러.
//!
//! `RecordResolver` trait 으로 alloy provider / mock 둘 다 지원.
//! 인덱서는 handle 목록을 받아서 records 를 batch 수집.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsRecord {
    pub handle: String,
    pub key: String,
    pub value: String,
}

#[async_trait]
pub trait RecordResolver: Send + Sync {
    async fn resolve(&self, handle: &str, key: &str) -> Result<Option<String>>;
}

/// 테스트/오프라인 — 메모리 맵으로 응답.
#[derive(Debug, Clone, Default)]
pub struct MockEnsResolver {
    pub data: Arc<Mutex<HashMap<(String, String), String>>>,
}

impl MockEnsResolver {
    pub fn set(&self, handle: &str, key: &str, value: &str) {
        self.data
            .lock()
            .unwrap()
            .insert((handle.into(), key.into()), value.into());
    }
}

#[async_trait]
impl RecordResolver for MockEnsResolver {
    async fn resolve(&self, handle: &str, key: &str) -> Result<Option<String>> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .get(&(handle.to_string(), key.to_string()))
            .cloned())
    }
}

pub struct EnsCrawler<R: RecordResolver> {
    pub resolver: R,
    pub keys: Vec<String>,
}

impl<R: RecordResolver> EnsCrawler<R> {
    pub fn new(resolver: R) -> Self {
        Self {
            resolver,
            keys: vec![
                "xgram.handle".into(),
                "xgram.daemon".into(),
                "xgram.pubkey".into(),
                "xgram.bio".into(),
                "xgram.visibility".into(),
            ],
        }
    }

    pub async fn crawl(&self, handles: &[&str]) -> Result<Vec<EnsRecord>> {
        let mut out = Vec::new();
        for h in handles {
            for k in &self.keys {
                if let Some(v) = self.resolver.resolve(h, k).await? {
                    out.push(EnsRecord {
                        handle: (*h).into(),
                        key: k.clone(),
                        value: v,
                    });
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_resolver_returns_set_values() {
        let m = MockEnsResolver::default();
        m.set("starian.base.eth", "xgram.handle", "starian");
        m.set("starian.base.eth", "xgram.bio", "autonomous AI agent");

        let c = EnsCrawler::new(m);
        let recs = c.crawl(&["starian.base.eth"]).await.unwrap();
        assert_eq!(recs.len(), 2, "set 한 두 key 만 반환");
        assert!(recs.iter().any(|r| r.key == "xgram.handle" && r.value == "starian"));
    }

    #[tokio::test]
    async fn unknown_handle_yields_empty() {
        let m = MockEnsResolver::default();
        let c = EnsCrawler::new(m);
        let recs = c.crawl(&["unknown.base.eth"]).await.unwrap();
        assert_eq!(recs.len(), 0);
    }
}
