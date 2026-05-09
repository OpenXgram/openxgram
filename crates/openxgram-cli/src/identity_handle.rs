//! Phase 3 — 핸들 / 정체성 / 검색 / 친구 추가.
//!
//! 본 모듈은 chain-touching 부분 (Basenames registrar, ENS resolver onchain) 을
//! `Resolver` trait 으로 추상화. 실 RPC 는 `XGRAM_BASE_RPC` env 가 있을 때만 (alloy 의존).
//! 그 외엔 manifest 갱신 + 친구 요청 메시징만 동작.
//!
//! Friend-request flow (3.4) 는 chain 의존 0 — 메시지 magic prefix 로 구현.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use openxgram_core::paths::manifest_path;

pub const FRIEND_REQUEST_PREFIX: &str = "xgram-friend-request-v1";
pub const FRIEND_ACCEPT_PREFIX: &str = "xgram-friend-accept-v1";
pub const FRIEND_DENY_PREFIX: &str = "xgram-friend-deny-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Unlisted,
    Private,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Unlisted => "unlisted",
            Self::Private => "private",
        }
    }
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "public" => Ok(Self::Public),
            "unlisted" => Ok(Self::Unlisted),
            "private" => Ok(Self::Private),
            other => Err(anyhow!("invalid visibility: {other}")),
        }
    }
}

/// install-manifest.json 에 추가되는 identity 섹션 (3.1.2 / 3.5).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdentitySection {
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default)]
    pub claimed_at: Option<String>,
}

fn default_visibility() -> String {
    "public".into()
}

/// 3.1.1 — Basenames `claim @<handle>.base.eth`.
/// 본 함수: manifest 갱신 (영속) + (옵션) onchain 등록.
/// onchain 등록은 `XGRAM_BASE_RPC` + `XGRAM_BASE_PRIVATE_KEY` 둘 다 있어야 시도. 없으면 dry-run.
pub fn claim_handle(data_dir: &Path, handle: &str) -> Result<IdentitySection> {
    if !handle.ends_with(".base.eth") {
        bail!("handle 형식 오류: `<name>.base.eth` 필요 (예: starian.base.eth)");
    }
    if handle.starts_with('@') {
        bail!("handle 에 `@` 는 빼고 입력 (예: starian.base.eth)");
    }

    let mut section = read_identity(data_dir).unwrap_or_default();
    if section.handle.as_deref() == Some(handle) {
        eprintln!("[identity] 이미 등록된 handle: {handle}");
        return Ok(section);
    }
    section.handle = Some(handle.to_string());
    section.claimed_at = Some(openxgram_core::time::kst_now().to_rfc3339());
    if section.visibility.is_empty() {
        section.visibility = "public".into();
    }
    write_identity(data_dir, &section)?;
    eprintln!("[identity] manifest 갱신 — handle={handle}");

    // onchain 등록은 RPC + key 둘 다 있을 때만. 없으면 명시적 안내.
    match (
        std::env::var("XGRAM_BASE_RPC"),
        std::env::var("XGRAM_BASE_PRIVATE_KEY"),
    ) {
        (Ok(rpc), Ok(_)) if !rpc.trim().is_empty() => {
            eprintln!("[identity] onchain 등록 — XGRAM_BASE_RPC 감지 ({rpc}). (alloy 호출은 다음 PR 통합 예정)");
            // 실 구현: alloy provider 로 Basenames RegistrarController.register() 호출.
            // 현 단계: 인터페이스만 제공 + manifest 갱신 완료.
        }
        _ => {
            eprintln!("[identity] onchain 등록 skip — XGRAM_BASE_RPC + XGRAM_BASE_PRIVATE_KEY 둘 다 필요");
        }
    }
    Ok(section)
}

