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
///
/// `allow_create`:
///   - `false` — 마스터 룰(rc.347) 보존: **이미 존재하는 peer 만 UPDATE**. 처음 보는
///     alias/eth(미등록 머신 추정)는 CREATE 하지 않는다. 신뢰 경계가 불확실한 일반
///     gossip 호출에 사용.
///   - `true` — pull 대상이 **등록된(`agent_profiles.source='user'`) 머신만**으로 이미
///     좁혀진 신뢰 경로(`sync_tick_once`)에서 사용. 미존재 peer 도 CREATE 허용 →
///     등록된 원격 머신(예: zalman)의 에이전트가 routable peer 로 흡수된다.
///     이때도 위의 안전 필터(unreachable/eth·pubkey 누락/self-eth)는 그대로 적용된다.
pub fn merge_remote_peers(
    db: &mut openxgram_db::Db,
    remote: &[RemotePeer],
    self_eth: Option<&str>,
    allow_create: bool,
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
        // 마스터 룰(rc.347) — gossip 신규 흡수 차단. `allow_create=false` 면 이미 존재하는
        //   peer 만 UPDATE 하고, 처음 보는 alias/eth 는 CREATE 하지 않는다(미등록 머신
        //   자동흡수 방지). `allow_create=true`(등록 머신만 pull 하는 신뢰 경로)면 신규
        //   CREATE 를 허용해 등록 원격 머신의 에이전트를 routable peer 로 흡수한다.
        if !allow_create {
            let exists = store.get_by_eth_address(rp.eth_address.trim())?.is_some()
                || store.get_by_alias(&rp.alias)?.is_some();
            if !exists {
                tracing::debug!(alias = %rp.alias, eth = %rp.eth_address, "peer-sync skip — gossip 신규 흡수 차단(미존재 peer, allow_create=false)");
                continue;
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
    // 마스터 룰 — pull(gossip) 대상은 UI 로 등록한 머신만.
    //   agent_profiles.source='user' 행이 있는 alias 의 peer 만 pull 후보로 채택.
    //   미등록 머신은 pull 안 함 → 그 머신의 peer 가 흡수되지 않는다.
    let mut registered: std::collections::HashSet<String> = Default::default();
    if let Ok(mut stmt) = db
        .conn()
        .prepare("SELECT alias FROM agent_profiles WHERE source = 'user'")
    {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for a in rows.flatten() {
                registered.insert(a);
            }
        }
    }
    let mut store = PeerStore::new(&mut db);
    let peers = store.list()?;
    Ok(peers
        .into_iter()
        .filter(|p| registered.contains(&p.alias))
        .map(|p| p.address)
        .filter(|a| !is_unreachable_address(a))
        .collect())
}

/// Part 1 (cross-machine peer-routing seed) — peers 테이블 유래 후보 + env 부트스트랩 URL 을
/// 합쳐 중복·self·도달불가를 제거한 **유일** pull 후보 목록을 만드는 순수 함수.
///
/// 배경(본질 결함): cross-machine sync 는 pull 기반인데, 등록된 **원격 머신**(예: zalman)은
/// 아직 로컬 `peers` row 가 없고 그 transport URL 이 DB 어디에도 구조화 저장돼 있지 않다 →
/// seoul 이 zalman 에서 pull 할 시드(base URL)가 없다. 운영자가 `XGRAM_PEER_BOOTSTRAP_URLS`
/// (콤마/공백 구분 transport base URL 목록)로 시드를 제공하면 첫 pull 이 성사되고, 그 후
/// Part 2(`allow_create`)로 원격 에이전트가 `peers` row 로 흡수돼 자생적 후보가 된다
/// (부트스트랩은 seed/recover 용도로만 필요).
///
/// 규칙(정적 하드코딩 금지 — env 가 진리원천):
///   - `bootstrap_env` 를 콤마/공백으로 split, trim, 빈 항목 제거.
///   - `peer_table_urls` + env 후보를 합친다.
///   - `is_unreachable_address`(loopback/unspecified/빈값)는 제외.
///   - `self_host` 와 host 가 같은 후보는 제외(자기 자신 pull 방지).
///   - 동일 URL(정규화: trailing '/' 제거 + ascii 소문자) 중복은 1회만.
/// 입력 순서를 보존하되 첫 등장만 채택한다.
pub fn combine_pull_candidates(
    peer_table_urls: Vec<String>,
    bootstrap_env: Option<&str>,
    self_host: Option<&str>,
) -> Vec<String> {
    let env_urls = bootstrap_env
        .into_iter()
        .flat_map(|s| s.split([',', ' ', '\t', '\n', '\r']))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut seen: std::collections::HashSet<String> = Default::default();
    let mut out: Vec<String> = Vec::new();
    for url in peer_table_urls.into_iter().chain(env_urls) {
        let url = url.trim().to_string();
        if url.is_empty() || is_unreachable_address(&url) {
            continue;
        }
        // self host 와 같은 후보 제외 — 자기 데몬을 pull 대상으로 삼지 않는다.
        if let (Some(sh), Some(uh)) = (self_host, url_host(&url)) {
            if sh.eq_ignore_ascii_case(&uh) {
                continue;
            }
        }
        let norm = url.trim_end_matches('/').to_ascii_lowercase();
        if seen.insert(norm) {
            out.push(url);
        }
    }
    out
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

    // BUG2/3 (cross-machine A2A discovery) — 자기 머신의 LOCAL ACP-drivable 에이전트 alias 집합.
    // `agent_capabilities` 에 row 가 있고 role != 'tmux' 이면 그 alias 는 이 머신의 데몬이
    // 로컬 ACP(`is_acp_drivable`/`load_a2a_agent_meta` 와 동일 기준)로 구동 가능한 신원이다.
    // 이런 에이전트(예: zalman 의 navi)는 라이브 tmux pane 이 없어 sid_map 에 'tmux:*' 가
    // 없을 수 있다 → 기존 rc.273 필터의 `_ => return None` 에 걸려 광고에서 누락됐다.
    //   결과: seoul 이 "navi 가 zalman 에 산다"를 영영 알 수 없었다(=BUG2/3 discovery 갭).
    // 이 집합으로 LOCAL ACP 에이전트를 추가 광고 대상에 포함한다. 원격 병합 peer 는
    // 이 머신의 agent_capabilities 에 row 가 없으므로 여전히 재광고되지 않는다(소유권 보존).
    let local_acp_aliases: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        if let Ok(mut stmt) = db.conn().prepare(
            "SELECT alias FROM agent_capabilities WHERE role IS NOT 'tmux' AND alias IS NOT NULL AND alias != ''",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for a in rows.flatten() {
                    set.insert(a);
                }
            }
        }
        set
    };

    let mut store = PeerStore::new(&mut db);
    let peers = store.list()?;

    // rc.345 — self 머신의 reachable transport 주소는 **동적 설정(env)이 진리원천**이다.
    // DB self peer row 의 address 는 캐시일 뿐 — gossip 으로 옛/오염 값이 들어올 수 있어
    // self 광고의 권위로 신뢰하지 않는다(예: zalman self-row 가 seoul 주소로 오염됐던 사건).
    // 마스터가 머신별로 설정한 XGRAM_TRANSPORT_PUBLIC_URL(또는 XGRAM_SELF_ADDRESS)을 우선
    // 채택(도달 가능 시). 그래야 각 머신이 자기 env 만 맞으면 self 광고가 자동 정상화되고
    // 수동 DB 시드/신원 선택이 영구 불필요해진다. env 미설정·도달불가일 때만 self peer row
    // (eth==self_eth) 주소로 폴백한다.
    let self_machine_addr: Option<String> = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
        .ok()
        .filter(|u| !u.trim().is_empty() && !is_unreachable_address(u))
        .or_else(|| {
            std::env::var("XGRAM_SELF_ADDRESS")
                .ok()
                .filter(|u| !u.trim().is_empty() && !is_unreachable_address(u))
        })
        .or_else(|| {
            self_eth.as_deref().and_then(|me| {
                peers.iter().find_map(|p| {
                    let eth = p.eth_address.as_deref()?;
                    if me.eq_ignore_ascii_case(eth.trim()) && !is_unreachable_address(&p.address) {
                        Some(p.address.clone())
                    } else {
                        None
                    }
                })
            })
        });

    Ok(peers
        .iter()
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
            // BUG2/3 — LOCAL ACP-drivable 에이전트면 머신 데몬 주소로 광고(아래에서 override).
            let mut is_local_acp_agent = false;
            if !is_self {
                match sid_map.get(&p.alias) {
                    // LOCAL tmux peer — 살아있는 세션 집합에 있을 때만 광고.
                    Some(sid) if sid.starts_with("tmux:") => {
                        if !live.contains(sid) {
                            return None; // 죽은 tmux LOCAL peer — 광고 제외.
                        }
                    }
                    // BUG2/3 — LOCAL ACP-drivable 에이전트(tmux pane 없음)는 광고 대상에 포함.
                    //   seoul 이 alias→machine 매핑으로 cross-machine A2A 라우팅을 할 수 있게 한다.
                    //   수신측 process_inbound(① fix)가 recipient_alias 로 navi 의 로컬 ACP 를 구동한다.
                    _ if local_acp_aliases.contains(&p.alias) => {
                        is_local_acp_agent = true;
                    }
                    // session_identifier 없음 = 원격 병합 peer (또는 비-tmux LOCAL) — 자기 것만 광고하므로 제외.
                    _ => return None,
                }
            }
            // rc.344 — 오등록 원격 에이전트 가드. agent_capabilities 에 row 가 있어
            // is_local_acp_agent 로 잡혔더라도, 이미 self 와 **다른 reachable 주소**를 광고
            // 중이면 실제로는 원격 머신 에이전트(이 데몬 DB 에 오등록된 peer, 예: zalman)다.
            // self 머신 주소로 덮어쓰지 말고 재광고에서 제외한다 — 그 머신이 자기를 광고하게 둔다.
            // 안 그러면 seoul 이 zalman 을 자기 주소로 광고 → 자기-pull 로 재흡수 → 영영 self 주소로
            // 고정되어 cross-machine A2A 라우팅이 404("unknown agent")로 떨어진다(sidecar peer-lookup 404).
            // 현재 모든 peer 주소가 self 와 같으면 이 가드는 no-op(동작 변화 없음).
            if is_local_acp_agent {
                if let Some(self_addr) = self_machine_addr.as_deref() {
                    let pa = p.address.trim_end_matches('/');
                    let sa = self_addr.trim_end_matches('/');
                    if !is_unreachable_address(&p.address) && !pa.eq_ignore_ascii_case(sa) {
                        tracing::warn!(
                            alias = %p.alias,
                            addr = %p.address,
                            self_addr = %self_addr,
                            "peer-sync skip — local_acp 등록됐으나 self 와 다른 reachable 주소 광고(원격 오등록 추정 → 소유권 그 머신에 양보)"
                        );
                        return None;
                    }
                }
            }
            // rc.345 — self peer 와 LOCAL ACP 에이전트는 **동적 설정(env) 기반 self 머신 주소**로
            // 광고한다. self peer 가 자기 DB row(캐시·오염 가능)가 아닌 env 권위 주소로 광고해야
            // 다른 머신이 나를 올바른 주소로 학습한다(zalman 이 자기를 seoul 주소로 광고하던 버그의
            // 핵심 — self 가 p.address 로 광고하던 줄). 순수 원격 병합 peer 만 그들이 광고한 주소를
            // 그대로 전달한다.
            let address = if is_self || is_local_acp_agent {
                self_machine_addr.clone().unwrap_or_else(|| p.address.clone())
            } else {
                p.address.clone()
            };
            Some(RemotePeer {
                alias: p.alias.clone(),
                public_key_hex: p.public_key_hex.clone(),
                eth_address: eth,
                gui_address: derive_gui_url(&address),
                address,
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
    let peer_table_urls = reachable_peer_addresses(data_dir)?;
    // Part 1 — env 부트스트랩 URL 을 합쳐 등록 원격 머신(예: zalman)을 시드한다.
    //   self 머신 주소(env 권위)는 reachable_remote_peers 와 동일 우선순위로 읽어
    //   host 만 추출 → 자기 자신 pull 제외에 사용한다.
    let self_machine_addr = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
        .ok()
        .filter(|u| !u.trim().is_empty() && !is_unreachable_address(u))
        .or_else(|| {
            std::env::var("XGRAM_SELF_ADDRESS")
                .ok()
                .filter(|u| !u.trim().is_empty() && !is_unreachable_address(u))
        });
    let self_host = self_machine_addr.as_deref().and_then(url_host);
    let bootstrap_env = std::env::var("XGRAM_PEER_BOOTSTRAP_URLS").ok();
    let candidates =
        combine_pull_candidates(peer_table_urls, bootstrap_env.as_deref(), self_host.as_deref());
    if candidates.is_empty() {
        tracing::debug!("peer-sync tick: pull 후보 0 — skip");
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
                // allow_create=true — pull 대상은 reachable_peer_addresses 가 이미
                //   `agent_profiles.source='user'`(등록 머신)으로 좁힌 신뢰 경로다.
                //   따라서 등록된 원격 머신의 신규 에이전트도 routable peer 로 흡수(CREATE)한다.
                match merge_remote_peers(&mut db, &remote, self_eth.as_deref(), true) {
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
        // update-only 정책 — merge 대상은 이미 존재해야 UPDATE 됨. "good" 만 pre-seed.
        //   localhost/0.0.0.0 는 주소 단계에서 먼저 거부되므로 pre-seed 와 무관하게 skip.
        {
            let mut store = PeerStore::new(&mut db);
            store
                .add_with_eth(
                    "good",
                    "02seed-good",
                    "http://100.101.237.9:47300",
                    Some("0xbbb1"),
                    PeerRole::Worker,
                    None,
                )
                .unwrap();
        }
        let remote = vec![
            rp("local", "http://127.0.0.1:47300", "0xaaa1"),
            rp("zero", "http://0.0.0.0:47300", "0xaaa2"),
            rp("good", "http://100.101.237.9:47300", "0xbbb1"),
        ];
        // allow_create=false — 미존재 흡수 차단 정책 하에서도 localhost/0.0.0.0 는
        //   주소 단계에서 먼저 거부됨을 검증.
        let merged = merge_remote_peers(&mut db, &remote, None, false).unwrap();
        assert_eq!(merged, 1, "localhost/0.0.0.0 는 거부, 기존 reachable 1 개만 UPDATE");
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xbbb1").unwrap().is_some());
        // localhost peer 는 흡수 안 됨 (주소 거부 + 미존재).
        assert!(store.get_by_eth_address("0xaaa1").unwrap().is_none());
    }

    #[test]
    fn merge_skips_self() {
        let mut db = open_mem_db();
        // update-only — "other" 를 pre-seed 해 기존 상태로 만든 뒤 merge → UPDATE 검증.
        //   "me"(self) 는 self_eth 단계에서 거부되므로 pre-seed 안 함.
        {
            let mut store = PeerStore::new(&mut db);
            store
                .add_with_eth(
                    "other",
                    "02seed-other",
                    "http://100.64.0.6:47300",
                    Some("0xOTHER"),
                    PeerRole::Worker,
                    None,
                )
                .unwrap();
        }
        let remote = vec![
            rp("me", "http://100.64.0.5:47300", "0xSELF"),
            rp("other", "http://100.64.0.6:47300", "0xOTHER"),
        ];
        // allow_create=true 라도 self-eth 가드는 항상 우선 — self 는 절대 흡수 안 됨.
        let merged = merge_remote_peers(&mut db, &remote, Some("0xself"), true).unwrap();
        assert_eq!(merged, 1, "self(eth 동일, 대소문자 무시) 제외, other 1 개만 병합");
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xOTHER").unwrap().is_some());
        // self 는 흡수 안 됨 (allow_create 여부와 무관).
        assert!(store.get_by_eth_address("0xSELF").unwrap().is_none());
    }

    #[test]
    fn merge_is_idempotent() {
        let mut db = open_mem_db();
        // update-only — "dup" 을 pre-seed 한 뒤 두 번 merge. 매번 기존 row UPDATE → 중복 없음.
        {
            let mut store = PeerStore::new(&mut db);
            store
                .add_with_eth(
                    "dup",
                    "02seed-dup",
                    "http://100.64.0.7:47300",
                    Some("0xdup"),
                    PeerRole::Worker,
                    None,
                )
                .unwrap();
        }
        let remote = vec![rp("dup", "http://100.64.0.7:47300", "0xdup")];
        // allow_create=true — 두 번 호출해도 eth→pubkey 키 UPSERT 라 row 중복 없음.
        let a = merge_remote_peers(&mut db, &remote, None, true).unwrap();
        let b = merge_remote_peers(&mut db, &remote, None, true).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 1, "재실행해도 row 중복 안 생김 (UPSERT)");
        let mut store = PeerStore::new(&mut db);
        let all = store.list().unwrap();
        assert_eq!(all.iter().filter(|p| p.alias == "dup").count(), 1);
    }

    #[test]
    fn merge_skips_unknown_peers_when_create_disallowed() {
        // 마스터 룰(rc.347) — allow_create=false 면 미존재 alias/eth 는 CREATE 안 함.
        //   (미등록 머신 gossip 자동흡수 방지 경로.)
        let mut db = open_mem_db();
        let remote = vec![rp("stranger", "http://100.64.0.8:47300", "0xstranger")];
        let merged = merge_remote_peers(&mut db, &remote, None, false).unwrap();
        assert_eq!(merged, 0, "미존재 peer 는 흡수 안 됨 (allow_create=false, update-only)");
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xstranger").unwrap().is_none());
        assert!(store.get_by_alias("stranger").unwrap().is_none());
    }

    #[test]
    fn merge_creates_when_allowed_with_gossiped_origin_address() {
        // 핵심 fix — allow_create=true(등록 머신만 pull 하는 신뢰 경로)면 미존재 peer 도
        //   CREATE 된다. 그리고 생성된 row 의 주소는 self 주소가 아니라 **원격이 광고한
        //   origin 주소**여야 한다(cross-machine 라우팅이 zalman 주소로 도달하도록).
        let mut db = open_mem_db();
        // pre-seed 없음 — 처음 보는 zalman 에이전트를 시뮬레이트.
        let zalman_addr = "http://100.80.35.17:17400";
        let remote = vec![rp("Zalman", zalman_addr, "0xZALMAN")];
        // self_eth 는 다른 신원 — self-eth 가드에 안 걸림.
        let merged = merge_remote_peers(&mut db, &remote, Some("0xseoul"), true).unwrap();
        assert_eq!(merged, 1, "등록 머신의 미존재 peer 는 allow_create=true 로 CREATE 됨");
        let mut store = PeerStore::new(&mut db);
        let created = store
            .get_by_eth_address("0xZALMAN")
            .unwrap()
            .expect("zalman peer 가 생성돼야 함");
        // 결정적 검증 — 생성된 주소가 원격 origin 주소(zalman) 이지 self/seoul 주소가 아님.
        assert_eq!(
            created.address, zalman_addr,
            "생성된 peer 의 주소는 원격이 광고한 origin(zalman) 이어야 함, self 가 아님"
        );
        assert_eq!(
            created.eth_address.as_deref(),
            Some("0xZALMAN"),
            "eth 신원 보존"
        );
    }

    #[test]
    fn merge_create_preserves_safety_filters_even_when_allowed() {
        // allow_create=true 라도 안전 필터(unreachable·eth/pubkey 누락·self-eth)는 유지.
        let mut db = open_mem_db();
        let mut unreachable = rp("loop", "http://127.0.0.1:17400", "0xLOOP");
        let mut no_eth = rp("noeth", "http://100.80.35.17:17400", "");
        no_eth.eth_address = "".to_string();
        let self_peer = rp("selfp", "http://100.80.35.17:17400", "0xMINE");
        // pubkey 누락 케이스.
        let mut no_pub = rp("nopub", "http://100.80.35.17:17400", "0xNOPUB");
        no_pub.public_key_hex = "".to_string();
        // (위 unreachable 는 mut 불필요하지만 일관성 유지)
        let _ = &mut unreachable;
        let _ = &mut no_eth;
        let remote = vec![unreachable, no_eth, self_peer, no_pub];
        let merged = merge_remote_peers(&mut db, &remote, Some("0xMINE"), true).unwrap();
        assert_eq!(
            merged, 0,
            "allow_create=true 라도 unreachable/eth·pubkey 누락/self-eth 는 전부 skip"
        );
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_alias("loop").unwrap().is_none());
        assert!(store.get_by_alias("noeth").unwrap().is_none());
        assert!(store.get_by_eth_address("0xMINE").unwrap().is_none());
        assert!(store.get_by_alias("nopub").unwrap().is_none());
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
        // 같은 base host(100.64.0.5) 에 두 peer 가 있었다. update-only 정책상
        //   초기 상태는 직접 add 로 구성 (merge 는 더 이상 신규 생성 안 함).
        {
            let mut store = PeerStore::new(&mut db);
            store
                .add_with_eth("alive", "02seed-alive", "http://100.64.0.5:47300", Some("0xALIVE"), PeerRole::Worker, None)
                .unwrap();
            store
                .add_with_eth("dead", "02seed-dead", "http://100.64.0.5:47300", Some("0xDEAD"), PeerRole::Worker, None)
                .unwrap();
            // 다른 머신 peer — prune 대상 아님.
            store
                .add_with_eth("other_host", "02seed-other", "http://100.64.0.9:47300", Some("0xOTHER"), PeerRole::Worker, None)
                .unwrap();
        }

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
        // update-only 정책상 self peer 도 직접 add 로 초기 구성.
        {
            let mut store = PeerStore::new(&mut db);
            store
                .add_with_eth("me", "02seed-me", "http://100.64.0.5:47300", Some("0xSELF"), PeerRole::Worker, None)
                .unwrap();
        }
        // 원격이 self 를 광고 안 해도 (빈 목록) self 는 보호.
        let pruned =
            prune_absent_from_remote(&mut db, "http://100.64.0.5:47300", &[], Some("0xself"))
                .unwrap();
        assert_eq!(pruned, 0, "self peer 는 prune 금지");
        let mut store = PeerStore::new(&mut db);
        assert!(store.get_by_eth_address("0xSELF").unwrap().is_some());
    }

    #[test]
    fn combine_includes_bootstrap_env_url() {
        // env 부트스트랩 URL 이 후보로 포함된다 (peers 테이블이 비어도).
        let out = combine_pull_candidates(
            vec![],
            Some("http://100.80.35.17:17400"),
            None,
        );
        assert_eq!(out, vec!["http://100.80.35.17:17400".to_string()]);
    }

    #[test]
    fn combine_excludes_self_host() {
        // env 안에 self host(seoul) URL 이 있으면 자기 자신 pull 방지로 제외.
        let out = combine_pull_candidates(
            vec![],
            Some("http://100.101.237.9:47300, http://100.80.35.17:17400"),
            Some("100.101.237.9"),
        );
        assert_eq!(
            out,
            vec!["http://100.80.35.17:17400".to_string()],
            "self host(100.101.237.9) 후보는 제외, zalman 만 남음"
        );
    }

    #[test]
    fn combine_excludes_unreachable() {
        // loopback/unspecified 후보는 제외.
        let out = combine_pull_candidates(
            vec!["http://127.0.0.1:47300".to_string()],
            Some("http://0.0.0.0:17400  http://100.80.35.17:17400"),
            None,
        );
        assert_eq!(
            out,
            vec!["http://100.80.35.17:17400".to_string()],
            "loopback/0.0.0.0 는 제외"
        );
    }

    #[test]
    fn combine_dedupes_same_url_in_table_and_env() {
        // peers 테이블 + env 에 같은 URL 이 있으면 1회만 등장 (trailing '/' 무시).
        let out = combine_pull_candidates(
            vec!["http://100.80.35.17:17400".to_string()],
            Some("http://100.80.35.17:17400/"),
            None,
        );
        assert_eq!(
            out,
            vec!["http://100.80.35.17:17400".to_string()],
            "중복 URL 은 1회만 (정규화 dedupe)"
        );
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
