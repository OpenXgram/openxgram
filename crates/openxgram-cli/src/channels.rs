//! e) 핸들 디렉터리 통합 — `xgram channels` 관리 + `xgram send @<h>` 다채널 라우팅.
//!
//! 채널 한 개 = `(kind, address)`. kind = discord / telegram / whatsapp / xgram-peer / nostr / email / ...
//! 본 노드의 등록된 채널 = `IdentitySection.channels` (manifest).
//! 다른 핸들의 채널 = directory 조회 (ENS records `xgram.channels` JSON 또는 local cache).

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use openxgram_core::paths::manifest_path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// "discord" | "telegram" | "whatsapp" | "xgram-peer" | "nostr" | "email" | ...
    pub kind: String,
    /// 그 채널의 식별자 (channel_id / chat_id / phone / URL / npub / email)
    pub address: String,
    /// 공개 여부 (public / unlisted / private)
    #[serde(default = "default_visibility")]
    pub visibility: String,
}

fn default_visibility() -> String {
    "public".into()
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ChannelsSection {
    #[serde(default)]
    channels: Vec<Channel>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ManifestWithChannels {
    #[serde(default)]
    channels: Option<Vec<Channel>>,
    #[serde(flatten)]
    other: HashMap<String, serde_json::Value>,
}

fn read_channels(data_dir: &Path) -> Result<Vec<Channel>> {
    let p = manifest_path(data_dir);
    if !p.exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&p)?;
    let m: ManifestWithChannels = serde_json::from_str(&raw).unwrap_or_default();
    Ok(m.channels.unwrap_or_default())
}

fn write_channels(data_dir: &Path, channels: &[Channel]) -> Result<()> {
    let p = manifest_path(data_dir);
    let raw = if p.exists() {
        std::fs::read_to_string(&p)?
    } else {
        "{}".into()
    };
    let mut m: ManifestWithChannels =
        serde_json::from_str(&raw).unwrap_or_else(|_| ManifestWithChannels::default());
    m.channels = Some(channels.to_vec());
    let pretty = serde_json::to_string_pretty(&m)?;
    std::fs::write(&p, pretty)?;
    Ok(())
}

pub fn channel_add(data_dir: &Path, kind: &str, address: &str, visibility: &str) -> Result<()> {
    if kind.trim().is_empty() || address.trim().is_empty() {
        bail!("kind / address 필수");
    }
    if !matches!(visibility, "public" | "unlisted" | "private") {
        bail!("visibility = public | unlisted | private");
    }
    let mut channels = read_channels(data_dir)?;
    if channels.iter().any(|c| c.kind == kind && c.address == address) {
        eprintln!("[channels] 이미 등록 — skip");
        return Ok(());
    }
    channels.push(Channel {
        kind: kind.into(),
        address: address.into(),
        visibility: visibility.into(),
    });
    write_channels(data_dir, &channels)?;
    eprintln!("[channels] 추가: {kind}:{address} ({visibility})");
    Ok(())
}

pub fn channel_remove(data_dir: &Path, kind: &str, address: &str) -> Result<()> {
    let mut channels = read_channels(data_dir)?;
    let before = channels.len();
    channels.retain(|c| !(c.kind == kind && c.address == address));
    if channels.len() == before {
        bail!("매칭 채널 없음: {kind}:{address}");
    }
    write_channels(data_dir, &channels)?;
    eprintln!("[channels] 제거: {kind}:{address}");
    Ok(())
}

pub fn channel_list(data_dir: &Path) -> Result<Vec<Channel>> {
    read_channels(data_dir)
}

/// 디렉터리 cache — 핸들 → channels.
/// 본 PR 은 local file `~/.xgram/directory.json` 만. 실 ENS resolver / openxgram.org 통합은 다음 단계.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DirectoryCache {
    pub entries: HashMap<String, Vec<Channel>>,
}

fn directory_path() -> Result<std::path::PathBuf> {
    Ok(crate::bot::xgram_root()?.join("directory.json"))
}

pub fn directory_load() -> Result<DirectoryCache> {
    let p = directory_path()?;
    if !p.exists() {
        return Ok(DirectoryCache::default());
    }
    let raw = std::fs::read_to_string(&p)?;
    let parsed: DirectoryCache = serde_json::from_str(&raw).unwrap_or_default();
    Ok(parsed)
}

pub fn directory_save(d: &DirectoryCache) -> Result<()> {
    let p = directory_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(d)?)?;
    Ok(())
}

