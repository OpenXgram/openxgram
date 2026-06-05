//! Cross-machine peer registry sync (gossip) — 데몬 간 reachable agent 목록 교환.
//!
//! ## 배경 (해결하는 본질 결함)
//! 각 머신 데몬은 자기 로컬 등록 peer 만 안다. 다른 머신(B)의 sub-agent 를 모르므로
//! A→B-subagent 직접 전송이 불가능했다(지금은 primary 만 알음 → primary 가 위임).
//! 각 데몬이 fleet 전체의 reachable agent 를 알면 직접 연결이 된다.
//!
//! ## 설계 (최소 구현 — primary↔primary sub-agent 목록 교환)
//! 1. 주기 tick(기본 60s)이 자기 DB 의 **reachable peer**(localhost 아님)들을 순회.
//! 2. 각 reachable peer 의 데몬에서 그쪽 peer 목록을 pull → `merge_remote_peers` 로 병합.
//! 3. 병합 규칙(`merge_remote_peers`):
//!    - identity 키 = eth_address(우선) / pubkey. `upsert_announce` 로 idempotent UPSERT.
//!    - **localhost/unspecified 주소는 받지도 전파하지도 않음**(오염 방지).
//!    - 자기 자신(같은 eth)·중복 제거. 더 최근 정보가 주소를 덮어씀.
//! 4. 보안: pull 대상은 tailnet/LAN(`is_unreachable_address` 의 역) 로 제한.
//!    서명 신뢰 경계는 기존 envelope 체계를 따른다(이 sync 는 주소록 힌트만 제공하며,
//!    실제 메시지 수신 시 process_inbound 가 서명 검증 + eth→pubkey 매칭으로 재확인).
//!
//! ## 현재 구현 범위 (이 PR)
//! - `merge_remote_peers` — 순수·단위테스트 가능한 병합 로직 (완성).
//! - `RemotePeer` — 교환 DTO (eth_address 포함 — 식별 키로 필수).
//! - `spawn_peer_sync` / `sync_tick_once` — 주기 tick scaffold (reachable peer 순회 +
//!   merge 호출). 실제 원격 pull 은 eth_address 를 노출하는 교환 엔드포인트가 필요.
//!
//! ## 설계 노트 — 후속(별도 PR) 필요 사항
//! 기존 `GET /v1/gui/peers`(PeerDto)는 **eth_address 미노출** + Bearer 인증 필요라
//! gossip pull 에 부적합. 다음 중 하나가 필요:
//!   (A) transport 서버(envelope 포트)에 read-only `GET /v1/peers/reachable` 추가 —
//!       reachable peer 만(localhost 제외) {alias,pubkey,eth,address,role} 반환.
//!       transport 크레이트에 DB 접근을 주입해야 하므로 별도 PR 로 분리(범위 보호).
//!   (B) 또는 PeerDto 에 eth_address 필드 추가 + 전용 sync 토큰.
//! 이 PR 은 (A) 의 client/merge 측을 완비하고 endpoint 는 노트로 남긴다.

use std::path::{Path, PathBuf};

use openxgram_peer::{PeerRole, PeerStore};
use openxgram_transport::tailscale::is_unreachable_address;
use serde::{Deserialize, Serialize};

/// 데몬 간 교환되는 reachable peer 요약. identity 키로 eth_address 필수.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemotePeer {
    pub alias: String,
    pub public_key_hex: String,
    /// 0x… ECDSA 주소 — `upsert_announce` 의 identity 키.
    pub eth_address: String,
    /// http://<reachable-ip>:<port> — localhost 면 merge 에서 거부.
    pub address: String,
    /// gui_address(transport+2) — cross-machine 터미널 proxy 용. 없으면 derive 시도.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui_address: Option<String>,
    /// "primary" / "secondary" / "worker".
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "worker".to_string()
}

/// 원격 peer 목록을 로컬 DB 로 병합. 병합된(신규/갱신) row 수 반환.
///
/// 규칙:
///   - `address` 가 localhost/unspecified/빈값이면 skip (오염 방지).
///   - `eth_address`·`public_key_hex` 둘 다 비면 skip (식별 불가).
///   - `self_eth` 와 같은 신원은 skip (자기 자신).
///   - 그 외는 `upsert_announce`(eth→pubkey 키 UPSERT, idempotent).
pub fn merge_remote_peers(
    db: &mut openxgram_db::Db,
    remote: &[RemotePeer],
    self_eth: Option<&str>,
) -> anyhow::Result<usize> {
    let mut merged = 0usize;
    let mut store = PeerStore::new(db);
    for rp in remote {
        if is_unreachable_address(&rp.address) {
            tracing::debug!(alias = %rp.alias, addr = %rp.address, "peer-sync skip — 도달 불가 주소");
            continue;
        }
        if rp.eth_address.trim().is_empty() || rp.public_key_hex.trim().is_empty() {
            tracing::debug!(alias = %rp.alias, "peer-sync skip — eth/pubkey 누락");
            continue;
        }
        if let Some(me) = self_eth {
            if me.eq_ignore_ascii_case(rp.eth_address.trim()) {
                continue; // 자기 자신
            }
        }
        let role = PeerRole::parse(&rp.role).unwrap_or(PeerRole::Worker);
        let gui = rp
            .gui_address
            .clone()
            .or_else(|| derive_gui_url(&rp.address));
        match store.upsert_announce(
            &rp.alias,
            &rp.public_key_hex,
            &rp.address,
            gui.as_deref(),
            &rp.eth_address,
            role,
        ) {
            Ok(_) => {
                merged += 1;
                tracing::info!(alias = %rp.alias, eth = %rp.eth_address, addr = %rp.address, "peer-sync merge");
            }
            Err(e) => {
                tracing::warn!(alias = %rp.alias, error = %e, "peer-sync merge 실패");
            }
        }
    }
    Ok(merged)
}

