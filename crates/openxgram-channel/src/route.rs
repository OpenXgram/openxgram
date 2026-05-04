//! Route engine — Starian channel-mcp 호환 동작.
//!
//! 두 진입점:
//! - `send_to_platform(platform, channel_id, text, reply_to?)`
//!   → AdapterRegistry 에서 (kind=platform, label=channel_id) 조회 후 전송.
//!   channel_id 미일치 시 같은 kind 의 첫 번째 어댑터로 fallback.
//! - `send_message(to, summary, type, details?)`
//!   → PeerRegistry.get(role=to) → default_platform / channel_id 기반 라우팅.

use openxgram_core::time::kst_now;
use serde::{Deserialize, Serialize};

use crate::adapter::{AdapterKind, AdapterRegistry};
use crate::peer::PeerRegistry;
use crate::{ChannelError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteResult {
    pub platform: String,
    pub channel: String,
    pub bytes: usize,
    pub at: String,
}

#[derive(Clone)]
pub struct RouteEngine {
    pub adapters: AdapterRegistry,
    pub peers: PeerRegistry,
}

impl RouteEngine {
    pub fn new(adapters: AdapterRegistry, peers: PeerRegistry) -> Self {
        Self { adapters, peers }
    }

    pub async fn send_to_platform(
        &self,
        platform: &str,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<RouteResult> {
        let kind = AdapterKind::parse(platform)?;
        let entry = match self.adapters.get(kind, channel_id).await {
            Some(e) => e,
            None => self.adapters.first_of(kind).await.ok_or_else(|| {
                ChannelError::AdapterNotFound(format!("{}:{}", platform, channel_id))
            })?,
        };
        let body = match reply_to {
            Some(r) if !r.is_empty() => format!("↪ {r}\n{text}"),
            _ => text.to_string(),
        };
        entry.send(&body).await?;
        Ok(RouteResult {
            platform: kind.as_str().into(),
            channel: entry.label.clone(),
            bytes: body.len(),
            at: kst_now().to_rfc3339(),
        })
    }

    pub async fn send_message(
        &self,
        to: &str,
        summary: &str,
        msg_type: &str,
        details: Option<&str>,
    ) -> Result<RouteResult> {
        let peer = self.peers.get(to).await?;
        let head = format!("[{}] @{} — {}", msg_type, to, summary);
        let body = match details {
            Some(d) if !d.is_empty() => format!("{head}\n{d}"),
            _ => head,
        };
        let entry = if let Some(ch) = peer.channel_id.as_deref() {
            match self.adapters.get(peer.default_platform, ch).await {
                Some(e) => e,
                None => self
                    .adapters
                    .first_of(peer.default_platform)
                    .await
                    .ok_or_else(|| {
                        ChannelError::AdapterNotFound(format!(
                            "{}:{}",
                            peer.default_platform.as_str(),
                            ch
                        ))
                    })?,
            }
        } else {
            self.adapters
                .first_of(peer.default_platform)
                .await
                .ok_or_else(|| {
                    ChannelError::AdapterNotFound(peer.default_platform.as_str().into())
                })?
        };
        entry.send(&body).await?;
        let now = kst_now().to_rfc3339();
        let _ = self.peers.touch(to, now.clone()).await;
        Ok(RouteResult {
            platform: peer.default_platform.as_str().into(),
            channel: entry.label.clone(),
            bytes: body.len(),
            at: now,
        })
    }
}