/// 디렉터리 cache 에 핸들 추가 (수동 등록 — friend accept 시 자동으로 호출 가능).
pub fn directory_set(handle: &str, channels: Vec<Channel>) -> Result<()> {
    let mut d = directory_load()?;
    d.entries.insert(handle.into(), channels);
    directory_save(&d)?;
    Ok(())
}

/// 핸들 → channels lookup.
/// 1) directory cache 에서 먼저 찾음
/// 2) 없으면 (TODO) ENS resolver 호출 → 결과 캐시
/// 3) 없으면 빈 vec
pub fn directory_lookup(handle: &str) -> Result<Vec<Channel>> {
    let d = directory_load()?;
    Ok(d.entries.get(handle).cloned().unwrap_or_default())
}

/// 채널 우선순위 (best-fit 라우팅) — public 우선, 그 다음 kind 순.
/// 호출자가 첫 번째 (가장 적합한) channel 사용 권장.
pub fn pick_best(channels: &[Channel], prefer_kind: Option<&str>) -> Option<Channel> {
    let public: Vec<&Channel> = channels.iter().filter(|c| c.visibility != "private").collect();
    if public.is_empty() {
        return None;
    }
    if let Some(k) = prefer_kind {
        if let Some(c) = public.iter().find(|c| c.kind == k) {
            return Some((*c).clone());
        }
    }
    // 기본 선호 순서: xgram-peer (직통) → discord → telegram → whatsapp → 기타
    for kind in ["xgram-peer", "discord", "telegram", "whatsapp"] {
        if let Some(c) = public.iter().find(|c| c.kind == kind) {
            return Some((*c).clone());
        }
    }
    Some((*public.first().unwrap()).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_dir() -> tempfile::TempDir {
        let tmp = tempdir().unwrap();
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(&mp, "{}").unwrap();
        tmp
    }

    #[test]
    fn add_list_remove_round_trip() {
        let tmp = make_dir();
        let dir = tmp.path();
        channel_add(dir, "discord", "channel-id-123", "public").unwrap();
        channel_add(dir, "telegram", "@my_bot", "unlisted").unwrap();
        let list = channel_list(dir).unwrap();
        assert_eq!(list.len(), 2);
        channel_remove(dir, "discord", "channel-id-123").unwrap();
        let list2 = channel_list(dir).unwrap();
        assert_eq!(list2.len(), 1);
        assert_eq!(list2[0].kind, "telegram");
    }

    #[test]
    fn add_idempotent_on_duplicate() {
        let tmp = make_dir();
        let dir = tmp.path();
        channel_add(dir, "discord", "x", "public").unwrap();
        channel_add(dir, "discord", "x", "public").unwrap();
        assert_eq!(channel_list(dir).unwrap().len(), 1);
    }

    #[test]
    fn add_rejects_invalid_visibility() {
        let tmp = make_dir();
        let dir = tmp.path();
        assert!(channel_add(dir, "discord", "x", "secret").is_err());
    }

    #[test]
    fn remove_unknown_errors() {
        let tmp = make_dir();
        let dir = tmp.path();
        assert!(channel_remove(dir, "discord", "ghost").is_err());
    }

    #[test]
    fn pick_best_prefers_explicit_kind() {
        let chans = vec![
            Channel { kind: "telegram".into(), address: "t".into(), visibility: "public".into() },
            Channel { kind: "discord".into(), address: "d".into(), visibility: "public".into() },
        ];
        let pick = pick_best(&chans, Some("telegram")).unwrap();
        assert_eq!(pick.kind, "telegram");
    }

    #[test]
    fn pick_best_default_prefers_xgram_peer() {
        let chans = vec![
            Channel { kind: "discord".into(), address: "d".into(), visibility: "public".into() },
            Channel { kind: "xgram-peer".into(), address: "p".into(), visibility: "public".into() },
        ];
        let pick = pick_best(&chans, None).unwrap();
        assert_eq!(pick.kind, "xgram-peer");
    }

    #[test]
    fn pick_best_skips_private() {
        let chans = vec![
            Channel { kind: "discord".into(), address: "d".into(), visibility: "private".into() },
            Channel { kind: "telegram".into(), address: "t".into(), visibility: "public".into() },
        ];
        let pick = pick_best(&chans, None).unwrap();
        assert_eq!(pick.kind, "telegram");
    }

    #[test]
    fn pick_best_empty_public_returns_none() {
        let chans = vec![Channel {
            kind: "discord".into(),
            address: "x".into(),
            visibility: "private".into(),
        }];
        assert!(pick_best(&chans, None).is_none());
    }
}
