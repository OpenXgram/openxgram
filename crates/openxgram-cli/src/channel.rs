//! xgram channel — 내장 Channel MCP 서버 + 클라이언트.
//!
//! 서브명령:
//! - `xgram channel serve --bind 127.0.0.1:7250 [--auth-token X]`
//!   OpenXgram 자체를 channel-mcp 호환 HTTP 서버로 기동.
//!   기본 어댑터: env 에 webhook/bot 토큰이 있으면 자동 등록.
//!   DISCORD_WEBHOOK_URL → discord:default
//!   TELEGRAM_BOT_TOKEN + TELEGRAM_CHAT_ID → telegram:default
//!   SLACK_WEBHOOK_URL → slack:default
//! - `xgram channel send --to-role <role> --text <txt>` (또는 platform 직접)
//!   기동 중인 서버에 클라이언트로 호출.
//! - `xgram channel list-adapters` / `list-peers` — 서버에 조회.
//!
//! 절대 규칙: 0.0.0.0 바인딩 금지 (server.rs 의 enforce_loopback 으로 강제).

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_adapter::{DiscordWebhookAdapter, TelegramBotAdapter};
use openxgram_channel::{
    serve, AdapterEntry, AdapterKind, AdapterRegistry, ChannelPeer, KakaoTalkPlaceholderAdapter,
    PeerRegistry, RouteEngine, ServerConfig, SlackWebhookAdapter,
};

#[derive(Debug, Clone)]
pub enum ChannelAction {
    Serve {
        bind: String,
        auth_token: Option<String>,
    },
    Send {
        server: String,
        auth_token: Option<String>,
        to_role: Option<String>,
        platform: Option<String>,
        channel_id: Option<String>,
        text: String,
        reply_to: Option<String>,
        msg_type: String,
    },
    ListAdapters {
        server: String,
        auth_token: Option<String>,
    },
    ListPeers {
        server: String,
        auth_token: Option<String>,
    },
}

pub async fn run(action: ChannelAction) -> Result<()> {
    match action {
        ChannelAction::Serve { bind, auth_token } => serve_cmd(&bind, auth_token).await,
        ChannelAction::Send {
            server,
            auth_token,
            to_role,
            platform,
            channel_id,
            text,
            reply_to,
            msg_type,
        } => {
            send_cmd(
                &server,
                auth_token.as_deref(),
                to_role.as_deref(),
                platform.as_deref(),
                channel_id.as_deref(),
                &text,
                reply_to.as_deref(),
                &msg_type,
            )
            .await
        }
        ChannelAction::ListAdapters { server, auth_token } => {
            list_cmd(&server, auth_token.as_deref(), "list_adapters").await
        }
        ChannelAction::ListPeers { server, auth_token } => {
            list_cmd(&server, auth_token.as_deref(), "list_peers").await
        }
    }
}

async fn serve_cmd(bind: &str, auth_token: Option<String>) -> Result<()> {
    let addr: SocketAddr = bind.parse().context("--bind 주소 파싱 실패")?;

    let adapters = AdapterRegistry::new();
    register_default_adapters(&adapters).await?;

    let peers = PeerRegistry::new();
    register_default_peers(&peers).await;

    let route = RouteEngine::new(adapters.clone(), peers.clone());
    let cfg = ServerConfig {
        bind: addr,
        auth_token: auth_token.clone(),
    };
    let handle = serve(cfg, route)
        .await
        .map_err(|e| anyhow!("server 기동 실패: {e}"))?;

    println!("✓ OpenXgram Channel MCP 서버 기동");
    println!("  bind        : {}", handle.bound_addr);
    println!(
        "  auth        : {}",
        if auth_token.is_some() {
            "Bearer 토큰 필수"
        } else {
            "비활성 (loopback 만 허용)"
        }
    );
    let descs = adapters.list().await;
    println!("  adapters    : {}", descs.len());
    for d in &descs {
        println!("    - {}:{}", d.kind.as_str(), d.label);
    }
    let plist = peers.list().await;
    println!("  peers       : {}", plist.len());
    for p in &plist {
        println!(
            "    - {} → {}:{}",
            p.role,
            p.default_platform.as_str(),
            p.channel_id.as_deref().unwrap_or("(default)")
        );
    }
    println!();
    println!("Ctrl+C 로 종료");

    tokio::signal::ctrl_c()
        .await
        .context("Ctrl+C 시그널 등록 실패")?;
    println!("\n✓ 종료");
    Ok(())
}

