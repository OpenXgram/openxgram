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

/// transport/base URL 에서 host(authority 의 host 부분)만 추출. scheme·port·path 무시.
/// 예: "http://100.64.0.5:47300" → "100.64.0.5". 파싱 실패 시 None.
fn url_host(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    let host = authority.rsplit_once(':').map(|(h, _)| h).unwrap_or(authority);
    let host = host.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
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
///
/// rc.273 — **자기 머신의 살아있는 tmux 에이전트만 광고**한다 (마스터 룰: 죽은/비-tmux peer 노출 금지).
/// 판정:
///   - LOCAL peer = `session_identifier` 가 `tmux:<name>` 형식 (auto_seed 가 자기 머신 세션에만 기록).
///     이 LOCAL peer 는 `local_live_tmux_agent_idents()` 의 live 집합에 그 ident 가 있어야 광고.
///     → 죽은 tmux·비-tmux(session_identifier 없음 또는 비-tmux) LOCAL 등록 peer 는 제외.
///   - 원격 병합 peer(session_identifier 없음, eth 가 self 아님)는 **재광고하지 않음**
///     (자기 머신 것만 광고 — 원격은 그 원격의 데몬이 책임).
///   - self peer(session_identifier 없어도 eth==self_eth)는 항상 광고 — 다른 머신이 나를 알아야 함
///     (e2e #3 cross-machine 인지 보존).
///
/// session_identifier 는 PeerStore.list() 미반환 필드라 raw SQL 로 alias→ident 맵을 prefetch.
pub fn reachable_remote_peers(data_dir: &Path) -> anyhow::Result<Vec<RemotePeer>> {
    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;

    // alias → session_identifier prefetch (gui_peers 와 동일 방식).
    let mut sid_map: std::collections::HashMap<String, String> = Default::default();
    if let Ok(mut stmt) = db.conn().prepare(
        "SELECT alias, session_identifier FROM peers WHERE session_identifier IS NOT NULL AND session_identifier != ''",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                sid_map.insert(row.0, row.1);
            }
        }
    }
    // 자기 머신의 살아있는 tmux 에이전트 ident 집합 (단일 헬퍼 — 회귀 방지 중앙화).
    let live = crate::daemon::local_live_tmux_agent_idents();
    let self_eth = self_eth_address(data_dir);

    let mut store = PeerStore::new(&mut db);
    let peers = store.list()?;
    Ok(peers
        .into_iter()
        .filter(|p| !is_unreachable_address(&p.address))
        .filter_map(|p| {
            let eth = p.eth_address.clone()?;
            if eth.trim().is_empty() || p.public_key_hex.trim().is_empty() {
                return None;
            }
            // self peer 는 무조건 광고 (cross-machine 인지 보존).
            let is_self = self_eth
                .as_deref()
                .map(|me| me.eq_ignore_ascii_case(eth.trim()))
                .unwrap_or(false);
            if !is_self {
                match sid_map.get(&p.alias) {
                    // LOCAL tmux peer — 살아있는 세션 집합에 있을 때만 광고.
                    Some(sid) if sid.starts_with("tmux:") => {
                        if !live.contains(sid) {
                            return None; // 죽은 tmux LOCAL peer — 광고 제외.
                        }
                    }
                    // session_identifier 없음 = 원격 병합 peer (또는 비-tmux LOCAL) — 자기 것만 광고하므로 제외.
                    _ => return None,
                }
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
            Ok(remote) => {
                // rc.273 — tombstone prune: fetch **성공** 시에만, 그 base 가 더는 광고 안 하는
                // 그 base 소속(같은 host) 로컬 peer 를 제거. fetch 실패 시엔 절대 prune 금지(과삭제 방지).
                match prune_absent_from_remote(&mut db, base, &remote, self_eth.as_deref()) {
                    Ok(pruned) if pruned > 0 => {
                        tracing::info!(base = %base, pruned = pruned, "peer-sync tombstone prune (원격 미광고)")
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(base = %base, error = %e, "peer-sync prune 실패 (계속)")
                    }
                }
                match merge_remote_peers(&mut db, &remote, self_eth.as_deref()) {
                    Ok(n) => total += n,
                    Err(e) => {
                        tracing::warn!(base = %base, error = %e, "peer-sync merge 실패 (계속)")
                    }
                }
            }
            Err(e) => {
                // ⚠️ fetch 실패(네트워크 에러) — prune 절대 금지. 죽은 줄 알고 전체 삭제하는 사고 방지.
                tracing::warn!(base = %base, error = %e, "peer-sync pull 실패 (계속) — prune skip");
            }
        }
    }
    Ok(total)
}

/// rc.273 — 원격(base) 의 reachable 목록을 **성공적으로 fetch 한 뒤**, 그 base 소속(같은 host)
/// 인데 목록에 없는 로컬 peer 를 삭제(absence = tombstone). 삭제 row 수 반환.
///
/// 안전장치 (호출자 보장 + 여기 재확인):
///   - fetch 성공 시에만 호출됨 — 빈 목록(remote=[]) 도 "그 host 에 아무도 없음" 의 정당한 신호.
///   - 삭제 대상 = `address` 의 host 가 base host 와 일치하는 peer 만 (다른 머신·로컬 tmux 보호).
///   - self peer(eth==self_eth)·eth 없는 peer 는 제외.
///   - 자기 머신 LOCAL tmux peer 는 base host(원격) 와 host 가 다르므로 자연히 제외됨.
fn prune_absent_from_remote(
    db: &mut openxgram_db::Db,
    base: &str,
    remote: &[RemotePeer],
    self_eth: Option<&str>,
) -> anyhow::Result<usize> {
    let base_host = match url_host(base) {
        Some(h) => h,
        None => return Ok(0),
    };
    // 원격이 현재 광고하는 eth 집합 (소문자 정규화).
    let advertised: std::collections::HashSet<String> = remote
        .iter()
        .map(|p| p.eth_address.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let mut store = PeerStore::new(db);
    let peers = store.list()?;
    // 삭제 후보: base host 소속 + eth 보유 + self 아님 + 원격 미광고.
    let to_delete: Vec<String> = peers
        .into_iter()
        .filter_map(|p| {
            let eth = p.eth_address?;
            let eth_norm = eth.trim().to_ascii_lowercase();
            if eth_norm.is_empty() {
                return None;
            }
            if let Some(me) = self_eth {
                if me.eq_ignore_ascii_case(eth_norm.trim()) {
                    return None; // self 보호.
                }
            }
            // 그 base host 소속만 prune 대상 (다른 머신·로컬 tmux peer 보호).
            match url_host(&p.address) {
                Some(h) if h == base_host => {}
                _ => return None,
            }
            if advertised.contains(&eth_norm) {
                return None; // 여전히 광고됨 — 유지.
            }
            Some(p.alias)
        })
        .collect();

    let mut pruned = 0usize;
    for alias in to_delete {
        match store.delete(&alias) {
            Ok(_) => {
                pruned += 1;
                tracing::info!(alias = %alias, base = %base, "peer-sync prune — 원격 미광고 로컬 row 삭제");
            }
            Err(e) => tracing::warn!(alias = %alias, error = %e, "peer-sync prune delete 실패"),
        }
    }
    Ok(pruned)
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
    fn url_host_extracts_host_only() {
        assert_eq!(url_host("http://100.64.0.5:47300").as_deref(), Some("100.64.0.5"));
        assert_eq!(url_host("http://host.tld:1/path").as_deref(), Some("host.tld"));
        assert_eq!(url_host("100.64.0.6:47300").as_deref(), Some("100.64.0.6"));
        assert_eq!(url_host("").as_deref(), None);
    }

    #[test]
    fn prune_removes_only_absent_same_host_peers() {
        let mut db = open_mem_db();
        // 같은 base host(100.64.0.5) 에 두 peer 가 있었다.
        let initial = vec![
            rp("alive", "http://100.64.0.5:47300", "0xALIVE"),
            rp("dead", "http://100.64.0.5:47300", "0xDEAD"),
            // 다른 머신 peer — prune 대상 아님.
            rp("other_host", "http://100.64.0.9:47300", "0xOTHER"),
        ];
        merge_remote_peers(&mut db, &initial, None).unwrap();

        // base(100.64.0.5) 가 이제 alive 만 광고 → dead 는 tombstone.
        let now_remote = vec![rp("alive", "http://100.64.0.5:47300", "0xALIVE")];
        let pruned =
            prune_absent_from_remote(&mut db, "http://100.64.0.5:47300", &now_remote, None).unwrap();
        assert_eq!(pruned, 1, "dead 1 개만 prune");

        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xALIVE").unwrap().is_some());
        assert!(store.get_by_eth_address("0xDEAD").unwrap().is_none(), "미광고 → 삭제");
        assert!(
            store.get_by_eth_address("0xOTHER").unwrap().is_some(),
            "다른 host peer 는 보호"
        );
    }

    #[test]
    fn prune_protects_self() {
        let mut db = open_mem_db();
        merge_remote_peers(
            &mut db,
            &[rp("me", "http://100.64.0.5:47300", "0xSELF")],
            None,
        )
        .unwrap();
        // 원격이 self 를 광고 안 해도 (빈 목록) self 는 보호.
        let pruned =
            prune_absent_from_remote(&mut db, "http://100.64.0.5:47300", &[], Some("0xself"))
                .unwrap();
        assert_eq!(pruned, 0, "self peer 는 prune 금지");
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xSELF").unwrap().is_some());
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
