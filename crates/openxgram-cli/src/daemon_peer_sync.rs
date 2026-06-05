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
//! ## 현재 구현 범위 (rc.263 — 활성화 완료)
//! - `merge_remote_peers` — 순수·단위테스트 가능한 병합 로직 (완성).
//! - `RemotePeer` — 교환 DTO (eth_address 포함 — 식별 키로 필수).
//! - `reachable_remote_peers` — 자기 DB → provider 가 노출할 `Vec<RemotePeer>` 매핑.
//! - `fetch_remote_peers` — 원격 `GET /v1/peers/reachable` pull (reqwest, timeout 3s).
//! - `sync_tick_once` — reachable base 순회 → pull → merge (per-peer warn+continue).
//! - `spawn_peer_sync` — 60s 주기 tick.
//!
//! ## 엔드포인트 (설계 노트 (A) — rc.263 합류)
//! transport 서버(envelope 포트)에 read-only `GET /v1/peers/reachable` 추가됨 —
//! reachable peer 만(localhost 제외) {alias,pubkey,eth,address,gui,role} 반환.
//! transport 크레이트는 저수준(openxgram-db/peer 무의존)이므로 daemon 이
//! `reachable_remote_peers` 결과를 `ReachablePeerProvider` closure 로 주입한다
//! (의존성 순환 방지). 기존 `GET /v1/gui/peers`(PeerDto, eth 미노출 + Bearer)는 무관.

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

/// 자기 DB 에서 reachable(localhost 아님) + eth_address·pubkey 보유 peer 를 `RemotePeer` 로 매핑.
/// `GET /v1/peers/reachable` provider 가 노출할 목록 — 다른 머신 데몬이 이를 pull 해 merge 한다.
/// self 항목도 자기 peers table 에 있으면 포함되어 다른 머신이 나를 알 수 있다(받는 쪽이 self_eth 로 거름).
pub fn reachable_remote_peers(data_dir: &Path) -> anyhow::Result<Vec<RemotePeer>> {
    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;
    let mut store = PeerStore::new(&mut db);
    let peers = store.list()?;
    Ok(peers
        .into_iter()
        .filter(|p| !is_unreachable_address(&p.address))
        .filter_map(|p| {
            let eth = p.eth_address?;
            if eth.trim().is_empty() || p.public_key_hex.trim().is_empty() {
                return None;
            }
            Some(RemotePeer {
                alias: p.alias,
                public_key_hex: p.public_key_hex,
                eth_address: eth,
                gui_address: derive_gui_url(&p.address),
                address: p.address,
                role: p.role.as_str().to_string(),
            })
        })
        .collect())
}

/// 자기 신원(eth address). `XGRAM_KEYSTORE_PASSWORD` 가 있으면 master keystore 에서 derive.
/// 없으면 None — 그 경우 merge 가 self 항목을 idempotent 재upsert 할 뿐 무해.
fn self_eth_address(data_dir: &Path) -> Option<String> {
    use openxgram_keystore::Keystore;
    let pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").ok()?;
    let ks = openxgram_keystore::FsKeystore::new(openxgram_core::paths::keystore_dir(data_dir));
    let kp = ks
        .load(openxgram_core::paths::MASTER_KEY_NAME, &pw)
        .ok()?;
    Some(kp.address.as_str().to_string())
}

/// 원격 데몬의 `GET {base}/v1/peers/reachable` 를 pull → `Vec<RemotePeer>` 역직렬화.
/// timeout 3s. 실패는 에러 반환(호출자 sync_tick_once 가 per-peer 로 잡아 warn 후 continue).
async fn fetch_remote_peers(base: &str) -> anyhow::Result<Vec<RemotePeer>> {
    let url = format!("{}/v1/peers/reachable", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    let resp = client.get(&url).send().await?.error_for_status()?;
    let peers = resp.json::<Vec<RemotePeer>>().await?;
    Ok(peers)
}

/// 1회 sync tick — reachable peer 들의 `GET /v1/peers/reachable` 를 pull 해 merge.
///
/// rc.263: 설계 노트 (A) 엔드포인트 합류로 활성화됨. 각 reachable base 를 순회하며
/// `fetch_remote_peers` 로 목록을 받아 `merge_remote_peers` 로 병합한다.
/// per-peer 실패(네트워크/역직렬화)는 전체를 멈추지 않고 warn 후 continue.
/// 병합된 총 row 수 반환.
pub async fn sync_tick_once(data_dir: &Path) -> anyhow::Result<usize> {
    let candidates = reachable_peer_addresses(data_dir)?;
    if candidates.is_empty() {
        tracing::debug!("peer-sync tick: reachable peer 0 — skip");
        return Ok(0);
    }
    let self_eth = self_eth_address(data_dir);
    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;
    let mut total = 0usize;
    for base in &candidates {
        match fetch_remote_peers(base).await {
            Ok(remote) => match merge_remote_peers(&mut db, &remote, self_eth.as_deref()) {
                Ok(n) => total += n,
                Err(e) => {
                    tracing::warn!(base = %base, error = %e, "peer-sync merge 실패 (계속)")
                }
            },
            Err(e) => {
                tracing::warn!(base = %base, error = %e, "peer-sync pull 실패 (계속)");
            }
        }
    }
    Ok(total)
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
    fn remote_peer_json_matches_dto_shape() {
        // RemotePeer 가 transport 의 ReachablePeerDto 와 동일 JSON 형태여야 한다
        // (provider 직렬화 → fetch_remote_peers 역직렬화 round-trip).
        let dto = openxgram_transport::ReachablePeerDto {
            alias: "akashic".to_string(),
            public_key_hex: "02abc".to_string(),
            eth_address: "0xDEAD".to_string(),
            address: "http://100.64.0.9:47300".to_string(),
            gui_address: Some("http://100.64.0.9:47302".to_string()),
            role: "primary".to_string(),
        };
        let json = serde_json::to_string(&dto).unwrap();
        let rp: RemotePeer = serde_json::from_str(&json).unwrap();
        assert_eq!(rp.alias, "akashic");
        assert_eq!(rp.eth_address, "0xDEAD");
        assert_eq!(rp.address, "http://100.64.0.9:47300");
        assert_eq!(rp.gui_address.as_deref(), Some("http://100.64.0.9:47302"));
        assert_eq!(rp.role, "primary");
        // 역방향도 동일 형태 — DTO 로 다시 역직렬화 가능.
        let back: openxgram_transport::ReachablePeerDto =
            serde_json::from_str(&serde_json::to_string(&rp).unwrap()).unwrap();
        assert_eq!(back, dto);
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