/// transport URL(http://host:PORT) → GUI URL(포트 +2). 파싱 실패 시 None.
fn derive_gui_url(transport_url: &str) -> Option<String> {
    let idx = transport_url.rfind(':')?;
    let (head, rest) = transport_url.split_at(idx);
    let port: u16 = rest[1..].trim_end_matches('/').parse().ok()?;
    Some(format!("{head}:{}", port + 2))
}

/// 자기 DB 에서 reachable(localhost 아님) peer 들의 base address 집합을 모은다.
/// gossip pull 대상 후보 — 각 reachable peer 의 데몬에 sync 엔드포인트가 있다고 가정.
pub fn reachable_peer_addresses(data_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;
    let mut store = PeerStore::new(&mut db);
    let peers = store.list()?;
    Ok(peers
        .into_iter()
        .map(|p| p.address)
        .filter(|a| !is_unreachable_address(a))
        .collect())
}

/// 1회 sync tick — reachable peer 들로부터 목록 pull 시도 후 merge.
///
/// 현 PR: 원격 pull 엔드포인트(설계 노트 (A)) 부재로, 후보 reachable peer 수를
/// 로깅하고 merge scaffold 만 수행(noop pull). endpoint 합류 시 `fetch_remote_peers`
/// 만 구현하면 즉시 활성화된다.
pub async fn sync_tick_once(data_dir: &Path) -> anyhow::Result<usize> {
    let candidates = reachable_peer_addresses(data_dir)?;
    if candidates.is_empty() {
        tracing::debug!("peer-sync tick: reachable peer 0 — skip");
        return Ok(0);
    }
    tracing::debug!(
        count = candidates.len(),
        "peer-sync tick: reachable peer 후보 (원격 pull 엔드포인트 합류 시 활성화)"
    );
    // 후속 PR: for base in candidates { let remote = fetch_remote_peers(&base).await?; ... }
    Ok(0)
}

/// 주기 sync tick spawn — 기본 60초 간격. daemon startup 에서 호출.
pub fn spawn_peer_sync(data_dir: PathBuf) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        // 첫 tick 즉시 발화 회피 — startup 직후 retroactive self-heal 이 먼저 끝나게.
        interval.tick().await;
        loop {
            interval.tick().await;
            match sync_tick_once(&data_dir).await {
                Ok(n) if n > 0 => tracing::info!(merged = n, "peer-sync: cross-machine 병합"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "peer-sync tick 실패 (계속)"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rp(alias: &str, addr: &str, eth: &str) -> RemotePeer {
        RemotePeer {
            alias: alias.to_string(),
            public_key_hex: format!("02{:0>62}", alias.len()),
            eth_address: eth.to_string(),
            address: addr.to_string(),
            gui_address: None,
            role: "worker".to_string(),
        }
    }

    // tempfile dev-dep 추가 회피 — std 만으로 고유 임시 DB 파일 경로 생성.
    fn open_mem_db() -> openxgram_db::Db {
        let mut p = std::env::temp_dir();
        let uniq = format!(
            "oxg-peer-sync-test-{}-{}.sqlite",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        p.push(uniq);
        let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
            path: p,
            ..Default::default()
        })
        .expect("temp db");
        db.migrate().expect("migrate");
        db
    }

    #[test]
    fn merge_skips_localhost_addresses() {
        let mut db = open_mem_db();
        let remote = vec![
            rp("local", "http://127.0.0.1:47300", "0xaaa1"),
            rp("zero", "http://0.0.0.0:47300", "0xaaa2"),
            rp("good", "http://100.101.237.9:47300", "0xbbb1"),
        ];
        let merged = merge_remote_peers(&mut db, &remote, None).unwrap();
        assert_eq!(merged, 1, "localhost/0.0.0.0 는 거부, reachable 1 개만 merge");
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xbbb1").unwrap().is_some());
        assert!(store.get_by_eth_address("0xaaa1").unwrap().is_none());
    }

    #[test]
    fn merge_skips_self() {
        let mut db = open_mem_db();
        let remote = vec![
            rp("me", "http://100.64.0.5:47300", "0xSELF"),
            rp("other", "http://100.64.0.6:47300", "0xOTHER"),
        ];
        let merged = merge_remote_peers(&mut db, &remote, Some("0xself")).unwrap();
        assert_eq!(merged, 1, "self(eth 동일, 대소문자 무시) 제외");
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xOTHER").unwrap().is_some());
    }

    #[test]
    fn merge_is_idempotent() {
        let mut db = open_mem_db();
        let remote = vec![rp("dup", "http://100.64.0.7:47300", "0xdup")];
        let a = merge_remote_peers(&mut db, &remote, None).unwrap();
        let b = merge_remote_peers(&mut db, &remote, None).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 1, "재실행해도 row 중복 안 생김 (UPSERT)");
        let mut store = PeerStore::new(&mut db);
        let all = store.list().unwrap();
        assert_eq!(all.iter().filter(|p| p.alias == "dup").count(), 1);
    }

    #[test]
    fn derive_gui_url_adds_two() {
        assert_eq!(
            derive_gui_url("http://100.64.0.1:47300").as_deref(),
            Some("http://100.64.0.1:47302")
        );
        assert_eq!(
            derive_gui_url("http://192.168.1.5:17400").as_deref(),
            Some("http://192.168.1.5:17402")
        );
    }
}
