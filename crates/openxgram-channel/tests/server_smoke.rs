//! Smoke test — axum 서버 기동 → mock adapter 호출 검증.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use openxgram_channel::{
    serve, AdapterEntry, AdapterKind, AdapterRegistry, ChannelAdapter, ChannelPeer, PeerRegistry,
    RouteEngine, ServerConfig,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct MockAdapter {
    sent: Mutex<Vec<String>>,
}

#[async_trait]
impl ChannelAdapter for MockAdapter {
    async fn send_text(&self, text: &str) -> openxgram_channel::Result<()> {
        self.sent.lock().await.push(text.to_string());
        Ok(())
    }
}

async fn setup() -> (SocketAddr, Arc<MockAdapter>, Arc<MockAdapter>) {
    let adapters = AdapterRegistry::new();
    let discord_mock = Arc::new(MockAdapter::default());
    let slack_mock = Arc::new(MockAdapter::default());
    adapters
        .register(AdapterEntry::new(
            AdapterKind::Discord,
            "alpha",
            discord_mock.clone(),
        ))
        .await;
    adapters
        .register(AdapterEntry::new(
            AdapterKind::Slack,
            "beta",
            slack_mock.clone(),
        ))
        .await;

    let peers = PeerRegistry::new();
    peers
        .upsert(ChannelPeer {
            role: "eno".into(),
            alias: "Eno".into(),
            default_platform: AdapterKind::Discord,
            channel_id: Some("alpha".into()),
            note: None,
            last_seen: None,
        })
        .await;

    let route = RouteEngine::new(adapters.clone(), peers.clone());
    let cfg = ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        auth_token: Some("secret".into()),
    };
    let handle = serve(cfg, route).await.expect("serve");
    (handle.bound_addr, discord_mock, slack_mock)
}

#[tokio::test]
async fn send_to_platform_routes_to_adapter() {
    let (addr, discord, _slack) = setup().await;
    let url = format!("http://{}/tools/send_to_platform", addr);
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth("secret")
        .json(&serde_json::json!({
            "platform": "discord",
            "channel_id": "alpha",
            "text": "hi",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    let sent = discord.sent.lock().await;
    assert_eq!(sent.as_slice(), &["hi".to_string()]);
}

#[tokio::test]
async fn send_message_uses_peer_default_platform() {
    let (addr, discord, _slack) = setup().await;
    let url = format!("http://{}/tools/send_message", addr);
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth("secret")
        .json(&serde_json::json!({
            "to": "eno",
            "summary": "build green",
            "type": "result",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let sent = discord.sent.lock().await;
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("[result] @eno — build green"));
}

#[tokio::test]
async fn list_adapters_and_peers_round_trip() {
    let (addr, _d, _s) = setup().await;
    let client = reqwest::Client::new();

    let r: serde_json::Value = client
        .post(format!("http://{}/tools/list_adapters", addr))
        .bearer_auth("secret")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(r["adapters"].as_array().unwrap().len(), 2);

    let r: serde_json::Value = client
        .post(format!("http://{}/tools/list_peers", addr))
        .bearer_auth("secret")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(r["peers"][0]["role"], "eno");
}

#[tokio::test]
async fn auth_token_required() {
    let (addr, _d, _s) = setup().await;
    let resp = reqwest::Client::new()
        .post(format!("http://{}/tools/list_adapters", addr))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn external_bind_rejected() {
    let adapters = AdapterRegistry::new();
    let peers = PeerRegistry::new();
    let route = RouteEngine::new(adapters, peers);
    let cfg = ServerConfig {
        bind: "0.0.0.0:0".parse().unwrap(),
        auth_token: None,
    };
    let res = serve(cfg, route).await;
    assert!(res.is_err(), "0.0.0.0 binding must be rejected");
}