/// 3.2 — `xgram identity publish`. 5개 records 를 ENS resolver 에 setText.
/// `XGRAM_BASE_RPC` + `XGRAM_BASE_PRIVATE_KEY` 미설정 시 dry-run (records 만 print).
/// visibility=private 면 publish 안 함 (3.2.2).
pub fn publish_records(data_dir: &Path) -> Result<HashMap<String, String>> {
    let section = read_identity(data_dir).context("identity 미설정 — `xgram identity claim` 먼저")?;
    let handle = section
        .handle
        .as_deref()
        .ok_or_else(|| anyhow!("handle 미설정"))?;
    let visibility = Visibility::parse(&section.visibility)?;
    if visibility == Visibility::Private {
        eprintln!("[identity] visibility=private — publish skip (3.2.2)");
        return Ok(HashMap::new());
    }

    let mut records = HashMap::new();
    records.insert("xgram.handle".into(), handle.to_string());
    records.insert(
        "xgram.daemon".into(),
        std::env::var("XGRAM_DAEMON_URL").unwrap_or_else(|_| "http://localhost:47300".into()),
    );
    if let Some(b) = section.bio.as_deref() {
        records.insert("xgram.bio".into(), b.to_string());
    }
    records.insert("xgram.visibility".into(), visibility.as_str().into());
    // pubkey 는 keystore 에서 — 호출자가 keystore 패스워드 안 가졌을 수 있어 manifest 의 alias 만 fallback.
    if let Ok(p) = openxgram_core::paths::keystore_dir(data_dir).join("master.pub").canonicalize() {
        if let Ok(pub_hex) = std::fs::read_to_string(&p) {
            records.insert("xgram.pubkey".into(), pub_hex.trim().into());
        }
    }
    // e-마무리 — 등록된 채널을 xgram.channels JSON 으로 포함 (visibility=private 채널은 제외).
    let chans = crate::channels::channel_list(data_dir).unwrap_or_default();
    let public_chans: Vec<_> = chans.into_iter().filter(|c| c.visibility != "private").collect();
    if !public_chans.is_empty() {
        if let Ok(json) = serde_json::to_string(&public_chans) {
            records.insert("xgram.channels".into(), json);
        }
    }

    match std::env::var("XGRAM_BASE_RPC") {
        Ok(rpc) if !rpc.trim().is_empty() => {
            eprintln!("[identity] publish — RPC={rpc} (alloy setText 호출은 다음 PR)");
        }
        _ => {
            eprintln!("[identity] dry-run publish (XGRAM_BASE_RPC 미설정):");
            for (k, v) in &records {
                eprintln!("  {k} = {v}");
            }
        }
    }
    Ok(records)
}

/// 3.3 — `xgram find @<handle>` — Resolver 호출 + cache.
pub struct HandleResolver {
    cache: std::sync::Mutex<HashMap<String, (HashMap<String, String>, SystemTime)>>,
    pub ttl: Duration,
}

impl HandleResolver {
    pub fn new() -> Self {
        Self {
            cache: std::sync::Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(60 * 60), // 1h (3.3.1.3)
        }
    }

    /// records 조회. 캐시 만료/미스 시 fetch_fn 호출.
    pub fn resolve_with<F>(&self, handle: &str, fetch_fn: F) -> Result<HashMap<String, String>>
    where
        F: FnOnce(&str) -> Result<HashMap<String, String>>,
    {
        let now = SystemTime::now();
        {
            let cache = self.cache.lock().unwrap();
            if let Some((rec, ts)) = cache.get(handle) {
                if now.duration_since(*ts).unwrap_or(Duration::MAX) < self.ttl {
                    return Ok(rec.clone());
                }
            }
        }
        let rec = fetch_fn(handle)?;
        self.cache
            .lock()
            .unwrap()
            .insert(handle.to_string(), (rec.clone(), now));
        Ok(rec)
    }
}