/// env 에 자격이 있는 어댑터를 자동 등록. 데이터 외부 노출 0 — 사용자 자기 토큰만 사용.
async fn register_default_adapters(reg: &AdapterRegistry) -> Result<()> {
    if let Ok(url) = std::env::var("DISCORD_WEBHOOK_URL") {
        if !url.is_empty() {
            reg.register(AdapterEntry::new(
                AdapterKind::Discord,
                "default",
                Arc::new(DiscordWebhookAdapter::new(url)),
            ))
            .await;
        }
    }
    if let (Ok(token), Ok(chat)) = (
        std::env::var("TELEGRAM_BOT_TOKEN"),
        std::env::var("TELEGRAM_CHAT_ID"),
    ) {
        if !token.is_empty() && !chat.is_empty() {
            reg.register(AdapterEntry::new(
                AdapterKind::Telegram,
                "default",
                Arc::new(TelegramBotAdapter::new(token, chat)),
            ))
            .await;
        }
    }
    if let Ok(url) = std::env::var("SLACK_WEBHOOK_URL") {
        if !url.is_empty() {
            reg.register(AdapterEntry::new(
                AdapterKind::Slack,
                "default",
                Arc::new(SlackWebhookAdapter::new(url)),
            ))
            .await;
        }
    }
    if let Ok(channel) = std::env::var("KAKAOTALK_CHANNEL_ID") {
        if !channel.is_empty() {
            reg.register(AdapterEntry::new(
                AdapterKind::Kakaotalk,
                "default",
                Arc::new(KakaoTalkPlaceholderAdapter::new(channel)),
            ))
            .await;
        }
    }
    Ok(())
}

/// `XGRAM_CHANNEL_PEERS=role1:platform:channel,role2:...` 환경변수로 추가.
async fn register_default_peers(reg: &PeerRegistry) {
    if let Ok(spec) = std::env::var("XGRAM_CHANNEL_PEERS") {
        for entry in spec.split(',').filter(|s| !s.is_empty()) {
            let parts: Vec<&str> = entry.split(':').collect();
            if parts.len() < 2 {
                continue;
            }
            let Ok(kind) = AdapterKind::parse(parts[1]) else {
                continue;
            };
            reg.upsert(ChannelPeer {
                role: parts[0].to_string(),
                alias: parts[0].to_string(),
                default_platform: kind,
                channel_id: parts.get(2).map(|s| s.to_string()),
                note: None,
                last_seen: None,
            })
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_cmd(
    server: &str,
    auth_token: Option<&str>,
    to_role: Option<&str>,
    platform: Option<&str>,
    channel_id: Option<&str>,
    text: &str,
    reply_to: Option<&str>,
    msg_type: &str,
) -> Result<()> {
    let client = reqwest::Client::new();
    let (path, body) = if let Some(role) = to_role {
        (
            "/tools/send_message",
            serde_json::json!({ "to": role, "summary": text, "type": msg_type }),
        )
    } else {
        let p = platform.ok_or_else(|| anyhow!("--to-role 또는 --platform 필요"))?;
        let c = channel_id.ok_or_else(|| anyhow!("--channel-id 필요"))?;
        (
            "/tools/send_to_platform",
            serde_json::json!({
                "platform": p,
                "channel_id": c,
                "text": text,
                "reply_to": reply_to,
            }),
        )
    };
    let url = format!("{}{}", server.trim_end_matches('/'), path);
    let mut req = client.post(&url).json(&body);
    if let Some(t) = auth_token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.context("서버 요청 실패")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    if !status.is_success() {
        bail!("서버 오류 {}: {}", status, body);
    }
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn list_cmd(server: &str, auth_token: Option<&str>, tool: &str) -> Result<()> {
    let url = format!("{}/tools/{}", server.trim_end_matches('/'), tool);
    let mut req = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({}));
    if let Some(t) = auth_token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.context("서버 요청 실패")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    if !status.is_success() {
        bail!("서버 오류 {}: {}", status, body);
    }
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}
