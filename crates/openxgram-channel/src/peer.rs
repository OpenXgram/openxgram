//! Channel peer registry — alias ↔ role ↔ default platform 매핑 (in-memory).
//!
//! openxgram-peer crate 의 SQLite peers 는 머신 단위 envelope routing 을 담당.
//! 본 모듈은 그 위에 "이 role 에게 보내려면 어느 platform/channel 로 가야 하는가"
//! 라는 channel routing 메타를 얹는다.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::adapter::AdapterKind;
use crate::{ChannelError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPeer {
    /// 역할명 (master / eno / qua / pip / res ...) — channel-mcp `to` 와 일치.
    pub role: String,
    /// 사람 친화 alias.
    pub alias: String,
    /// 기본 platform — send_message(to=role) 시 라우팅 대상.
    pub default_platform: AdapterKind,
    /// platform 내부 channel/chat id (Discord channel_id, Telegram chat_id 등).
    pub channel_id: Option<String>,
    /// 보조 metadata (예: peer crate 의 alias).
    pub note: Option<String>,
    /// 마지막 전송 성공 시각 (RFC3339 KST).
    pub last_seen: Option<String>,
}

#[derive(Default, Clone)]
pub struct PeerRegistry {
    inner: Arc<RwLock<HashMap<String, ChannelPeer>>>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn upsert(&self, peer: ChannelPeer) {
        self.inner.write().await.insert(peer.role.clone(), peer);
    }

    pub async fn get(&self, role: &str) -> Result<ChannelPeer> {
        self.inner
            .read()
            .await
            .get(role)
            .cloned()
            .ok_or_else(|| ChannelError::PeerNotFound(role.to_string()))
    }

    pub async fn list(&self) -> Vec<ChannelPeer> {
        let mut out: Vec<_> = self.inner.read().await.values().cloned().collect();
        out.sort_by(|a, b| a.role.cmp(&b.role));
        out
    }

    pub async fn touch(&self, role: &str, when_rfc3339: String) -> Result<()> {
        let mut g = self.inner.write().await;
        let p = g
            .get_mut(role)
            .ok_or_else(|| ChannelError::PeerNotFound(role.to_string()))?;
        p.last_seen = Some(when_rfc3339);
        Ok(())
    }
}
