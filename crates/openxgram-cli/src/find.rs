//! 4.4 — `xgram find @<handle> [--indexer URL]`. 핸들 → records (ENS resolver) 또는 indexer 검색.

use anyhow::{bail, Context, Result};

use crate::identity_handle::HandleResolver;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FindOpts {
    pub query: String,
    /// 사용자가 지정한 인덱서 URL (4.4.1). None 이면 manifest default (4.4.2).
    pub indexer: Option<String>,
}

pub async fn run_find(opts: FindOpts) -> Result<()> {
    let q = opts.query.trim_start_matches('@').to_string();
    if q.is_empty() {
        bail!("usage: xgram find @<handle>[.base.eth] [--indexer URL]");
    }

    if let Some(url) = opts.indexer {
        return run_find_via_indexer(&url, &q).await;
    }

    // 1) directory cache 먼저 — 핸들 → channels (e 작업 통합)
    let chans = crate::channels::directory_lookup(&q).unwrap_or_default();
    if !chans.is_empty() {
        println!("@{q} channels (directory cache):");
        for c in &chans {
            println!("  {:<12} {:<40} {}", c.kind, c.address, c.visibility);
        }
        return Ok(());
    }

    // 2) cache miss — ENS resolver (RPC 있을 때만 실 호출, 없으면 dry-run 안내)
    let resolver = HandleResolver::new();
    let records = resolver.resolve_with(&q, |handle| {
        if std::env::var("XGRAM_BASE_RPC").is_err() {
            eprintln!("[find] XGRAM_BASE_RPC 미설정 — onchain 조회 skip (dry-run for {handle})");
            Ok(HashMap::new())
        } else {
            eprintln!("[find] RPC 감지 — alloy resolver 호출 (다음 PR 통합)");
            Ok(HashMap::new())
        }
    })?;

    if records.is_empty() {
        println!("(no entry for @{q} — `xgram directory set @{q} <channels_json>` 또는 RPC 설정 필요)");
    } else {
        println!("ENS records for @{q}:");
        for (k, v) in records {
            println!("  {k} = {v}");
        }
    }
    Ok(())
}

async fn run_find_via_indexer(url: &str, query: &str) -> Result<()> {
    let url = format!(
        "{}/search?q={}",
        url.trim_end_matches('/'),
        urlencoded(query)
    );
    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("indexer GET {url}"))?;
    if !resp.status().is_success() {
        bail!("indexer HTTP {}", resp.status());
    }
    let body = resp.text().await.context("indexer body")?;
    println!("{body}");
    Ok(())
}

fn urlencoded(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoded_handles_korean() {
        // bytes() 가 UTF-8 byte sequence 를 그대로 percent-encode
        let enc = urlencoded("starian.base.eth");
        assert_eq!(enc, "starian.base.eth");
        let enc2 = urlencoded("a@b");
        assert!(enc2.contains("%40"));
    }
}
