//! e-마무리-2 — `xgram send @<h> <body>` 다채널 자동 라우팅.
//!
//! 흐름:
//!   1. directory_lookup(@h) → channels
//!   2. pick_best (옵션: --kind discord / telegram / xgram-peer / ...)
//!   3. kind 별 transport 호출:
//!        - xgram-peer  → run_peer_send_with_conv (alias 가 peer 테이블에 있어야)
//!        - discord     → webhook URL POST
//!        - telegram    → bot sendMessage (XGRAM_TELEGRAM_BOT_TOKEN env)
//!        - 그 외       → unsupported (안내)

use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

use crate::channels::{pick_best, Channel};

#[derive(Debug, Clone)]
pub struct SendOpts {
    pub handle: String,
    pub body: String,
    /// 명시적으로 채널 종류 강제 (discord / telegram / xgram-peer / ...)
    pub prefer_kind: Option<String>,
    /// conversation_id 동봉 (옵션)
    pub conversation_id: Option<String>,
}

pub async fn run_send(data_dir: &Path, opts: SendOpts) -> Result<()> {
    let h = opts.handle.trim_start_matches('@').to_string();
    if h.is_empty() {
        bail!("usage: xgram send @<handle> <body>");
    }
    if opts.body.trim().is_empty() {
        bail!("body 비어있음");
    }
    let chans = crate::channels::directory_lookup(&h)?;
    if chans.is_empty() {
        bail!("@{h}: 채널 없음 — `xgram directory set @{h} <json>` 또는 `xgram find @{h}` 후 재시도");
    }
    let pick = pick_best(&chans, opts.prefer_kind.as_deref())
        .ok_or_else(|| anyhow!("public 채널 없음 (모두 private)"))?;
    eprintln!("[send] @{h} → {}:{}", pick.kind, pick.address);
    dispatch(data_dir, &pick, &opts.body, opts.conversation_id).await
}

async fn dispatch(
    data_dir: &Path,
    channel: &Channel,
    body: &str,
    conversation_id: Option<String>,
) -> Result<()> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .context("reqwest client")?;
    match channel.kind.as_str() {
        "xgram-peer" => {
            // address 가 peer alias 라고 가정 (또는 URL — peer_send 의 PeerStore 가 alias 로 lookup).
            // 본 PR 은 alias 우선 — directory channel 의 address 가 alias 로 들어오는 경우만 동작.
            let pw = std::env::var("XGRAM_KEYSTORE_PASSWORD")
                .context("XGRAM_KEYSTORE_PASSWORD env 필요 (xgram-peer 송신 서명)")?;
            crate::peer_send::run_peer_send_with_conv(
                data_dir,
                &channel.address,
                None,
                body,
                &pw,
                conversation_id,
            )
            .await
        }
        "discord" => {
            // address = webhook URL (또는 channel id — webhook URL 권장)
            if !channel.address.starts_with("https://") {
                bail!("discord channel 은 webhook URL 필요 (https://...) — got: {}", channel.address);
            }
            let resp = http
                .post(&channel.address)
                .json(&serde_json::json!({"content": body}))
                .send()
                .await
                .context("discord webhook POST")?;
            if !resp.status().is_success() {
                bail!("discord webhook HTTP {}", resp.status());
            }
            Ok(())
        }
        "telegram" => {
            let token = std::env::var("XGRAM_TELEGRAM_BOT_TOKEN")
                .context("XGRAM_TELEGRAM_BOT_TOKEN env 필요")?;
            let chat_id = &channel.address;
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let resp = http
                .post(&url)
                .json(&serde_json::json!({"chat_id": chat_id, "text": body}))
                .send()
                .await
                .context("telegram sendMessage")?;
            if !resp.status().is_success() {
                bail!("telegram HTTP {}", resp.status());
            }
            Ok(())
        }
        other => bail!("미지원 채널 종류: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_rejected() {
        let opts = SendOpts {
            handle: "x".into(),
            body: "".into(),
            prefer_kind: None,
            conversation_id: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(run_send(dir.path(), opts));
        assert!(res.is_err());
    }

    #[test]
    fn empty_handle_rejected() {
        let opts = SendOpts {
            handle: "@".into(),
            body: "x".into(),
            prefer_kind: None,
            conversation_id: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res = rt.block_on(run_send(dir.path(), opts));
        assert!(res.is_err());
    }
}