impl Default for HandleResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// step 13 — `xgram identity register --to <indexer-url>` 자기 봇을 외부 디렉터리에 등록.
/// `<url>` 은 indexer-sdk service 의 `/register` 엔드포인트 (또는 호환 서비스).
/// reputation 카운트는 본 노드에서 집계 (메모리/payment/EAS) — 옵션.
pub async fn register_to_directory(
    data_dir: &Path,
    indexer_url: &str,
    include_counts: bool,
) -> Result<()> {
    let section = read_identity(data_dir).context("identity 미설정 — `xgram identity claim` 먼저")?;
    let handle = section
        .handle
        .as_deref()
        .ok_or_else(|| anyhow!("handle 미설정"))?;
    let visibility = Visibility::parse(&section.visibility)?;
    if visibility == Visibility::Private {
        bail!("visibility=private — register 거부 (privacy 우선). `xgram identity set-visibility public` 후 재시도");
    }

    let mut body = serde_json::json!({"handle": handle});
    if include_counts {
        let scores = crate::reputation::aggregate_local_scores(data_dir).unwrap_or_default();
        if let Some(my) = scores.iter().find(|s| s.identity == "me" || s.identity == handle) {
            body["messages"] = serde_json::Value::from(my.messages);
            body["payments_received"] = serde_json::Value::from(my.payments_received);
            body["endorsements_received"] = serde_json::Value::from(my.endorsements_received);
        }
    }

    let url = format!("{}/register", indexer_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("indexer POST")?;
    if !resp.status().is_success() {
        bail!("indexer HTTP {}", resp.status());
    }
    eprintln!("[identity] register 완료 — {url} (handle={handle})");
    Ok(())
}

/// 3.5 — visibility 변경. private 로 가면 ENS records 도 (가능하면) 비우는게 정석이지만
/// gas 절약 위해 현 PR 은 manifest 만 갱신 (다음 PR 에서 ENS clearText).
pub fn set_visibility(data_dir: &Path, mode: &str) -> Result<()> {
    let v = Visibility::parse(mode)?;
    let mut section = read_identity(data_dir).unwrap_or_default();
    section.visibility = v.as_str().into();
    write_identity(data_dir, &section)?;
    eprintln!("[identity] visibility = {}", v.as_str());
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 3.4 — friend request / accept / deny — chain 의존 0
// ─────────────────────────────────────────────────────────────────────────────

/// friend request 메시지 본문 생성 — magic prefix + sender_handle.
pub fn build_friend_request(sender_handle: &str) -> String {
    format!("{FRIEND_REQUEST_PREFIX}\n{sender_handle}")
}

pub fn build_friend_accept(sender_handle: &str) -> String {
    format!("{FRIEND_ACCEPT_PREFIX}\n{sender_handle}")
}

pub fn build_friend_deny(sender_handle: &str, reason: Option<&str>) -> String {
    match reason {
        Some(r) => format!("{FRIEND_DENY_PREFIX}\n{sender_handle}\n{r}"),
        None => format!("{FRIEND_DENY_PREFIX}\n{sender_handle}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FriendMessage {
    Request { sender_handle: String },
    Accept { sender_handle: String },
    Deny { sender_handle: String, reason: Option<String> },
}

pub fn parse_friend_message(body: &str) -> Option<FriendMessage> {
    let mut lines = body.lines();
    let prefix = lines.next()?;
    let handle = lines.next()?.trim();
    if handle.is_empty() {
        return None;
    }
    match prefix {
        FRIEND_REQUEST_PREFIX => Some(FriendMessage::Request {
            sender_handle: handle.into(),
        }),
        FRIEND_ACCEPT_PREFIX => Some(FriendMessage::Accept {
            sender_handle: handle.into(),
        }),
        FRIEND_DENY_PREFIX => {
            let reason = lines.collect::<Vec<_>>().join("\n");
            Some(FriendMessage::Deny {
                sender_handle: handle.into(),
                reason: if reason.is_empty() { None } else { Some(reason) },
            })
        }
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// manifest I/O
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct ManifestSubset {
    #[serde(default)]
    identity: Option<IdentitySection>,
    #[serde(flatten)]
    other: HashMap<String, serde_json::Value>,
}

fn read_identity(data_dir: &Path) -> Result<IdentitySection> {
    let p = manifest_path(data_dir);
    if !p.exists() {
        bail!("manifest 없음: {} — `xgram init` 먼저 실행", p.display());
    }
    let raw = std::fs::read_to_string(&p)?;
    let m: ManifestSubset = serde_json::from_str(&raw).context("manifest 파싱")?;
    Ok(m.identity.unwrap_or_default())
}

fn write_identity(data_dir: &Path, section: &IdentitySection) -> Result<()> {
    let p = manifest_path(data_dir);
    let raw = if p.exists() {
        std::fs::read_to_string(&p)?
    } else {
        "{}".into()
    };
    let mut m: ManifestSubset =
        serde_json::from_str(&raw).unwrap_or_else(|_| ManifestSubset::default());
    m.identity = Some(section.clone());
    let pretty = serde_json::to_string_pretty(&m)?;
    std::fs::write(&p, pretty)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn visibility_parse_round_trip() {
        for v in ["public", "unlisted", "private"] {
            let parsed = Visibility::parse(v).unwrap();
            assert_eq!(parsed.as_str(), v);
        }
        assert!(Visibility::parse("invalid").is_err());
    }

    #[test]
    fn claim_handle_writes_manifest_section() {
        let tmp = tempdir().unwrap();
        // manifest 가 없으면 read 가 bail — 빈 manifest 파일 만들고 진행
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(&mp, "{}").unwrap();

        let section = claim_handle(tmp.path(), "starian.base.eth").unwrap();
        assert_eq!(section.handle.as_deref(), Some("starian.base.eth"));
        assert_eq!(section.visibility, "public");
        assert!(section.claimed_at.is_some());

        // re-read
        let again = read_identity(tmp.path()).unwrap();
        assert_eq!(again.handle.as_deref(), Some("starian.base.eth"));
    }

    #[test]
    fn claim_handle_rejects_invalid_format() {
        let tmp = tempdir().unwrap();
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(&mp, "{}").unwrap();
        assert!(claim_handle(tmp.path(), "no-suffix").is_err());
        assert!(claim_handle(tmp.path(), "@starian.base.eth").is_err());
    }

    #[test]
    fn set_visibility_updates_manifest() {
        let tmp = tempdir().unwrap();
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(&mp, "{}").unwrap();
        set_visibility(tmp.path(), "unlisted").unwrap();
        let s = read_identity(tmp.path()).unwrap();
        assert_eq!(s.visibility, "unlisted");
    }

    #[test]
    fn publish_skips_when_private() {
        let tmp = tempdir().unwrap();
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(
            &mp,
            r#"{"identity": {"handle": "x.base.eth", "visibility": "private"}}"#,
        )
        .unwrap();
        let recs = publish_records(tmp.path()).unwrap();
        assert!(recs.is_empty());
    }

    #[test]
    fn publish_includes_handle_and_visibility() {
        let tmp = tempdir().unwrap();
        let mp = manifest_path(tmp.path());
        std::fs::create_dir_all(mp.parent().unwrap()).unwrap();
        std::fs::write(
            &mp,
            r#"{"identity": {"handle": "x.base.eth", "visibility": "public", "bio": "AI"}}"#,
        )
        .unwrap();
        let recs = publish_records(tmp.path()).unwrap();
        assert_eq!(recs.get("xgram.handle").map(String::as_str), Some("x.base.eth"));
        assert_eq!(
            recs.get("xgram.visibility").map(String::as_str),
            Some("public")
        );
        assert_eq!(recs.get("xgram.bio").map(String::as_str), Some("AI"));
    }

    #[test]
    fn friend_request_round_trip_via_magic_prefix() {
        let body = build_friend_request("starian.base.eth");
        let parsed = parse_friend_message(&body).unwrap();
        assert_eq!(
            parsed,
            FriendMessage::Request {
                sender_handle: "starian.base.eth".into()
            }
        );
    }

    #[test]
    fn friend_accept_and_deny_parse() {
        let acc = parse_friend_message(&build_friend_accept("a.base.eth")).unwrap();
        assert!(matches!(acc, FriendMessage::Accept { .. }));

        let den = parse_friend_message(&build_friend_deny("a.base.eth", Some("거절 사유"))).unwrap();
        match den {
            FriendMessage::Deny { reason, .. } => assert_eq!(reason.as_deref(), Some("거절 사유")),
            _ => panic!("expected Deny"),
        }
    }

    #[test]
    fn parse_unknown_prefix_returns_none() {
        assert!(parse_friend_message("unrelated body\nfoo").is_none());
        assert!(parse_friend_message("xgram-friend-request-v1\n").is_none());
    }

    #[test]
    fn handle_resolver_caches_within_ttl() {
        let r = HandleResolver::new();
        let mut count = 0;
        let _ = r.resolve_with("a.base.eth", |_| {
            count += 1;
            let mut m = HashMap::new();
            m.insert("xgram.handle".into(), "a".into());
            Ok(m)
        });
        let _ = r.resolve_with("a.base.eth", |_| {
            count += 1; // shouldn't fire
            Ok(HashMap::new())
        });
        assert_eq!(count, 1, "두 번째 호출은 cache hit");
    }
}
