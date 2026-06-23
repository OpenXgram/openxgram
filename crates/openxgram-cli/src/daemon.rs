//! xgram daemon — Phase 1 first PR: scheduler + transport server foreground.
//!
//! Phase 1 first PR: 단일 binary 가 foreground 로 실행. 종료 신호(Ctrl-C)
//! 까지 대기. systemd unit·fork·pid 파일 등은 후속 PR.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use openxgram_core::paths::db_path;
use openxgram_scheduler::{add_reflection_job, build_scheduler, NIGHTLY_REFLECTION_CRON};
use openxgram_transport::{spawn_server_with_peer_provider, MetricsProvider};
use std::sync::Arc;

use openxgram_core::ports::{GUI_PORT, RPC_PORT};

/// 기본 transport bind (loopback:RPC_PORT) — ports.rs SSOT 파생.
fn default_bind() -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), RPC_PORT)
}
/// 기본 GUI bind (loopback:GUI_PORT) — ports.rs SSOT 파생.
fn default_gui_bind() -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), GUI_PORT)
}

/// rc.244 zero-touch — transport URL(http://host:PORT) 에서 GUI URL 파생 (포트 +2 규약).
///   47300→47302, 17400→17402. 자동 등록 시 gui_address 를 채워 cross-machine 터미널
///   proxy 가 수동 교정 없이 동작하게 한다. 파싱 실패 시 None.
fn derive_gui_url(transport_url: &str) -> Option<String> {
    let idx = transport_url.rfind(':')?;
    let (head, rest) = transport_url.split_at(idx);
    let port: u16 = rest[1..].trim_end_matches('/').parse().ok()?;
    Some(format!("{head}:{}", port + 2))
}

/// 이 머신의 cross-machine reachable transport URL 을 계산한다.
///
/// 우선순위 (모두 도달 가능 주소만 채택, `127.0.0.1`/`0.0.0.0` 절대 금지):
///   ① env `XGRAM_TRANSPORT_PUBLIC_URL` (운영자 override) — 단 unreachable 이면 무시
///   ② install-manifest `machine.tailscale_ip` + port
///   ③ 동적 검출: `tailscale ip --4` → LAN IPv4 (`self_reachable_url`)
///
/// 이는 peer_send.rs 의 sender_transport_url 계산(rc.221)과 같은 로직을 공유하여,
/// 등록(register/retroactive)과 ACK 경로가 동일한 self 주소를 쓰게 한다.
/// 전부 실패하면 None — 호출측이 명시 로깅 후 localhost 폴백 여부를 결정.
fn compute_self_reachable_url(data_dir: &std::path::Path, port: u16) -> Option<String> {
    // ① env override (단 도달 불가 주소는 거부 — 옛 오염 env 무시)
    if let Ok(u) = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL") {
        if !u.is_empty() && !openxgram_transport::tailscale::is_unreachable_address(&u) {
            return Some(u);
        }
    }
    // ② manifest tailscale_ip
    if let Ok(m) =
        openxgram_manifest::InstallManifest::read(openxgram_core::paths::manifest_path(data_dir))
    {
        if let Some(ip) = m.machine.tailscale_ip.filter(|s| !s.is_empty()) {
            return Some(format!("http://{ip}:{port}"));
        }
    }
    // ③ 동적 검출 (tailscale → LAN)
    openxgram_transport::tailscale::self_reachable_url(port)
}

#[derive(Debug, Clone)]
pub struct DaemonOpts {
    pub data_dir: PathBuf,
    pub bind_addr: Option<SocketAddr>,
    /// GUI HTTP API (`/v1/gui/*`) bind. None 이면 비활성화.
    /// `Some(addr)` 면 해당 주소에 axum 서버 별도 가동 — Tauri 데스크톱 앱·기타 클라이언트용.
    pub gui_bind: Option<SocketAddr>,
    pub reflection_cron: Option<String>,
    /// true 시 `tailscale ip --4` 결과를 기본 bind IP 로 사용. mTLS 는 WireGuard
    /// 터널이 네트워크 레이어에서 제공.
    pub tailscale: bool,
}

pub async fn run_daemon(opts: DaemonOpts) -> Result<()> {
    // 싱글톤 가드 — data_dir 당 데몬 1개. 포트 바인딩 *전에* flock 획득.
    //   살아있는 데몬이 이미 점유 중이면 AlreadyRunning 에러로 비정상 종료(중복 기동 차단).
    //   `_daemon_lock` 은 run_daemon 스코프 전체에서 보유해야 락이 유지된다(드롭 금지).
    let _daemon_lock = crate::daemon_singleton::DaemonLock::acquire(&opts.data_dir)?;

    // rc.253 — 데몬 subprocess(tmux 등)가 launchd/systemd 의 빈약한 PATH 에서도 도구를
    //   찾도록 공통 bin 경로 보강. macOS Homebrew(/opt/homebrew/bin)·/usr/local/bin 등.
    //   이게 없으면 macOS launchd 데몬이 `tmux` 를 못 찾아 세션 탐지가 claude 로 폴백했음
    //   (마스터: "설치하면 자동으로 tmux 목록이 나와야"). 설치만 하면 자동 탐지되게.
    {
        let cur = std::env::var("PATH").unwrap_or_default();
        let extra = [
            "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin",
            "/data/data/com.termux/files/usr/bin", // Android Termux
        ];
        let missing: Vec<&str> = extra.iter().copied().filter(|p| !cur.split(':').any(|c| c == *p)).collect();
        if !missing.is_empty() {
            std::env::set_var("PATH", format!("{}:{}", missing.join(":"), cur));
        }
    }

    // rc.117 — daemon 첫 시작 시 ~/oxg.md + 전역 CLAUDE.md @~/oxg.md reference 자동 setup.
    // install.sh / cargo build / 무관하게 OpenXgram 깔리는 순간 자동 setup. idempotent.
    if let Err(e) = crate::mcp_install::setup_oxg_md() {
        tracing::warn!(error = %e, "oxg.md setup 실패 — 수동: xgram identity inject");
    }

    let bind = if let Some(addr) = opts.bind_addr {
        addr
    } else if opts.tailscale {
        let ip = openxgram_transport::tailscale::local_ipv4()
            .context("--tailscale 요청 — `tailscale ip --4` 실패")?;
        SocketAddr::new(std::net::IpAddr::V4(ip), RPC_PORT)
    } else {
        default_bind()
    };
    let cron = opts
        .reflection_cron
        .unwrap_or_else(|| NIGHTLY_REFLECTION_CRON.to_string());

    println!("xgram daemon");
    println!("  data_dir         : {}", opts.data_dir.display());
    println!("  transport bind   : {bind}");
    println!("  reflection cron  : {cron}");

    let mut scheduler = build_scheduler().await.context("scheduler 생성 실패")?;
    add_reflection_job(&mut scheduler, &cron, db_path(&opts.data_dir))
        .await
        .context("reflection job 등록 실패")?;
    // UI-MESSENGER-SPEC v1.4 §20 W-4/W-9 — workflows.cron_expr 트리거는
    // daemon_workers.rs 의 workflow_cron_tick (60s poll + dedup) 가 단일 정본.
    // 과거 boot-time tokio-cron 등록(register_workflow_cron_jobs)은 같은 워크플로를
    // 이중 발화(USDC 이중 지출)시켜 제거됨 — workflow_cron_tick 으로 일원화.
    scheduler.start().await.context("scheduler 시작 실패")?;
    println!("  ✓ reflection scheduler started");

    // Prometheus metrics provider — DB 카운트 매 scrape 마다 조회
    let data_dir_for_metrics = opts.data_dir.clone();
    let metrics: MetricsProvider = Arc::new(move || gather_db_metrics(&data_dir_for_metrics));
    // rc.263 — cross-machine peer-sync gossip: `GET /v1/peers/reachable` provider.
    // 자기 DB 의 reachable peer(localhost 제외 + eth/pubkey 보유)를 RemotePeer→DTO 로 매핑.
    // transport 크레이트가 openxgram-db/peer 에 무의존(저수준)이므로 closure 주입(순환 방지).
    let data_dir_for_peers = opts.data_dir.clone();
    let peer_provider: openxgram_transport::ReachablePeerProvider = Arc::new(move || {
        match crate::daemon_peer_sync::reachable_remote_peers(&data_dir_for_peers) {
            Ok(peers) => peers
                .into_iter()
                .map(|p| openxgram_transport::ReachablePeerDto {
                    alias: p.alias,
                    public_key_hex: p.public_key_hex,
                    eth_address: p.eth_address,
                    address: p.address,
                    gui_address: p.gui_address,
                    role: p.role,
                    display_name: p.display_name,
                    // rc.370 — 홈 권위 세션상태 전파(원격 머신의 acp:acp-1 날조 교정용).
                    session_identifier: p.session_identifier,
                    session_status: p.session_status,
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "peers/reachable provider: DB 읽기 실패 (빈 목록 반환)");
                Vec::new()
            }
        }
    });
    let server = spawn_server_with_peer_provider(bind, Some(metrics), Some(peer_provider))
        .await
        .context("transport server bind 실패")?;
    println!("  ✓ transport server bound: http://{}", server.bound_addr);

    // GUI HTTP API (`/v1/gui/*`) — Tauri 데스크톱 앱이 원격 daemon 데이터에 접근.
    // 별도 axum 서버, transport 와 다른 포트. 토큰 인증 (mcp_tokens 재사용).
    let gui_bind = opts
        .gui_bind
        .unwrap_or_else(default_gui_bind);
    crate::daemon_gui::spawn_gui_server(opts.data_dir.clone(), gui_bind)
        .await
        .context("GUI HTTP API 서버 가동 실패")?;

    // rc.137 — session cache background warming. 30초마다 collect → cache 갱신.
    // endpoint 는 항상 cache 즉시 반환 → 5s TTL 만료로 GUI 사이드바가 빈 화면 되는 문제 해결.
    crate::daemon_gui_sessions::spawn_session_warming();
    println!("  ✓ session cache warming spawned (30s interval)");

    // UI-MESSENGER-SPEC v1.3 enforcement workers (M-4 idle, M-6 auto-topup, L6 expiry, V6 outbound).
    crate::daemon_workers::spawn_all_from_dir(opts.data_dir.clone())
        .context("messenger enforcement workers 가동 실패")?;
    println!("  ✓ messenger workers (M-4 / M-6 / L6 / V6) spawned");

    // rc.201 — auto-seed: 자기 머신 tmux session 을 agent_capabilities + peer 자동 등록.
    // 마스터의 본질: "peer = 터미널". daemon 가 자기 tmux 의 active session 을 자동 agent 등록
    // → 마스터가 manual GUI toggle 안 해도 됨 → 진정한 peer-per-terminal architecture.
    match auto_seed_local_tmux_agents(&opts.data_dir) {
        Ok(n) if n > 0 => println!("  ✓ rc.201 auto-seed: {n} tmux session → agent + sub-keystore + peer 자동 등록"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "rc.201 auto-seed 실패 (계속)"),
    }

    // 기본 동봉(built-in) 특수에이전트 seed — xgram-ops(OpenXgram 운영 전담).
    // 설치됨·미활성(activated=0)으로 등록 → 마스터가 GUI 활성화 버튼으로 동작. idempotent.
    match seed_builtin_agents(&opts.data_dir) {
        Ok(n) if n > 0 => println!("  ✓ built-in 에이전트 seed: {n}개 (xgram-ops 등, 미활성 — GUI에서 활성화)"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "built-in 에이전트 seed 실패 (계속)"),
    }

    // rc.315 — per-머신 운영 에이전트 보장: `<machine>-master`.
    // 머신마다 워크플로우/운영 소유 에이전트 1개가 항상 존재하도록 daemon boot 시 UPSERT.
    // 머신 식별은 daemon 의 detect_machine() (= /v1/gui/sessions 의 machine 소스)과 동일,
    // slug 화(소문자·비영숫자→'-'·trim)하여 cross-machine name collision 방지.
    // 레거시 xgram-ops 는 (충돌 없을 때만) <slug>-master 로 rename 마이그레이션.
    {
        // 머신 alias — SSOT(machine_id). data-dir 영속값(manifest→캐시) → env → hostname.
        //   같은 data-dir 의 모든 프로세스(daemon·mcp-serve 등)가 항상 동일 머신명을 얻는다.
        //   ops 에이전트 slug 에만 영향(peer 신원 무관).
        let machine_name = crate::machine_id::machine_alias(&opts.data_dir);
        let slug = machine_slug(&machine_name);
        match ensure_machine_master(&opts.data_dir, &slug) {
            Ok(alias) => {
                println!("  ✓ rc.316 ops 에이전트 보장: {alias} (머신 '{machine_name}' → slug '{slug}')");
                tracing::info!(alias = %alias, machine = %machine_name, slug = %slug, "per-machine ops agent ensured");
            }
            Err(e) => tracing::warn!(error = %e, slug = %slug, "rc.315 per-machine ops agent 보장 실패 (계속)"),
        }
    }

    // rc.268 — auto-seed 주기 tick: rc.201 auto-seed 는 startup 1회뿐이라, daemon 시작 이후
    // 새로 만든 tmux LLM 세션이 재시작 전까지 안 뜨던 본질 결함을 fix. 30초마다 재실행하여
    // 새 LLM tmux 세션을 재시작 없이 자동 등록 (auto_seed_local_tmux_agents 재사용 — INSERT OR IGNORE 라 idempotent).
    {
        let seed_data_dir = opts.data_dir.clone();
        // rc.269 gap#2 — auto-seed tick 이 agent_capabilities 만 채우고 peer row 는 안 만들던 결함 fix.
        // auto-seed 직후 retroactive_register_agents 를 재실행하여, 새 tmux LLM 세션이 재시작 없이
        // 30초 내 로스터(peers)에도 등재되게 한다. peer 생성 로직은 retroactive_register_agents 재사용
        // (keystore + PeerStore::add_with_eth + session_identifier) — 중복 구현 금지.
        let seed_bind_port = bind.port();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await; // 첫 tick 즉시 발화 회피 — startup 에서 이미 1회 실행됨.
            loop {
                interval.tick().await;
                let dd = seed_data_dir.clone();
                let seeded = match tokio::task::spawn_blocking(move || auto_seed_local_tmux_agents(&dd)).await {
                    Ok(Ok(n)) => {
                        if n > 0 {
                            tracing::info!(seeded = n, "rc.268 auto-seed tick: 새 tmux LLM 세션 자동 등록");
                        }
                        n
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "rc.268 auto-seed tick 실패 (계속)");
                        0
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "rc.268 auto-seed tick join 실패 (계속)");
                        0
                    }
                };
                // rc.269 gap#2 — peer row 생성 (재시작 불요). agent_capabilities 에 새로 들어온
                // (또는 아직 peer 없는) agent 를 peers 로스터에 등재. retroactive 자체가 idempotent
                // (이미 있는 peer 는 skip) 라 seeded==0 여도 안전하지만, 신규 seed 가 있을 때만 호출하여
                // tmux list/keystore 비용 절약.
                if seeded > 0 {
                    let dd2 = seed_data_dir.clone();
                    match tokio::task::spawn_blocking(move || retroactive_register_agents(&dd2, seed_bind_port)).await {
                        Ok(Ok(r)) if r > 0 => {
                            tracing::info!(registered = r, "rc.269 auto-seed tick: 새 agent → peer 로스터 등재")
                        }
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => tracing::warn!(error = %e, "rc.269 auto-seed tick retroactive 실패 (계속)"),
                        Err(e) => tracing::warn!(error = %e, "rc.269 auto-seed tick retroactive join 실패 (계속)"),
                    }
                }
            }
        });
        println!("  ✓ rc.268/269 auto-seed tick (30s) — 새 tmux LLM 세션 재시작 없이 capability + peer 로스터 자동 등록");
    }

    // rc.196 — retroactive register: messenger_enabled=1 인데 peer entry 없는 옛 agent 들의
    // sub-keystore + peer 자동 생성. 마스터의 portal/akashic 등 ui 토글만 켜고 rc.192 fix 이전
    // 등록된 agent 들이 mock 상태였던 본질 결함 해결.
    match retroactive_register_agents(&opts.data_dir, bind.port()) {
        Ok(n) if n > 0 => println!("  ✓ rc.196 retroactive: {n} 옛 agent → verified peer 자동 등록"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "rc.196 retroactive register 실패 (계속 진행)"),
    }

    // cross-machine peer registry sync (gossip) — reachable agent 목록 주기 교환(60s).
    // 각 데몬이 fleet 전체의 reachable agent 를 알게 되어 직접 연결을 가능케 함.
    // 자세한 설계·범위·후속 endpoint 노트는 daemon_peer_sync 모듈 doc 참조.
    crate::daemon_peer_sync::spawn_peer_sync(opts.data_dir.clone());
    println!("  ✓ peer-sync spawned (cross-machine registry gossip, 60s interval)");

    // inbound processor — 1초 주기로 server.drain_received() 한 후 envelope.from 매칭으로
    // peer.touch_by_eth_address. 매칭 실패는 silent (anonymous envelope 도 정상 도착).
    // rc.190 — 명시 logging 추가 (마스터 본질 fix). zalman 측 process_inbound 호출 안 되는 root cause 식별.
    let data_dir_clone = opts.data_dir.clone();
    let received_arc = std::sync::Arc::new(server);
    let received_for_task = received_arc.clone();
    let processor = tokio::spawn(async move {
        tracing::info!("inbound_processor: task spawned, 1s polling start");
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut tick_count: u64 = 0;
        loop {
            interval.tick().await;
            tick_count += 1;
            // rc.238 — 매 tick 마다 health 의 last_inbound_tick update. 외부 watchdog 가 stuck 감지.
            // drain + process 전에 mark → tick 자체가 돌고 있음을 보장 (process 가 느려도 tick 진행).
            received_for_task.mark_inbound_tick();
            let envelopes = received_for_task.drain_received();
            if tick_count % 60 == 0 {
                tracing::debug!(tick=tick_count, "inbound_processor: heartbeat (no envelopes recently)");
            }
            if envelopes.is_empty() {
                continue;
            }
            tracing::info!(count=envelopes.len(), tick=tick_count, "inbound_processor: envelopes drained, calling process_inbound");
            // rc.238 — process_inbound 는 내부에서 envelope 별 독립 처리 + hang point(tmux/ACK send)
            // 마다 timeout 적용. 따라서 batch 전체가 hang 하지 않음. 결과는 envelope 단위 skip 누적.
            // process_inbound 자체는 block_in_place 사용 → 여기 async task 의 worker thread 에서 직접 호출
            // (spawn_blocking 안에서는 block_in_place 가 panic 하므로 직접 호출 유지).
            match process_inbound(&data_dir_clone, &envelopes) {
                Ok(_) => tracing::info!(count=envelopes.len(), "inbound_processor: process_inbound 성공"),
                Err(e) => tracing::warn!(error = %e, "inbound_processor: process_inbound 실패 (tick 계속)"),
            }
        }
    });
    tracing::info!("inbound_processor: spawn complete (main thread)");
    println!("  ✓ inbound processor running (1s interval)");

    // Discord inbound listener — rc.92: 멀티 봇 지원.
    // 1) notify.toml.discord.bot_token (default 봇, 옛 single-bot 호환)
    // 2) discord_bots 테이블 (멀티 봇, 채널별 다른 봇)
    // 각 봇마다 독립 Gateway connection spawn. Keypair 가 Clone 안 되므로 매 bot 마다 keystore 재로드.
    let mut _discord_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let keystore_pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").ok();
    let load_master = |dir: &std::path::Path| -> Option<openxgram_keystore::Keypair> {
        let pw = keystore_pw.as_ref()?;
        use openxgram_keystore::Keystore;
        let ks = openxgram_keystore::FsKeystore::new(openxgram_core::paths::keystore_dir(dir));
        ks.load(openxgram_core::paths::MASTER_KEY_NAME, pw).ok()
    };
    // (1) discord_bots 테이블 (multibot) — 우선
    let extra_bots: Vec<(String, String)> = {
        let path = opts.data_dir.join("db.sqlite");
        match openxgram_db::Db::open(openxgram_db::DbConfig { path, ..Default::default() }) {
            Ok(mut db) => db.conn().prepare(
                "SELECT alias, bot_token FROM discord_bots WHERE active = 1"
            ).and_then(|mut s| {
                s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                 .and_then(|m| m.collect::<rusqlite::Result<Vec<_>>>())
            }).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    };
    // (2) notify.toml default 봇 — multibot 비어있을 때만 fallback.
    //     multibot 의 한 봇이 같은 token 이면 spawn 중복 → Discord Gateway 4004 충돌 발생하므로 skip.
    let multibot_tokens: std::collections::HashSet<String> =
        extra_bots.iter().map(|(_, t)| t.clone()).collect();
    if let Ok(cfg) = crate::notify_setup::NotifyConfig::load(Some(&opts.data_dir)) {
        if let Some(d) = cfg.discord {
            if !d.bot_token.is_empty() && !multibot_tokens.contains(&d.bot_token) {
                let dir = opts.data_dir.clone();
                let token = d.bot_token.clone();
                let key = load_master(&opts.data_dir);
                let handle = tokio::spawn(async move {
                    if let Err(e) = crate::notify::run_discord_inbound_for_daemon(dir, token, key).await {
                        tracing::warn!(error = %e, "discord default listener 종료");
                    }
                });
                _discord_handles.push(handle);
                println!("  ✓ discord listener spawned (default, notify.toml)");
            } else if !d.bot_token.is_empty() {
                println!("  ↩ discord default listener skip — same token already spawned via discord_bots");
            }
        }
    }
    for (alias, token) in extra_bots {
        let dir = opts.data_dir.clone();
        let key = load_master(&opts.data_dir);
        let alias_clone = alias.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = crate::notify::run_discord_inbound_for_daemon(dir, token, key).await {
                tracing::warn!(alias = %alias_clone, error = %e, "discord extra listener 종료");
            }
        });
        _discord_handles.push(handle);
        println!("  ✓ discord listener spawned (bot: {alias})");
    }
    if _discord_handles.is_empty() {
        tracing::info!("discord inbound skip — bot token 없음 (notify.toml + discord_bots 둘 다 비어있음)");
    }

    // Nostr inbound processor (PRD-NOSTR-10) — XGRAM_NOSTR_RELAYS env 가 설정된 경우만 활성.
    // master keystore 패스워드는 XGRAM_NOSTR_PASSWORD env 에서 로드 (없으면 skip).
    let (nostr_shutdown_tx, nostr_handle) = match crate::nostr_inbound::NostrInboundConfig::from_env(
        opts.data_dir.clone(),
    ) {
        Some(cfg) => match std::env::var("XGRAM_NOSTR_PASSWORD") {
            Ok(pw) => {
                use openxgram_keystore::Keystore;
                let ks = openxgram_keystore::FsKeystore::new(openxgram_core::paths::keystore_dir(
                    &opts.data_dir,
                ));
                match ks.load(openxgram_core::paths::MASTER_KEY_NAME, &pw) {
                    Ok(master) => {
                        let (tx, rx) = tokio::sync::watch::channel(false);
                        let handle =
                            crate::nostr_inbound::spawn_nostr_inbound_processor(cfg, master, rx)
                                .await
                                .context("nostr inbound processor 시작 실패")?;
                        println!(
                            "  ✓ nostr inbound processor running ({} relay(s))",
                            handle_relay_count()
                        );
                        (Some(tx), Some(handle))
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "nostr inbound — master 로드 실패 (skip)");
                        (None, None)
                    }
                }
            }
            Err(_) => {
                tracing::info!("XGRAM_NOSTR_RELAYS 설정됨 — XGRAM_NOSTR_PASSWORD 미설정으로 nostr inbound skip");
                (None, None)
            }
        },
        None => (None, None),
    };

    println!();
    println!("Ctrl-C 로 종료.");

    tokio::signal::ctrl_c().await.context("signal 대기 실패")?;
    println!();
    println!("종료 신호 수신 — shutdown 중...");

    if let Some(tx) = nostr_shutdown_tx {
        let _ = tx.send(true);
    }
    if let Some(h) = nostr_handle {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), h).await;
    }

    scheduler
        .shutdown()
        .await
        .context("scheduler shutdown 실패")?;
    processor.abort();
    if let Ok(server) = std::sync::Arc::try_unwrap(received_arc) {
        server.shutdown();
    }

    println!("✓ daemon stopped");
    Ok(())
}

fn handle_relay_count() -> usize {
    std::env::var("XGRAM_NOSTR_RELAYS")
        .ok()
        .map(|s| s.split(',').filter(|x| !x.trim().is_empty()).count())
        .unwrap_or(0)
}

/// 매 /v1/metrics scrape 마다 호출 — Prometheus exposition format 추가 metrics.
/// 실패 시 빈 문자열 반환 (silent — transport baseline metrics 는 영향 없음).
fn gather_db_metrics(data_dir: &std::path::Path) -> String {
    use openxgram_db::{Db, DbConfig};
    let mut db = match Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    }) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    if db.migrate().is_err() {
        return String::new();
    }
    let conn = db.conn();
    let counts = [
        ("openxgram_messages_total", "messages"),
        ("openxgram_episodes_total", "episodes"),
        ("openxgram_memories_total", "memories"),
        ("openxgram_patterns_total", "patterns"),
        ("openxgram_traits_total", "traits"),
        ("openxgram_vault_entries_total", "vault_entries"),
        ("openxgram_vault_acl_total", "vault_acl"),
        (
            "openxgram_vault_pending_total",
            "vault_pending_confirmations",
        ),
        ("openxgram_vault_audit_total", "vault_audit"),
        ("openxgram_peers_total", "peers"),
        ("openxgram_payment_intents_total", "payment_intents"),
        ("openxgram_mcp_tokens_total", "mcp_tokens"),
    ];
    let mut out = String::new();
    for (metric, table) in counts {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap_or(-1);
        if n < 0 {
            continue;
        }
        out.push_str(&format!(
            "# HELP {metric} 행 수 ({table})\n# TYPE {metric} gauge\n{metric} {n}\n"
        ));
    }
    out
}

/// 데몬 자신의 GUI HTTP base URL. `XGRAM_DAEMON_GUI_URL` 우선, 없으면 loopback 기본.
/// (mcp_serve.rs::daemon_gui_url 과 동일 규약 — 데몬 ACP 레지스트리 self-call 용.)
fn self_gui_url() -> String {
    std::env::var("XGRAM_DAEMON_GUI_URL").unwrap_or_else(|_| format!("http://{}", default_gui_bind()))
}

/// fix④ Option A — peer_send inbound 의 ACP-우선 전달 결과.
///   - Delivered  : ACP 엔드포인트(=ai_type→adapter 보유 내부 에이전트)로 전달 + 영속 완료.
///                  (a2a_send → load_a2a_agent_meta → handle_task 가 conv_key `a2a:{from}->{alias}`
///                   에 me/agent 양측 기록까지 수행 — 기존 spawn 머신리 그대로 재사용.)
///   - NoEndpoint : 그 alias 는 ACP 로 구동 불가(순수 터미널 peer 또는 미등록) → tmux fallback.
///   - Unavailable: ACP 경로 자체를 시도할 수 없음(데몬 토큰 부재·GUI 미응답 등). 명시 로그 후
///                  tmux fallback. **silent X (절대 규칙 1).**
enum AcpInboundOutcome {
    Delivered,
    NoEndpoint,
    Unavailable,
}

/// fix① (A2A double-delivery) — 이 alias 가 **ACP 로 구동 가능한 신원**인지 판정.
/// `load_a2a_agent_meta`(daemon_gui.rs)와 동일 기준: `agent_capabilities` 에 row 가 있고
/// `role != 'tmux'` 이면 ACP-drivable. 이 alias 는 A2A inbound 를 ACP 세션이 **소유**하므로
/// tmux 로는 절대 fallback 하지 않는다(이중 전달·tmux 중복 주입 방지). 순수 터미널 peer
/// (capabilities row 없음 또는 role='tmux')만 tmux 전달 대상.
///
/// 동기 판정(self-call HTTP 결과·90s timeout 과 무관) — 전달 결정의 단일 진리원천.
/// 쿼리 실패/DB 오류 시 false 반환(보수적: 잘못 ACP-drivable 로 분류해 메시지 유실하느니
/// tmux fallback 경로를 살린다). 오류는 명시 로그(절대 규칙 1).
fn is_acp_drivable(db: &mut openxgram_db::Db, alias: &str) -> bool {
    let found: rusqlite::Result<i64> = db.conn().query_row(
        "SELECT 1 FROM agent_capabilities WHERE alias = ?1 AND role IS NOT 'tmux' LIMIT 1",
        [alias],
        |r| r.get(0),
    );
    match found {
        Ok(_) => true,
        Err(rusqlite::Error::QueryReturnedNoRows) => false,
        Err(e) => {
            tracing::warn!(
                alias = %alias,
                error = %e,
                "fix① is_acp_drivable 쿼리 오류 — 보수적으로 false(=tmux 경로 유지). silent X"
            );
            false
        }
    }
}

/// rc.365 — 인바운드 peer 메시지가 **tmux 자동 주입 대상**인지 판정하는 순수 함수.
/// 라우팅 결정의 경계를 한 곳에 모아 단위 테스트 가능하게 한다(daemon.process_inbound 에서 참조).
///
/// 주입 대상 조건(모두 만족):
///   1. 제어 메시지가 아님 — `[AGENT_ADD_REQUEST]`/`[AGENT_ADD_ACCEPT]` prefix 는 대화가 아니라
///      제어 신호 → 주입 금지(이미 process_inbound 가 `continue` 로 차단하지만 경계를 명시).
///   2. 본문이 비어있지 않음 — 빈 ack/keepalive 류 주입 금지(스팸 방지).
///   3. peer tmux 주입이 비활성화되지 않음 — `XGRAM_DISABLE_PEER_TMUX_INJECT` 게이팅.
///   4. 수신자에게 라이브 tmux 세션이 존재 — 사람-구동 수신처가 실제로 살아있을 때만.
///
/// 순수(부수효과 없음) — tmux liveness / disabled 플래그는 호출측이 미리 계산해 넘긴다.
fn should_inject_inbound_to_tmux(body: &str, tmux_session_live: bool, inject_disabled: bool) -> bool {
    if inject_disabled {
        return false;
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return false;
    }
    // 제어 메시지(대화 아님) — auto-inject 대상 아님.
    if trimmed.starts_with("[AGENT_ADD_REQUEST] ") || trimmed.starts_with("[AGENT_ADD_ACCEPT] ") {
        return false;
    }
    tmux_session_live
}

/// rc.366 — 인바운드 peer 메시지의 수신자가 **자동 spawn 가능(spawnable)** 한지 판정.
/// 기준은 `acp_spawn_for_alias`(daemon_gui.rs) 와 **동일**: agent_capabilities⋈agent_profiles 에
/// row 가 있고, `agent_capabilities.project_path`(=ACP cwd)가 비어있지 않으며, `ai_type` 이
/// 알려진 ACP 어댑터(claude/codex/gemini/opencode)로 매핑되어야 spawn 경로(`POST
/// /v1/gui/agents/{alias}/spawn`)가 실제로 성공한다. 기동 정보가 없으면 spawn 불가 → false.
///
/// 동기 판정(self-call HTTP 전). 쿼리 실패/DB 오류는 보수적으로 false(=spawn 안 함 → 인박스
/// 보존). 오류는 명시 로그(절대 규칙 1).
fn is_recipient_spawnable(db: &mut openxgram_db::Db, alias: &str) -> bool {
    let row: rusqlite::Result<(String, String)> = db.conn().query_row(
        "SELECT COALESCE(p.ai_type,'claude'), COALESCE(ac.project_path,'') \
         FROM agent_capabilities ac JOIN agent_profiles p ON p.alias = ac.alias \
         WHERE ac.alias = ?1",
        [alias],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    );
    match row {
        Ok((ai_type, project_path)) => {
            if project_path.trim().is_empty() {
                return false;
            }
            // acp_spawn_for_alias 의 adapter 매핑과 동일 — 알려진 ai_type 만 spawn 가능.
            matches!(
                ai_type.trim().to_ascii_lowercase().as_str(),
                "claude" | "codex" | "gemini" | "opencode"
            )
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => false,
        Err(e) => {
            tracing::warn!(
                alias = %alias,
                error = %e,
                "rc.366 is_recipient_spawnable 쿼리 오류 — 보수적으로 false(=spawn 안 함, 인박스 보존). silent X"
            );
            false
        }
    }
}

/// rc.366 — 죽은 수신자(라이브 tmux 세션 없음)를 자동 spawn 해야 하는지 판정하는 순수 함수.
/// spawn-storm 방지 가드를 한 곳에 모아 단위 테스트 가능하게 한다.
///
/// spawn 대상 조건(모두 만족):
///   1. peer tmux 주입이 비활성화되지 않음 — `XGRAM_DISABLE_PEER_TMUX_INJECT` 게이팅(rc.365 와 동일).
///   2. 본문이 비어있지 않고 제어 메시지가 아님 — should_inject_inbound_to_tmux 와 동일 필터.
///   3. **라이브 tmux 세션이 없음** — 살아있으면 rc.365 주입이 처리하므로 spawn 불필요.
///   4. 수신자가 spawnable — 기동 정보(ai_type+project_path) 보유.
///   5. rate-limit 미해당 — 동일 수신자를 단시간 내 재spawn 금지(storm 방지).
///
/// 순수(부수효과 없음) — tmux liveness / spawnable / rate_limited 는 호출측이 미리 계산해 넘긴다.
fn should_spawn_inbound_recipient(
    body: &str,
    tmux_session_live: bool,
    spawnable: bool,
    inject_disabled: bool,
    rate_limited: bool,
) -> bool {
    if inject_disabled {
        return false;
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("[AGENT_ADD_REQUEST] ") || trimmed.starts_with("[AGENT_ADD_ACCEPT] ") {
        return false;
    }
    // 라이브 tmux 가 있으면 rc.365 주입이 처리 — spawn 불필요.
    if tmux_session_live {
        return false;
    }
    spawnable && !rate_limited
}

/// rc.366 — 자동 spawn rate-limit 맵. 동일 수신자 alias 를 단시간(SPAWN_RATELIMIT_SECS) 내
/// 반복 spawn 하는 storm 을 막는다(WORKTREE_CACHE 의 OnceLock<Mutex<HashMap>> 패턴 재사용).
static SPAWN_RATELIMIT: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::OnceLock::new();

/// 동일 수신자 재spawn 금지 윈도우(초). 새 세션이 기동·등록되어 라이브 tmux 로 잡힐 때까지의
/// 여유. env `XGRAM_PEER_SPAWN_RATELIMIT_SECS` 로 조정 가능(기본 300초=5분).
fn spawn_ratelimit_secs() -> u64 {
    std::env::var("XGRAM_PEER_SPAWN_RATELIMIT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(300)
}

/// rc.366 — `alias` 가 rate-limit 윈도우 안에서 최근 spawn 시도됐는지 확인하고, 아니면
/// 지금 시각으로 마킹한다(check-and-set, 원자적). 반환 true = rate-limited(spawn 금지).
/// 마킹은 **시도 직전**에 한다 — 동시 envelope 의 중복 spawn 도 차단.
fn spawn_recently_attempted(alias: &str) -> bool {
    let map = SPAWN_RATELIMIT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let window = std::time::Duration::from_secs(spawn_ratelimit_secs());
    let now = std::time::Instant::now();
    let mut guard = match map.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(), // 락 poison 시에도 진행(보수적: 마킹은 한다)
    };
    // 오래된 엔트리 정리(맵 무한 성장 방지).
    guard.retain(|_, t| now.duration_since(*t) < window);
    if let Some(last) = guard.get(alias) {
        if now.duration_since(*last) < window {
            return true; // 윈도우 안 — rate-limited.
        }
    }
    guard.insert(alias.to_string(), now);
    false
}

/// rc.366 — 죽은 spawnable 수신자를 깨운다. `try_acp_inbound_delivery` 와 동일하게 데몬
/// self-call(`POST /v1/gui/agents/{alias}/spawn` → gui_agent_spawn → acp_spawn_for_alias)
/// 로 기존 spawn 경로를 재사용한다. 새 spawner 를 만들지 않는다.
///
/// 반환 true = spawn self-call 성공(세션 기동). false = 실패(토큰 부재·HTTP 오류·timeout).
/// **실패해도 메시지는 인박스에 보존**(호출측이 ACP fallback / undelivered 마커로 처리).
async fn try_spawn_recipient(target_alias: &str) -> bool {
    let token = match std::env::var("XGRAM_MCP_TOKEN") {
        Ok(t) if !t.trim().is_empty() => t,
        _ => {
            tracing::warn!(
                target_alias = %target_alias,
                "rc.366 자동 spawn 불가 — XGRAM_MCP_TOKEN 부재(데몬 self-call 인증 필요). 메시지는 인박스 보존."
            );
            return false;
        }
    };
    let url = format!(
        "{}/v1/gui/agents/{}/spawn",
        self_gui_url().trim_end_matches('/'),
        urlencoding::encode(target_alias)
    );
    let client = reqwest::Client::new();
    let send_fut = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send();
    // ACP 세션 spawn(어댑터 프로세스 기동)은 수 초 걸릴 수 있으나 inbound tick 을 무한정
    // 막으면 안 됨 — 60초 상한.
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(60), send_fut).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, target_alias = %target_alias, "rc.366 자동 spawn self-call 요청 실패 — 인박스 보존");
            return false;
        }
        Err(_elapsed) => {
            tracing::warn!(target_alias = %target_alias, "rc.366 자동 spawn self-call TIMEOUT(60s) — 인박스 보존");
            return false;
        }
    };
    let status = resp.status();
    if status.is_success() {
        tracing::info!(
            target_alias = %target_alias,
            "rc.366 죽은 spawnable 수신자 자동 spawn OK — 새 ACP 세션 기동(메시지 ACP 전달로 이어짐)"
        );
        return true;
    }
    let txt = resp.text().await.unwrap_or_default();
    tracing::warn!(
        target_alias = %target_alias,
        http_status = %status,
        body = %txt,
        "rc.366 자동 spawn self-call 비-2xx — 인박스 보존(절대 규칙 1: silent X)"
    );
    false
}

/// fix④ — peer_send inbound 를 ACP/A2A 전달 척추로 보낸다. 데몬은 자기 GUI HTTP 서버
/// (`spawn_gui_server`, 동일 프로세스)의 `/v1/gui/a2a/send` 를 self-call 한다 → a2a_send 가
/// **기존** spawn 머신리(load_a2a_agent_meta + handle_task, lazy ACP 세션 find-or-create)를
/// 그대로 구동하고, 통합 conv_key `a2a:{from}->{alias}` 로 양측 메시지를 영속한다.
/// 새 spawner 를 만들지 않는다 — endpoint="new_acp" 분기만 재사용.
async fn try_acp_inbound_delivery(
    target_alias: &str,
    sender_alias: &str,
    body: &str,
) -> AcpInboundOutcome {
    // 데몬 self-call 토큰. 부재 시 ACP 경로 불가 — bypass 금지(절대 규칙 1) → tmux fallback.
    let token = match std::env::var("XGRAM_MCP_TOKEN") {
        Ok(t) if !t.trim().is_empty() => t,
        _ => {
            tracing::warn!(
                target_alias = %target_alias,
                "fix④ ACP 전달 불가 — XGRAM_MCP_TOKEN 부재(데몬 self-call 인증 필요). tmux fallback 으로 진행"
            );
            return AcpInboundOutcome::Unavailable;
        }
    };
    let url = format!("{}/v1/gui/a2a/send", self_gui_url().trim_end_matches('/'));
    // a2a_send 의 SendBody — endpoint="new_acp"(기본) 내부 alias 경로.
    let payload = serde_json::json!({
        "target": target_alias,
        "from_agent": sender_alias,
        "task": body,
        "endpoint": "new_acp",
    });
    let client = reqwest::Client::new();
    let send_fut = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send();
    // ACP 턴은 길 수 있으나 inbound tick 을 무한정 막으면 안 됨 — 90초 상한.
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(90), send_fut).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, target_alias = %target_alias, "fix④ ACP self-call 요청 실패 — tmux fallback");
            return AcpInboundOutcome::Unavailable;
        }
        Err(_elapsed) => {
            tracing::warn!(target_alias = %target_alias, "fix④ ACP self-call TIMEOUT(90s) — tmux fallback");
            return AcpInboundOutcome::Unavailable;
        }
    };
    let status = resp.status();
    if status.is_success() {
        tracing::info!(
            target_alias = %target_alias,
            sender = %sender_alias,
            "fix④ inbound → ACP/A2A 전달 OK (conv_key a2a:{}->{} 영속)",
            sender_alias, target_alias
        );
        return AcpInboundOutcome::Delivered;
    }
    // 404/422 = 그 alias 는 ACP 로 구동 불가(role=tmux 또는 ai_type/adapter 없음) → tmux fallback.
    if status == reqwest::StatusCode::NOT_FOUND
        || status == reqwest::StatusCode::UNPROCESSABLE_ENTITY
    {
        tracing::info!(
            target_alias = %target_alias,
            http_status = %status,
            "fix④ ACP 엔드포인트 없음(순수 터미널 peer 추정) — tmux fallback 으로 진행"
        );
        return AcpInboundOutcome::NoEndpoint;
    }
    // 그 외 HTTP 오류 — 명시 로그(절대 규칙 1) 후 tmux fallback.
    let txt = resp.text().await.unwrap_or_default();
    tracing::warn!(
        target_alias = %target_alias,
        http_status = %status,
        body = %txt,
        "fix④ ACP self-call 비-2xx — tmux fallback 으로 진행"
    );
    AcpInboundOutcome::Unavailable
}

/// fix④ — ACP 도 tmux 도 전달 못한 envelope 을 **조용히 버리지 않는다(절대 규칙 1)**.
/// 수신자 bare-alias identity 스레드(GUI 리더 daemon_gui.rs:3900/4282 가 읽는 키)에
/// undelivered 마커를 영속(데몬 GUI self-call) + 명시 로그.
/// 사용자/에이전트가 그 스레드에서 미전달 사실을 보고 후속 처리할 수 있게 한다.
async fn record_inbound_undelivered(target_alias: &str, sender_alias: &str, body: &str) {
    let conv_key = target_alias.to_string();
    let marker = format!(
        "⚠️ [미전달] {sender_alias} → {target_alias}: ACP 엔드포인트 없음 + tmux 화면 없음으로 즉시 전달 실패. 원문: {body}"
    );
    let token = match std::env::var("XGRAM_MCP_TOKEN") {
        Ok(t) if !t.trim().is_empty() => Some(t),
        _ => None,
    };
    // self-call 로 acp_messages 스레드에 기록(POST /v1/gui/acp/conversations/{key}/messages).
    if let Some(token) = token {
        let url = format!(
            "{}/v1/gui/acp/conversations/{}/messages",
            self_gui_url().trim_end_matches('/'),
            urlencoding::encode(&conv_key)
        );
        let payload = serde_json::json!({ "role": "system", "text": marker });
        let client = reqwest::Client::new();
        let fut = client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send();
        match tokio::time::timeout(std::time::Duration::from_secs(5), fut).await {
            Ok(Ok(r)) if r.status().is_success() => {
                tracing::warn!(
                    conv_key = %conv_key,
                    sender = %sender_alias,
                    "fix④ inbound 미전달 — 수신자 identity 스레드 '{}'(bare alias)에 undelivered 마커 영속(후속 처리 가능)",
                    target_alias
                );
                return;
            }
            Ok(Ok(r)) => tracing::error!(conv_key = %conv_key, http_status = %r.status(), "fix④ undelivered 마커 영속 실패(비-2xx)"),
            Ok(Err(e)) => tracing::error!(error = %e, conv_key = %conv_key, "fix④ undelivered 마커 영속 요청 실패"),
            Err(_) => tracing::error!(conv_key = %conv_key, "fix④ undelivered 마커 영속 TIMEOUT(5s)"),
        }
    }
    // 영속까지 실패해도 절대 silent X — 최소한 ERROR 로그로 미전달 사실을 남긴다.
    tracing::error!(
        target_alias = %target_alias,
        sender = %sender_alias,
        conv_key = %conv_key,
        "fix④ inbound 미전달 + 스레드 영속도 실패 — 메시지 유실 위험. (XGRAM_MCP_TOKEN/GUI 상태 점검 필요)"
    );
}

/// drain 된 envelope 들을 한 번의 DB open 으로 처리.
/// 각 envelope: peer 조회 → 서명 검증 → L0 message insert → peer.touch (성공 시).
/// 검증 실패·미등록 peer 는 silent drop + WARN (PRD-2.0.1 / 2.0.2 / 2.0.3).
/// rc.181 — Step 2 silent fail 진단 위한 명시 로깅 강화.
pub fn process_inbound(
    data_dir: &std::path::Path,
    envelopes: &[openxgram_transport::Envelope],
) -> Result<()> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_keystore::verify_with_pubkey;
    use openxgram_memory::{message_embedder, MessageStore, SessionStore};
    use openxgram_peer::PeerStore;

    tracing::info!(count = envelopes.len(), "process_inbound: entry");

    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open (inbound) 실패")?;
    tracing::debug!("process_inbound: DB open ok");
    db.migrate().context("DB migrate (inbound) 실패")?;
    tracing::debug!("process_inbound: migrate ok");
    // rc.270 본질 fix — 메시지 전달은 임베더 init 에 강결합되면 안 된다.
    // 이전: default_embedder()? 가 실패하면 인바운드 envelope 전부 DB 저장 전 드롭
    //   (macmini 214/214 drop, error="embedder init 실패"). embedder 실패가 전체
    //   process_inbound 를 early-return 시키던 근본 버그.
    // 현재: message_embedder() 는 init 실패 시 WARN 로그 + DummyEmbedder degrade →
    //   메시지 L0 저장은 항상 진행 (의미 임베딩만 best-effort). 정상 경로(FastEmbedder
    //   init 성공)는 기존과 동일하게 의미 임베딩 사용 — 회귀 없음.
    let embedder = message_embedder();
    tracing::debug!("process_inbound: embedder ready (best-effort, degrade-on-fail)");

    for env in envelopes {
        // rc.219 — ACK envelope branch. envelope_type="ack" 면 outbound_queue.ack_at UPDATE 만.
        // inbox 저장 / tmux inject / peer touch 모두 skip (ACK 자체는 사용자 메시지 X).
        if env.envelope_type.as_deref() == Some("ack") {
            let ulid = env.ack_for_ulid.clone().unwrap_or_default();
            let status = env.ack_status.clone().unwrap_or_else(|| "unknown".to_string());
            if ulid.is_empty() {
                tracing::warn!(from = %env.from, "rc.219 ACK envelope 도착했으나 ack_for_ulid 비어있음 (skip)");
                continue;
            }
            let now_str = chrono::Utc::now().to_rfc3339();
            let conn = db.conn();
            match conn.execute(
                "UPDATE outbound_queue SET ack_at = ?1, ack_status = ?2 WHERE msg_ulid = ?3",
                rusqlite::params![now_str, status, ulid],
            ) {
                Ok(n) if n > 0 => {
                    tracing::info!(
                        msg_ulid = %ulid,
                        ack_status = %status,
                        from = %env.from,
                        "rc.219 ACK 수신 → outbound_queue.ack_at UPDATE"
                    );
                }
                Ok(_) => {
                    tracing::warn!(
                        msg_ulid = %ulid,
                        ack_status = %status,
                        from = %env.from,
                        "rc.219 ACK 수신했으나 outbound_queue 매칭 row 없음 (이미 archived 또는 unknown ulid)"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, msg_ulid = %ulid, "rc.219 ACK UPDATE 실패");
                }
            }

            // rc.368 본질 fix — app_ack 텔레메트리 갭.
            // 이전: ACK envelope 는 outbound_queue.ack_at(transport) 만 UPDATE 하고
            //   `continue` 로 바로 빠져나가 app_ack conversation_id UPDATE(line ~1138)에
            //   절대 도달하지 못했다. 그 결과 수신측 LLM 이 같은 conversation_id 로 별도
            //   답신을 보내지 않는 한(거의 안 함) app_ack_status 가 영원히 NULL→5분 후 blocked.
            //   증거: zalman→seoul outbound 가 ack_status='inbox_stored'(=전송·저장 성공) 인데도
            //   app_ack_status='blocked' 로 남음. 반면 same-machine(portal)은 LLM 일반 답신이
            //   line 1138 을 타서 processed.
            // 현재: ACK envelope 가 conversation_id 를 운반하고(line ~1681에서 동봉) ack_status 가
            //   전달 성공(inbox_stored/tmux_injected/acp_delivered)이면, 이 ACK 자체가
            //   application-level 전달 확증이므로 같은 conversation_id 의 가장 최근 미답신
            //   outbound row 1건을 app_ack_status='processed' 로 flip. (line 1138 의 일반-답신
            //   경로와 동일한 매칭 로직 — 단지 ACK envelope 로도 트리거되게 한다.)
            // → LLM 답신 규율 의존 제거. 양방향 app_ack 대칭 추적. transport ack_at 과 독립.
            let delivered = matches!(
                status.as_str(),
                "inbox_stored" | "tmux_injected" | "acp_delivered"
            );
            if delivered {
                if let Some(conv_id) = env.conversation_id.as_deref() {
                    if !conv_id.is_empty() {
                        let conn = db.conn();
                        match conn.execute(
                            "UPDATE outbound_queue \
                             SET app_ack_at = ?1, app_ack_status = 'processed' \
                             WHERE rowid = ( \
                                 SELECT rowid FROM outbound_queue \
                                 WHERE conversation_id = ?2 \
                                   AND app_ack_at IS NULL \
                                 ORDER BY enqueued_at DESC LIMIT 1 \
                             )",
                            rusqlite::params![now_str, conv_id],
                        ) {
                            Ok(rows) if rows > 0 => tracing::info!(
                                conversation_id = %conv_id,
                                ack_status = %status,
                                from = %env.from,
                                "rc.368 app_ack: ACK envelope(전달성공) → outbound_queue.app_ack_status=processed flip"
                            ),
                            Ok(_) => tracing::debug!(
                                conversation_id = %conv_id,
                                "rc.368 app_ack: ACK 매칭 outbound row 없음 (자기가 안 보낸 conv 또는 이미 처리됨)"
                            ),
                            Err(e) => tracing::warn!(
                                error = %e,
                                conversation_id = %conv_id,
                                "rc.368 app_ack UPDATE 실패 (silent X)"
                            ),
                        }
                    }
                }
            }
            continue;
        }

        // 신원 전파 수신 — envelope_type="identity_update" 면 자기 로컬 표시 이름/역할 갱신.
        // body 는 JSON { "alias", "display_name", "role" } (변경된 필드만 Some). 제어 메시지이므로
        // inbox 저장 / tmux inject / peer touch 모두 skip (사용자 메시지 X) → ack 분기와 동일하게 continue.
        if env.envelope_type.as_deref() == Some("identity_update") {
            let payload_bytes = hex::decode(&env.payload_hex).unwrap_or_default();
            match serde_json::from_slice::<serde_json::Value>(&payload_bytes) {
                Ok(v) => {
                    let alias = v.get("alias").and_then(|x| x.as_str());
                    let dn = v.get("display_name").and_then(|x| x.as_str());
                    let role = v.get("role").and_then(|x| x.as_str());
                    if let Some(alias) = alias {
                        let conn = db.conn();
                        let mut affected = 0u64;
                        if let Some(dn) = dn {
                            match conn.execute(
                                "UPDATE agent_profiles SET display_name = ?1 WHERE alias = ?2",
                                rusqlite::params![dn, alias],
                            ) {
                                Ok(n) => affected += n as u64,
                                Err(e) => tracing::warn!(error = %e, alias, "identity_update agent_profiles.display_name UPDATE 실패"),
                            }
                            match conn.execute(
                                "UPDATE peers SET display_name = ?1 WHERE alias = ?2",
                                rusqlite::params![dn, alias],
                            ) {
                                Ok(n) => affected += n as u64,
                                Err(e) => tracing::warn!(error = %e, alias, "identity_update peers.display_name UPDATE 실패"),
                            }
                        }
                        if let Some(role) = role {
                            match conn.execute(
                                "UPDATE agent_capabilities SET role = ?1 WHERE alias = ?2",
                                rusqlite::params![role, alias],
                            ) {
                                Ok(n) => affected += n as u64,
                                Err(e) => tracing::warn!(error = %e, alias, "identity_update agent_capabilities.role UPDATE 실패"),
                            }
                            match conn.execute(
                                "UPDATE peers SET role = ?1 WHERE alias = ?2",
                                rusqlite::params![role, alias],
                            ) {
                                Ok(n) => affected += n as u64,
                                Err(e) => tracing::warn!(error = %e, alias, "identity_update peers.role UPDATE 실패"),
                            }
                        }
                        if affected > 0 {
                            tracing::info!(alias, from = %env.from, "identity_update 수신 → display_name/role 갱신 적용");
                        } else {
                            tracing::warn!(alias, from = %env.from, "identity_update: alias 매칭 row 없음 (로컬 변경 없음)");
                        }
                    } else {
                        tracing::warn!(from = %env.from, "identity_update 도착했으나 alias 비어있음 (skip)");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, from = %env.from, "identity_update payload JSON 파싱 실패 (skip)");
                }
            }
            continue;
        }

        // rc.173 — 메신저 본질: unknown peer 도 inbox 에 저장 (Telegram/카카오톡 식).
        // 단 신원 미검증 표시. 검증 정책은 sender_label prefix 로 구분 (peer: vs unverified:).
        let mut peer_opt = match PeerStore::new(&mut db).get_by_eth_address(&env.from) {
            Ok(opt) => opt,
            Err(e) => {
                tracing::warn!(error = %e, "peer 조회 실패");
                continue;
            }
        };

        // payload decode — 실패 시 placeholder body
        let payload_bytes = hex::decode(&env.payload_hex).unwrap_or_default();
        let sig_bytes = hex::decode(&env.signature_hex).unwrap_or_default();

        // rc.193 본질 fix — unknown peer + sender hint (alias, pubkey, transport_url) 있으면 자동 등록.
        // 서명 검증 OK 인 경우만 (sender 가 자기 pubkey 로 sign 했는지). 거짓 alias claim 방지.
        // rc.244 zero-touch — sender hint (alias, pubkey, transport_url) + 서명 검증 OK 면
        //   매번 idempotent 하게 자동 등록/갱신 (기존 peer 도 주소 refresh). gui_address 는
        //   transport_url 포트+2 로 파생해 함께 저장 → cross-machine 터미널 proxy 가 수동 교정
        //   없이 동작. 신규는 머신 데몬 = primary 기본. 주소 'unknown' 은 기존 주소를 덮지 않음.
        if let (Some(alias), Some(pubkey_hex)) =
            (env.sender_alias.as_deref(), env.sender_pubkey_hex.as_deref())
        {
            if verify_with_pubkey(pubkey_hex, &payload_bytes, &sig_bytes).is_ok() {
                let real_addr = env
                    .sender_transport_url
                    .as_deref()
                    .filter(|s| !s.is_empty() && !s.contains("unknown"));
                let mut peer_store = PeerStore::new(&mut db);
                if let Some(addr) = real_addr {
                    let gui = derive_gui_url(addr);
                    let new_role = if peer_opt.is_some() {
                        openxgram_peer::PeerRole::Worker // update 경로 — upsert 가 role 보존
                    } else {
                        openxgram_peer::PeerRole::Primary // 신규 머신 데몬 = primary 기본
                    };
                    match peer_store.upsert_announce(
                        alias, pubkey_hex, addr, gui.as_deref(), &env.from, new_role,
                    ) {
                        Ok(p) => {
                            tracing::debug!(alias = %p.alias, addr = %addr, gui = ?gui, "rc.244 zero-touch upsert (addr+gui 자동 갱신)");
                            peer_opt = Some(p);
                        }
                        Err(e) => {
                            tracing::debug!(alias = %alias, error = %e, "zero-touch upsert skip");
                            if peer_opt.is_none() {
                                if let Ok(Some(p)) = peer_store.get_by_eth_address(&env.from) {
                                    peer_opt = Some(p);
                                }
                            }
                        }
                    }
                } else if peer_opt.is_none() {
                    // 주소 불명 + 미등록 — 최소 placeholder (이후 정상 주소 announce 시 갱신)
                    let _ = peer_store
                        .add_with_eth(
                            alias,
                            pubkey_hex,
                            "http://unknown",
                            Some(&env.from),
                            openxgram_peer::PeerRole::Worker,
                            Some("auto-registered (addr unknown) via envelope"),
                        )
                        .map(|p| peer_opt = Some(p));
                }
            }
        }

        // 서명 검증 — peer 있고 서명 verify 성공 시만 verified.
        let verified = match &peer_opt {
            Some(p) => verify_with_pubkey(&p.public_key_hex, &payload_bytes, &sig_bytes).is_ok(),
            None => false,
        };
        if peer_opt.is_none() {
            tracing::info!(from = %env.from, "unknown peer — inbox 저장 진행 (unverified)");
        } else if !verified {
            tracing::warn!(from = %env.from, "서명 검증 실패 — inbox 저장 진행 (unverified)");
        }

        let alias = peer_opt.as_ref().map(|p| p.alias.clone()).unwrap_or_else(|| env.from.clone());

        // session 매핑 — alias 별 inbox session ensure (PRD-2.0.3)
        let session_title = format!("inbox-from-{}", alias);
        tracing::debug!(session_title=%session_title, "process_inbound: ensure_by_title 호출");
        let session = match SessionStore::new(&mut db).ensure_by_title(&session_title, "inbound") {
            Ok(s) => {
                tracing::debug!(session_id=%s.id, "process_inbound: session ensure ok");
                s
            }
            Err(e) => {
                tracing::error!(error = %e, title=%session_title, "process_inbound: session ensure 실패 (silent X)");
                continue;
            }
        };

        // L0 message 자동 저장 (PRD-2.0.2)
        // sender label: verified 면 peer:{alias}, 아니면 unverified:{alias} (LLM 이 신뢰도 판단 가능).
        let body = String::from_utf8_lossy(&payload_bytes).into_owned();
        let sender_label = if verified {
            format!("peer:{}", alias)
        } else {
            format!("unverified:{}", alias)
        };
        tracing::debug!(session_id=%session.id, sender=%sender_label, body_len=body.len(), "process_inbound: MessageStore insert 시도");
        if let Err(e) = MessageStore::new(&mut db, embedder.as_ref()).insert(
            &session.id,
            &sender_label,
            &body,
            &env.signature_hex,
            env.conversation_id.as_deref(),
        ) {
            tracing::error!(error = %e, session_id=%session.id, "process_inbound: L0 insert 실패 (silent X)");
            continue;
        }
        tracing::info!(session_id=%session.id, sender=%sender_label, body_len=body.len(), "process_inbound: 메시지 inbox 저장 완료");

        // rc.227 — application-level ACK hook.
        // 이 envelope 가 자기 가 보낸 peer_send 의 답신 (같은 conversation_id) 이면
        // outbound_queue.app_ack_at UPDATE (가장 최근 미답신 row 만).
        // single-sender 의 같은 conv 의 최근 outbound 만 update — multi 답신 case 회피.
        if let Some(conv_id) = env.conversation_id.as_deref() {
            if !conv_id.is_empty() {
                let now_str = chrono::Utc::now().to_rfc3339();
                let conn = db.conn();
                // 같은 conversation_id 의 가장 최근 미답신 outbound row 1건만 UPDATE.
                // SQLite 는 UPDATE...LIMIT 미지원 → rowid 서브쿼리.
                let updated = conn.execute(
                    "UPDATE outbound_queue \
                     SET app_ack_at = ?1, app_ack_status = 'processed' \
                     WHERE rowid = ( \
                         SELECT rowid FROM outbound_queue \
                         WHERE conversation_id = ?2 \
                           AND app_ack_at IS NULL \
                         ORDER BY enqueued_at DESC LIMIT 1 \
                     )",
                    rusqlite::params![now_str, conv_id],
                );
                match updated {
                    Ok(rows) if rows > 0 => tracing::info!(
                        conversation_id = %conv_id,
                        from = %env.from,
                        rows = rows,
                        "rc.227 app_ack: 같은 conv 의 답신 도착 → outbound_queue.app_ack_at UPDATE (processed)"
                    ),
                    Ok(_) => tracing::debug!(
                        conversation_id = %conv_id,
                        "rc.227 app_ack: 매칭 outbound row 없음 (자기가 안 보낸 conv 또는 이미 처리됨)"
                    ),
                    Err(e) => tracing::warn!(
                        error = %e,
                        conversation_id = %conv_id,
                        "rc.227 app_ack UPDATE 실패 (silent X)"
                    ),
                }
            }
        }

        // ── P4b — agent-add handshake INBOUND 분류 (rc.336) ─────────────────
        // 송신측(daemon_gui gui_agent_request_create / gui_agent_request_accept)이
        // `[AGENT_ADD_REQUEST] {json}` / `[AGENT_ADD_ACCEPT] {json}` prefix 의 서명 envelope 를
        // 기존 peer transport 로 보낸다. 이는 **제어 메시지**이지 대화가 아니다 →
        // 여기서 분류·영속 후 `continue` 로 tmux/ACP 전달 경로를 타지 않게 한다(이중 전달·세션 오염 방지).
        // L0 inbox 저장(위)은 감사/추적용으로 유지(메시지 본문 자체는 보존). 전달만 중단.
        if let Some(json_str) = body.strip_prefix("[AGENT_ADD_REQUEST] ") {
            // 요청자 머신 → 소유자 머신 도착: incoming 요청 row 생성(pending).
            // 송신 payload 필드: kind,id,requester,requester_machine,target_agent,note.
            //   (target_owner/target_machine 은 payload 에 없음 — 수신측 머신이 소유자이므로 NULL 로 둔다.)
            match serde_json::from_str::<serde_json::Value>(json_str.trim()) {
                Ok(v) => {
                    let req_id = v.get("id").and_then(|x| x.as_str()).unwrap_or_default();
                    let requester = v.get("requester").and_then(|x| x.as_str()).unwrap_or_default();
                    let target_agent = v.get("target_agent").and_then(|x| x.as_str()).unwrap_or_default();
                    if req_id.is_empty() || requester.is_empty() || target_agent.is_empty() {
                        tracing::warn!(
                            from = %env.from,
                            "rc.336 [AGENT_ADD_REQUEST] 필수 필드(id/requester/target_agent) 누락 — skip(절대 규칙 1: silent X)"
                        );
                    } else {
                        let requester_machine = v.get("requester_machine").and_then(|x| x.as_str());
                        let terms = v.get("note").and_then(|x| x.as_str()).filter(|s| !s.is_empty());
                        let now = crate::daemon_gui::kst_now_string();
                        // INSERT OR IGNORE on id(PK) — redelivery 멱등. direction=incoming, status=pending.
                        match db.conn().execute(
                            "INSERT OR IGNORE INTO agent_add_request \
                                (id, requester, requester_machine, target_agent, \
                                 status, currency, direction, terms, created_at_kst) \
                             VALUES (?1, ?2, ?3, ?4, 'pending', 'USDC', 'incoming', ?5, ?6)",
                            rusqlite::params![req_id, requester, requester_machine, target_agent, terms, now],
                        ) {
                            Ok(n) => tracing::info!(
                                id = %req_id,
                                requester = %requester,
                                target_agent = %target_agent,
                                inserted = n,
                                "rc.336 P4b INBOUND [AGENT_ADD_REQUEST] → agent_add_request(direction=incoming, pending) 영속. tmux/ACP 전달 안 함(제어 메시지)."
                            ),
                            Err(e) => tracing::error!(
                                id = %req_id, error = %e,
                                "rc.336 [AGENT_ADD_REQUEST] INSERT 실패(silent X)"
                            ),
                        }
                    }
                }
                Err(e) => tracing::warn!(
                    from = %env.from, error = %e,
                    "rc.336 [AGENT_ADD_REQUEST] JSON 파싱 실패 — 제어 메시지로 분류했으나 본문 손상. drop(절대 규칙 1: 명시 로그)."
                ),
            }
            // 제어 메시지 — 정상 대화 경로(recv_alias/tmux/ACP)로 fall-through 금지.
            continue;
        }
        if let Some(json_str) = body.strip_prefix("[AGENT_ADD_ACCEPT] ") {
            // 소유자 머신 → 요청자 머신 도착: 요청자의 OUTGOING row 를 accepted 로 갱신 + 가격/조건 반영.
            // 송신 payload 필드: kind,id,target_agent,by,price_amount,price_unit,currency,terms.
            match serde_json::from_str::<serde_json::Value>(json_str.trim()) {
                Ok(v) => {
                    let req_id = v.get("id").and_then(|x| x.as_str()).unwrap_or_default();
                    if req_id.is_empty() {
                        tracing::warn!(
                            from = %env.from,
                            "rc.336 [AGENT_ADD_ACCEPT] id 누락 — skip(절대 규칙 1: silent X)"
                        );
                    } else {
                        let price_amount = v.get("price_amount").and_then(|x| x.as_f64());
                        let price_unit = v.get("price_unit").and_then(|x| x.as_str()).filter(|s| !s.is_empty());
                        let currency = v.get("currency").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).unwrap_or("USDC");
                        let terms = v.get("terms").and_then(|x| x.as_str()).filter(|s| !s.is_empty());
                        let now = crate::daemon_gui::kst_now_string();
                        // 요청자측 outgoing pending row 만 accepted 로 갱신(멱등 — 이미 accepted 면 0 rows).
                        let updated = db.conn().execute(
                            "UPDATE agent_add_request \
                                SET status='accepted', price_amount=?2, price_unit=?3, \
                                    currency=?4, terms=?5, decided_at_kst=?6 \
                             WHERE id=?1 AND direction='outgoing' AND status='pending'",
                            rusqlite::params![req_id, price_amount, price_unit, currency, terms, now],
                        );
                        match updated {
                            Ok(n) if n > 0 => {
                                // accept handler(소유자측) 미러 — 요청자측에서도 그 에이전트를 friend grant
                                //   (classification=friend, isolated/cost_tracked 강제)로 사용 가능하게 한다.
                                let target_agent = v.get("target_agent").and_then(|x| x.as_str());
                                if let Some(ta) = target_agent.filter(|s| !s.is_empty()) {
                                    let _ = crate::daemon_gui::upsert_agent_profile(
                                        db.conn(),
                                        ta,            // alias = target_agent (요청자가 사용할 에이전트)
                                        "claude",
                                        "friend",
                                        "on_demand",
                                        None,          // machine (요청자측 — 실행 경로는 별도 해석)
                                        None,          // worktree
                                        false,         // is_public
                                        &now,
                                        Some("request"),
                                        Some(true),    // friend_isolated 강제
                                        Some(true),    // friend_cost_tracked 강제
                                    );
                                }
                                tracing::info!(
                                    id = %req_id, rows = n,
                                    "rc.336 P4b INBOUND [AGENT_ADD_ACCEPT] → outgoing row accepted + 가격/조건 반영 + friend grant. tmux/ACP 전달 안 함(제어 메시지)."
                                );
                            }
                            Ok(_) => tracing::info!(
                                id = %req_id,
                                "rc.336 [AGENT_ADD_ACCEPT] 매칭 outgoing/pending row 없음(이미 처리됨 또는 unknown id) — 멱등 skip"
                            ),
                            Err(e) => tracing::error!(
                                id = %req_id, error = %e,
                                "rc.336 [AGENT_ADD_ACCEPT] UPDATE 실패(silent X)"
                            ),
                        }
                    }
                }
                Err(e) => tracing::warn!(
                    from = %env.from, error = %e,
                    "rc.336 [AGENT_ADD_ACCEPT] JSON 파싱 실패 — 제어 메시지로 분류했으나 본문 손상. drop(절대 규칙 1: 명시 로그)."
                ),
            }
            // 제어 메시지 — 정상 대화 경로로 fall-through 금지.
            continue;
        }

        // rc.197 본질 push 알림 — DB INSERT 만 ≠ 통신.
        // rc.199 — envelope.recipient_alias hint 우선 (송신측 명시). 그래야 cross-machine 시
        // 받는 측 peers 에 receiver alias 등록 안 됐어도 tmux 매핑 가능.
        // rc.219 — recv_alias resolve 결과를 INFO log 로 명시. None 일 때도 silent X.
        let manifest_self_alias_for_log = openxgram_manifest::InstallManifest::read(
            openxgram_core::paths::manifest_path(data_dir),
        )
        .ok()
        .map(|m| m.machine.alias);
        let recv_alias = env
            .recipient_alias
            .clone()
            .or_else(|| {
                PeerStore::new(&mut db)
                    .get_by_public_key(&env.to)
                    .ok()
                    .flatten()
                    .map(|p| p.alias)
            })
            .or_else(|| manifest_self_alias_for_log.clone());
        let env_to_short: String = env.to.chars().take(16).collect();
        tracing::info!(
            recv_alias = ?recv_alias,
            recipient_alias_hint = ?env.recipient_alias,
            envelope_to_pubkey_prefix = %env_to_short,
            self_manifest_alias = ?manifest_self_alias_for_log,
            "rc.219 recv_alias resolution result"
        );

        // rc.219 — tmux inject 결과를 mutable variable 로 캡쳐 → ACK envelope 의 ack_status 결정.
        let mut tmux_injected = false;
        // fix④ Option A — ACP/A2A 가 정본 전달 척추. 아래에서 ACP 우선 시도 → 전달되면 tmux skip.
        let mut acp_delivered = false;

        if let Some(target_alias) = recv_alias {
            // rc.207 본질 fix — inject 형식에 conversation_id 의 앞 8자 포함.
            // LLM 가 자기 peer_send 의 conversation_id 와 시각적 link → polling 무의미.
            // conv 가 없으면 [INBOX from X] (legacy), 있으면 [INBOX from X conv:abcd1234].
            let conv_suffix = env
                .conversation_id
                .as_ref()
                .map(|c| {
                    let short: String = c.chars().take(8).collect();
                    format!(" conv:{}", short)
                })
                .unwrap_or_default();
            let injected = format!("[INBOX from {}{}] {}", sender_label, conv_suffix, body);
            let target_clone = target_alias.clone();
            let injected_clone = injected.clone();
            // fix④ — tmux 클로저가 target_alias 를 move 하므로 undelivered 기록용 사본 확보.
            let target_for_undelivered = target_alias.clone();

            // ── fix④ Option A — ACP/A2A 우선 전달 ──────────────────────────────
            // OLD(tmux send-keys)는 라이브 tmux pane 이 없는 ACP/GUI 전용 에이전트에게
            // 메시지를 유실시켰다. 이제 alias → ACP 세션/엔드포인트를 **먼저** 해석한다:
            //   - ACP 엔드포인트 존재(또는 a2a_send 의 lazy spawn 으로 생성 가능) →
            //     기존 spawn 머신리(a2a_send → handle_task)로 전달 + 통합 conv_key
            //     `a2a:{from}->{alias}` 영속.
            //   - ACP 불가(순수 터미널 peer) → 아래 tmux send-keys 로 fallback.
            // (sender_alias hint 없으면 sender_label 을 발신자 표기로 사용.)
            let acp_sender = env
                .sender_alias
                .clone()
                .unwrap_or_else(|| sender_label.clone());

            // fix① (A2A double-delivery) — 이 alias 가 ACP-drivable 신원인지 **동기** 판정.
            // ACP-drivable 이면 그 ACP 세션이 A2A inbound 전달을 **소유**한다 → tmux 로는 절대
            // fallback 하지 않는다. self-call 이 90s timeout 으로 Unavailable 을 돌려줘도 그건
            // "ACP 턴이 90s 보다 길다"는 뜻일 뿐, 백그라운드 ACP 세션은 계속 돌며 답한다.
            // 따라서 timeout==전달실패로 오판해 tmux 까지 주입하던 이중 전달을 차단한다.
            let acp_drivable_raw_base = is_acp_drivable(&mut db, &target_alias);

            // rc.370 #A 추가 방어 — 수신자가 **원격-홈 peer**(address host ≠ self_host)면 로컬 ACP
            //   spawn 자체를 안 한다. seoul 이 잘만 tmux 에이전트(codex-ai-image 등)에 로컬 ACP 세션을
            //   띄워 bridge_session_as_peer 가 peers 행을 acp:acp-1 로 날조하던 #A 근원 경로를 차단한다.
            //   원격-홈이면 acp_drivable 을 false 로 강등 → 아래 tmux/transport fallback 으로만 전달
            //   (홈 머신이 그 에이전트의 ACP 를 소유). self_host 미상/로컬-홈이면 종전 동작(회귀 방지).
            let target_remote_homed = {
                let self_host = crate::daemon_peer_sync::self_machine_host(data_dir);
                let peer_addr: Option<String> = db
                    .conn()
                    .query_row(
                        "SELECT address FROM peers WHERE alias = ?1",
                        [target_alias.as_str()],
                        |r| r.get::<_, String>(0),
                    )
                    .ok();
                match peer_addr.as_deref() {
                    Some(addr) => crate::daemon_peer_sync::is_remote_homed_peer(self_host.as_deref(), addr),
                    None => false,
                }
            };
            if target_remote_homed {
                tracing::info!(
                    target_alias = %target_alias,
                    "rc.370 #A — 원격-홈 peer 수신자: 로컬 ACP spawn 강등(홈 머신이 ACP 소유). tmux/transport 경로로만 전달."
                );
            }
            let acp_drivable_raw = acp_drivable_raw_base && !target_remote_homed;

            // rc.365 — 인바운드 peer 메시지를 **라이브 tmux 세션을 가진 수신 LLM**에게 자동 주입
            //   (Discord/Telegram auto-echo 의 peer 버전 — 실시간 A2A 대화의 마지막 조각).
            //
            // 근본 문제: register_subagent(MCP) 로 자기 등록한 대화형 tmux Claude 에이전트는
            //   agent_capabilities 에 role≠'tmux' 로 들어간다 → is_acp_drivable=true.
            //   그 결과 inbound 가 new_acp(헤드리스 ACP 스폰) 경로로 가서 사람-구동 Claude TUI 에는
            //   닿지 못하고, 90s timeout(Unavailable) 시 acp_delivered=true 로 오판되어 tmux 주입이
            //   **억제**된다 → 수신 LLM 화면에 아무것도 안 뜸 → app_ack 5분 BLOCKED.
            //
            // 핵심 원칙: **라이브 tmux 세션 = 사람-구동 수신자 = 정본 수신처.** 그 세션이 살아있으면
            //   speculative new_acp 보다 우선. 따라서 라이브 tmux 가 있으면 라우팅상 ACP-drivable 을
            //   false 로 강등 → 기존 tmux 주입 블록(bracketed-paste → sleep → Enter)이 실제로 발화한다.
            //   라이브 tmux 가 없으면 종전 ACP 경로 그대로(순수 ACP/GUI 전용 에이전트 회귀 방지).
            //
            // 게이팅: 기본 활성. XGRAM_DISABLE_PEER_TMUX_INJECT=1 이면 종전(ACP-우선) 동작으로 끔.
            let peer_tmux_inject_disabled = std::env::var("XGRAM_DISABLE_PEER_TMUX_INJECT")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            // 라이브 tmux 세션 존재 여부 — resolve_alias_to_tmux 재사용(3s timeout, hang 방지).
            let tmux_session_live = if peer_tmux_inject_disabled {
                false
            } else {
                let probe_alias = target_alias.clone();
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(3),
                            crate::notify::resolve_alias_to_tmux(&probe_alias),
                        )
                        .await
                        {
                            Ok(opt) => opt.is_some(),
                            Err(_elapsed) => {
                                tracing::warn!(
                                    target_alias = %probe_alias,
                                    "rc.365 tmux liveness probe TIMEOUT(3s) — tmux 없음으로 간주(종전 ACP 경로)"
                                );
                                false
                            }
                        }
                    })
                })
            };
            // rc.365 — 주입 대상 판정(제어 메시지·빈 본문·disabled·tmux 부재 모두 거른다).
            let inject_to_tmux =
                should_inject_inbound_to_tmux(&body, tmux_session_live, peer_tmux_inject_disabled);

            // ── rc.366 — 죽은 spawnable 수신자 자동 spawn("꺼져 있어도 메시지가 깨운다") ──
            // 라이브 tmux 세션이 없고(=rc.365 주입 대상 아님), 수신자가 spawnable(ai_type+
            // project_path 기동 정보 보유) 이면 자동으로 깨운다. 깨운 뒤에는 그 ACP 세션이
            // 메시지를 처리해야 하므로, spawn 성공 시 acp_drivable_raw 가 true 이면 종전 ACP
            // 전달 경로(try_acp_inbound_delivery → a2a_send new_acp find-or-create)가 같은
            // label 세션을 재사용해 메시지를 전달한다(새 spawner 만들지 않음).
            //
            // 가드(spawn-storm 방지):
            //   - disabled / 빈 본문 / 제어 메시지 / 라이브 tmux 있음 → spawn 안 함(should_spawn_*).
            //   - spawnable(기동 정보) 아님 → spawn 안 함(미등록/system/cwd 없음 → 인박스 보존).
            //   - rate-limit: 동일 수신자 5분 내 재spawn 금지(spawn_recently_attempted, 시도 직전 마킹).
            //   - spawn 실패: WARN 로그 + 메시지 인박스 보존(이미 위에서 L0 저장됨, 유실 없음).
            if !inject_to_tmux {
                let spawnable = is_recipient_spawnable(&mut db, &target_alias);
                // rate_limited 판정은 실제 spawn 직전에만(아래) — 여기서는 storm 가드 전 단계 결정만.
                if should_spawn_inbound_recipient(
                    &body,
                    tmux_session_live,
                    spawnable,
                    peer_tmux_inject_disabled,
                    false, // rate_limit 은 spawn 실행 직전 check-and-set 으로 별도 적용.
                ) {
                    if spawn_recently_attempted(&target_alias) {
                        tracing::info!(
                            target_alias = %target_alias,
                            window_secs = spawn_ratelimit_secs(),
                            "rc.366 죽은 spawnable 수신자지만 rate-limit 윈도우 내 — 자동 spawn skip(storm 방지). 메시지는 ACP/인박스 경로로."
                        );
                    } else {
                        tracing::info!(
                            target_alias = %target_alias,
                            "rc.366 죽은 spawnable 수신자 발견 — 자동 spawn 시도(메시지가 깨운다)."
                        );
                        let spawn_alias = target_alias.clone();
                        let spawned = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current()
                                .block_on(try_spawn_recipient(&spawn_alias))
                        });
                        if spawned {
                            // 세션 안정화 대기 — 어댑터 프로세스가 ACP handshake 를 끝내고 등록될 시간.
                            // 이후 종전 ACP 전달 경로(acp_drivable_raw=true 인 경우)가 같은 label
                            // 세션을 find-or-create 로 재사용해 메시지를 전달한다.
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(async {
                                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                })
                            });
                            tracing::info!(
                                target_alias = %target_alias,
                                "rc.366 자동 spawn 완료 — 메시지를 ACP 전달 경로로 넘김(label 세션 재사용)."
                            );
                        }
                    }
                }
            }
            // 라이브 tmux 주입 대상이면 라우팅상 ACP-drivable 강등(tmux 우선).
            let acp_drivable = if inject_to_tmux {
                tracing::info!(
                    target_alias = %target_alias,
                    "rc.365 라이브 tmux 세션 발견 → inbound peer 메시지 tmux 자동 주입 우선(ACP suppression 해제)"
                );
                false
            } else {
                acp_drivable_raw
            };

            // ── P4a — 턴 모드 게이트 (맥락 누적 ≠ 강제 응답 분리) ──────────────────
            // 스펙 항목 3: 들어오면 무조건 턴=응답이 문제. 방 turn_mode=gated 면 inbound 는
            // 수신자 스레드에 **누적만** 하고 ACP 턴을 자동 발화하지 않는다(관찰자). 그 에이전트는
            // 나중에 "발언권 주기"(grant-turn) / @호명 / 조건으로만 누적 맥락 위에 한 번 발언한다.
            // 기본 turn_mode=auto(row 없음/미설정 포함) = 종전 동작 → 1:1 무회귀.
            // gated 는 ACP-drivable 신원에만 의미(tmux peer 는 게이트 대상 아님 — 종전 fallback).
            let turn_mode = crate::daemon_gui_a2a::server::room_turn_mode(&mut db, &target_alias);
            if acp_drivable && turn_mode == "gated" {
                // 누적 — 발신자 prefix 보존(grant-turn 시 맥락으로 읽힘). 동일 in-process DB 연결로
                // acp_messages 에 직접 기록(self-call HTTP 불필요). 빈 본문은 skip.
                let accrue_text = format!("[from {acp_sender}] {body}");
                if !body.trim().is_empty() {
                    let now = chrono::Utc::now().to_rfc3339();
                    match db.conn().execute(
                        "INSERT INTO acp_messages (conv_key, role, text, created_at) VALUES (?1,?2,?3,?4)",
                        rusqlite::params![target_alias, "me", accrue_text, now],
                    ) {
                        Ok(_) => tracing::info!(
                            target_alias = %target_alias,
                            sender = %acp_sender,
                            "P4a turn_mode=gated — inbound 맥락 누적만(턴 미발화). grant-turn/@호명/조건 대기. tmux 차단(관찰자)."
                        ),
                        Err(e) => tracing::error!(
                            target_alias = %target_alias,
                            error = %e,
                            "P4a gated 누적 기록 실패(silent X) — 회귀 방지 위해 ACP 자동전달로 폴백하지 않고 누적 의도만 로그"
                        ),
                    }
                }
                // 게이트됨 = ACP 세션이 전달을 소유(관찰자 모드). tmux fallback 차단.
                acp_delivered = true;
            }

            let acp_outcome = if acp_delivered {
                // gated 로 이미 처리됨 — 자동 턴 발화(try_acp_inbound_delivery) skip. 종전 auto 경로 불변.
                AcpInboundOutcome::Delivered
            } else if inject_to_tmux {
                // rc.365 — 라이브 tmux 세션 수신자: speculative new_acp self-call 을 **건너뛴다**.
                //   (헤드리스 ACP 중복 스폰 + 최대 90s 낭비 방지.) 곧장 tmux 주입 경로로 간다.
                //   NoEndpoint 로 처리 → 아래 tmux 주입 블록이 발화(acp_delivered 유지 false).
                tracing::info!(
                    target_alias = %target_alias,
                    "rc.365 라이브 tmux 수신자 — ACP self-call(new_acp) skip, tmux 직접 주입"
                );
                AcpInboundOutcome::NoEndpoint
            } else {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(try_acp_inbound_delivery(
                        &target_alias,
                        &acp_sender,
                        &body,
                    ))
                })
            };
            match acp_outcome {
                AcpInboundOutcome::Delivered => acp_delivered = true,
                // fix① — ACP-drivable 인데 timeout/일시오류(Unavailable) 면 ACP 세션이 전달을
                // 소유하므로 delivered 로 간주(option (b)). tmux suppress. ACP 턴/회신 로직은
                // 손대지 않는다(전달-결정만 변경) — handle_task 가 백그라운드에서 회신 영속.
                AcpInboundOutcome::Unavailable if acp_drivable => {
                    tracing::info!(
                        target_alias = %target_alias,
                        sender = %acp_sender,
                        "fix① ACP self-call Unavailable(=90s timeout/일시오류) 이나 alias 가 ACP-drivable — ACP 세션이 전달 소유, tmux fallback 차단(이중 전달 방지)"
                    );
                    acp_delivered = true;
                }
                // NoEndpoint(순수 터미널 peer) 또는 비-ACP-drivable Unavailable → tmux fallback.
                AcpInboundOutcome::NoEndpoint | AcpInboundOutcome::Unavailable => {}
            }
            // fix④ — ACP/A2A 로 이미 전달됐으면 tmux 는 skip. tmux 는 ACP 엔드포인트가 없는
            // 순수 터미널 peer 전용 fallback 으로 강등(절대 제거 X — 라이브 fleet 회귀 방지).
            // process_inbound 는 sync — block_in_place + block_on 으로 async tmux send-keys 호출
            // rc.219 — return bool 로 tmux inject 성공/실패 명시 (silent debug 제거).
            // rc.238 — 전체 tmux inject 를 5초 timeout 으로 감쌈. tmux send-keys / list-sessions /
            // resolve_alias_to_tmux 중 하나라도 hang 하면 inbound_processor tick 전체가 멈추던
            // 근본 버그(23:47 stuck) 해결. timeout 시 이 envelope inject 만 포기 + WARN, 다음 진행.
            // fix① — ACP-drivable alias 는 어떤 경우에도 tmux 로 보내지 않는다(CONFIRMED RULE:
            // A2A 는 수신자 ACP 로만, tmux 는 human-only). `!acp_delivered` 위에 `!acp_drivable`
            // 가드를 추가해 NoEndpoint 같은 예외 경로로도 tmux 가 새지 않게 한다.
            tmux_injected = !acp_delivered && !acp_drivable && tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    let inject_fut = async move {
                        if let Some((session, idx)) =
                            crate::notify::resolve_alias_to_tmux(&target_clone).await
                        {
                            let target = format!("{}:{}", session, idx);
                            let wrapped = format!("\x1b[200~{}\x1b[201~", injected_clone);
                            // rc.198 — Windows daemon → wsl tmux 자동 wrap
                            let _ = crate::notify::tmux_command_async()
                                .args(["send-keys", "-t", &target, "-l", &wrapped])
                                .output()
                                .await;
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            let _ = crate::notify::tmux_command_async()
                                .args(["send-keys", "-t", &target, "Enter"])
                                .output()
                                .await;
                            tracing::info!(
                                alias = %target_clone,
                                tmux_session = %session,
                                "rc.197 inbound push → tmux LLM 화면에 inject"
                            );
                            true
                        } else {
                            // rc.219 — silent debug 승격 → WARN. tmux session list 도 함께 log.
                            let sessions_listed = crate::notify::tmux_command_async()
                                .args(["list-sessions", "-F", "#{session_name}"])
                                .output()
                                .await
                                .ok()
                                .and_then(|o| {
                                    if o.status.success() {
                                        Some(String::from_utf8_lossy(&o.stdout).to_string())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| "(tmux list-sessions 실패 또는 tmux 미설치)".to_string());
                            tracing::warn!(
                                target_alias = %target_clone,
                                tmux_sessions = %sessions_listed.replace('\n', ","),
                                "rc.219 tmux 매칭 안 됨 — alias → tmux session resolve 실패"
                            );
                            false
                        }
                    };
                    // rc.238 — 5초 timeout. tmux 명령 hang 시 tick 멈춤 근본 fix.
                    match tokio::time::timeout(std::time::Duration::from_secs(5), inject_fut).await {
                        Ok(injected) => injected,
                        Err(_elapsed) => {
                            tracing::warn!(
                                target_alias = %target_alias,
                                "rc.238 tmux inject TIMEOUT (5s) — 이 envelope inject 포기, 다음 진행 (tick stuck 회피)"
                            );
                            false
                        }
                    }
                })
            });

            // fix④ — ACP 도 tmux 도 전달 실패 → 조용히 버리지 않는다(절대 규칙 1).
            // 통합 스레드 a2a:{alias} 에 undelivered 마커 영속 + 명시 로그.
            if !acp_delivered && !tmux_injected {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(record_inbound_undelivered(
                        &target_for_undelivered,
                        &acp_sender,
                        &body,
                    ))
                });
            }
        } else {
            tracing::warn!(
                envelope_to_pubkey_prefix = %env_to_short,
                "rc.219 recv_alias 미정 — recipient_alias hint X + peer table lookup 실패 + manifest 의 self alias 도 없음. tmux inject skip"
            );
        }

        // rc.219 — ACK envelope 송신. sender 측 outbound_queue.ack_at UPDATE 가능하도록.
        // ack_status: inbox_stored 는 항상 (위에서 insert 성공 후 도달).
        // fix④ — acp_delivered 면 acp_delivered, 아니면 tmux_injected 면 tmux_injected 로 격상.
        // nonce 는 envelope 의 것 (== outbound_queue.msg_ulid 와 다른 sender 측 generator. envelope.nonce 가 msg_ulid 매칭 키).
        // sender 측 outbound_queue.msg_ulid 는 sender 가 record 한 ulid. envelope.nonce 와 별개.
        // → 따라서 sender 가 outbound_queue INSERT 시 사용한 ulid 를 envelope 의 어떤 필드로 운반해야 매칭 가능.
        // 본 envelope 의 nonce 를 ulid 로 활용 (sender 가 record_outbox 시 동일 값 사용).
        let ack_for_ulid = env.nonce.clone();
        let ack_status_val = if acp_delivered {
            "acp_delivered"
        } else if tmux_injected {
            "tmux_injected"
        } else {
            "inbox_stored"
        };
        if let Some(ack_ulid) = ack_for_ulid.as_ref() {
            // sender hint — env.sender_transport_url 우선, 없으면 peer table 의 address.
            let sender_url = env.sender_transport_url.clone().or_else(|| {
                PeerStore::new(&mut db)
                    .get_by_eth_address(&env.from)
                    .ok()
                    .flatten()
                    .map(|p| p.address)
            });
            let to_pubkey_for_ack = env.sender_pubkey_hex.clone().unwrap_or_default();
            // rc.219 — ACK envelope 의 from 은 receiver 측 eth_address.
            // 자체적 derivation 비용 회피 위해 env.to (=자기 pubkey hex) 를 from 으로 사용.
            // sender 측 ACK 처리는 ack_for_ulid 매칭만 사용 — from/to 검증 X.
            let self_addr_for_ack = env.to.clone();
            if let Some(url) = sender_url {
                let ack_envelope = openxgram_transport::Envelope {
                    from: self_addr_for_ack,
                    to: to_pubkey_for_ack,
                    payload_hex: String::new(),
                    timestamp: openxgram_core::time::kst_now(),
                    signature_hex: String::new(),
                    nonce: Some(uuid::Uuid::new_v4().to_string()),
                    conversation_id: env.conversation_id.clone(),
                    sender_alias: manifest_self_alias_for_log.clone(),
                    sender_transport_url: std::env::var("XGRAM_TRANSPORT_PUBLIC_URL").ok(),
                    sender_pubkey_hex: None,
                    recipient_alias: env.sender_alias.clone(),
                    envelope_type: Some("ack".to_string()),
                    ack_for_ulid: Some(ack_ulid.clone()),
                    ack_status: Some(ack_status_val.to_string()),
                };
                let ulid_for_log = ack_ulid.clone();
                let url_clone = url.clone();
                // rc.238 — ACK 송신도 5초 timeout. send_envelope (reqwest) 가 hang 하면
                // inbound_processor tick 전체가 멈추던 hang point. timeout 시 ACK 포기 + WARN, 다음 진행.
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
                        let send_fut = async {
                            match openxgram_transport::send_envelope(&url_clone, &ack_envelope).await {
                                Ok(()) => {
                                    tracing::info!(
                                        ack_for_ulid = %ulid_for_log,
                                        ack_status = %ack_status_val,
                                        target_url = %url_clone,
                                        "rc.219 ACK envelope 송신 OK"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        ack_for_ulid = %ulid_for_log,
                                        target_url = %url_clone,
                                        "rc.219 ACK envelope 송신 실패 (sender 측 ack_at UPDATE 못 함)"
                                    );
                                }
                            }
                        };
                        if tokio::time::timeout(std::time::Duration::from_secs(5), send_fut)
                            .await
                            .is_err()
                        {
                            tracing::warn!(
                                ack_for_ulid = %ack_ulid,
                                "rc.238 ACK 송신 TIMEOUT (5s) — ACK 포기, 다음 envelope 진행 (tick stuck 회피)"
                            );
                        }
                    })
                });
            } else {
                tracing::warn!(
                    ack_for_ulid = %ack_ulid,
                    "rc.219 ACK 송신 skip — sender_transport_url + peer.address 둘 다 없음"
                );
            }
        } else {
            tracing::info!(
                "rc.219 ACK 송신 skip — envelope.nonce (=ack 매칭 키) 비어있음 (legacy sender)"
            );
        }
        // dummy peer var for legacy code paths below (payment receipt, touch).
        let peer = match peer_opt {
            Some(p) => p,
            None => {
                tracing::debug!(from = %env.from, session = %session.id, "unverified inbound 저장 완료 (peer 미등록, touch skip)");
                continue;
            }
        };

        // 4b. payment receipt 자동 인식 — 첫 줄이 magic prefix 면 L2 reference memory 로 추가 기록.
        //     수취인 측이 "최근 받은 결제" 를 메모리 검색으로 즉시 회상 가능 (PRD §16 양쪽 메모리 기록).
        if let Some(rest) = body.strip_prefix("xgr-payment-receipt-v1\n") {
            match record_payment_receipt(&mut db, &peer.alias, &env.from, rest) {
                Ok(memo_id) => tracing::info!(
                    from = %env.from,
                    memory_id = %memo_id,
                    "payment receipt → L2 memory(reference) 기록"
                ),
                Err(e) => tracing::warn!(error = %e, "payment receipt 메모리 기록 실패"),
            }
        }

        // 5. peer last_seen 갱신
        if let Err(e) = PeerStore::new(&mut db).touch_by_eth_address(&env.from) {
            tracing::warn!(error = %e, "peer.touch 실패");
        }
        tracing::info!(from = %env.from, session = %session.id, "inbound envelope 저장 완료");
    }
    Ok(())
}

/// magic-prefix 가 떨어진 JSON body 를 파싱해서 사람이 읽기 쉬운 reference memory 로 기록.
/// 반환: 새 memory id.
fn record_payment_receipt(
    db: &mut openxgram_db::Db,
    sender_alias: &str,
    sender_addr: &str,
    json_body: &str,
) -> Result<String> {
    use openxgram_memory::{MemoryKind, MemoryStore};
    let v: serde_json::Value =
        serde_json::from_str(json_body.trim()).with_context(|| "payment receipt JSON 파싱 실패")?;
    let amount = v
        .get("amount_display")
        .and_then(|x| x.as_str())
        .unwrap_or("?? USDC");
    let chain = v.get("chain").and_then(|x| x.as_str()).unwrap_or("?");
    let tx_hash = v.get("tx_hash").and_then(|x| x.as_str()).unwrap_or("?");
    let memo = v.get("memo").and_then(|x| x.as_str()).unwrap_or("");
    let intent_id = v.get("intent_id").and_then(|x| x.as_str()).unwrap_or("?");
    let content = format!(
        "Received {amount} from {sender_alias} ({sender_addr}) on {chain}.\n\
         tx_hash: {tx_hash}\n\
         intent_id: {intent_id}\n\
         memo: {memo}"
    );
    let m = MemoryStore::new(db).insert(None, MemoryKind::Reference, &content)?;
    Ok(m.id)
}

/// rc.232 — sync LLM-검증: tmux session 의 pane 프로세스 트리에 실제 LLM 이 도는지 검사.
/// 마스터의 본질: peer = portal 작동중 LLM 터미널 (claude/gemini/codex/ollama/aider).
/// LLM 안 도는 tmux (`-bash`, 빈 pane, `xgramd`, `starian` 같은 운영 shell) 은 peer 아님.
/// daemon startup 의 sync context 에서 호출되므로 std::process::Command 사용
/// (daemon_gui.rs 의 detect_llm_in_subtree async 버전과 동일 키워드, sync 재구현).
/// 검출 실패 시 false 반환 (그 session 은 peer 등록 skip — silent X, 호출부에서 log).
fn tmux_session_runs_llm(session: &str) -> bool {
    // pane PID 조회 (자기 머신 tmux. Windows = wsl tmux).
    let (cmd, base_arg) = if cfg!(windows) {
        ("wsl", Some("tmux"))
    } else {
        ("tmux", None)
    };
    let pane_pid: u32 = {
        let mut c = std::process::Command::new(cmd);
        if let Some(a) = base_arg {
            c.arg(a);
        }
        match c
            .args([
                "display-message",
                "-p",
                "-t",
                &format!("{session}:0"),
                "#{pane_pid}",
            ])
            .output()
        {
            Ok(out) if out.status.success() => {
                match String::from_utf8_lossy(&out.stdout).trim().parse::<u32>() {
                    Ok(p) => p,
                    Err(_) => return false,
                }
            }
            _ => return false,
        }
    };

    // pane_pid 자체 + 자식 process tree BFS (최대 깊이 4) — LLM 키워드 매칭.
    let is_llm_line = |line: &str| -> bool {
        let hay = line.to_lowercase();
        // daemon_gui.rs match_llm_in_line 과 동일 후보 + api 클라이언트 false-positive 제외.
        ((hay.contains("claude")) && !hay.contains("claude-api"))
            || (hay.contains("gemini") && !hay.contains("gemini-api"))
            || hay.contains("codex")
            || hay.contains("ollama")
            || hay.contains("aider")
            || hay.contains("cursor-agent")
            || hay.contains("cursor agent")
            || (hay.contains("continue") && hay.contains("dev"))
            || hay.contains("cline")
            // rc.278 — Hermes Agent (비-Claude 프레임워크) 도 LLM 에이전트로 인식.
            // 미인식 시 auto-seed 가 skip → peer 등재 안 됨 → 양방향 통신 불가였음.
            || hay.contains("hermes")
    };

    // 0회차 — pane_pid 자체.
    if let Ok(out) = std::process::Command::new("ps")
        .args(["-o", "pid=,comm=,args=", "-p", &pane_pid.to_string()])
        .output()
    {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if is_llm_line(line) {
                    return true;
                }
            }
        }
    }

    let mut frontier: Vec<u32> = vec![pane_pid];
    let mut visited: std::collections::HashSet<u32> = std::collections::HashSet::new();
    visited.insert(pane_pid);
    for _depth in 0..4 {
        if frontier.is_empty() {
            break;
        }
        let pids_csv = frontier
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let out = match std::process::Command::new("ps")
            .args(["-o", "pid=,comm=,args=", "--ppid", &pids_csv])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => break,
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut next_frontier: Vec<u32> = vec![];
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(pid_s) = trimmed.split_whitespace().next() {
                if let Ok(child_pid) = pid_s.parse::<u32>() {
                    if visited.insert(child_pid) {
                        next_frontier.push(child_pid);
                    }
                }
            }
            if is_llm_line(trimmed) {
                return true;
            }
        }
        frontier = next_frontier;
    }
    false
}

/// rc.273 — 단일 진실원천: 이 머신에서 **현재 살아있는 tmux 에이전트** 의 세션 식별자 집합.
/// 마스터 룰: 메신저 로스터에는 살아있는 tmux LLM 세션(①tmux LLM ②worktree ③서브에이전트 sv_)
/// 만 보여야 한다. gui_peers(로스터)·reachable_remote_peers(광고) 양쪽이 이 한 함수로 LOCAL
/// peer 의 생존을 판정해 회귀를 막는다.
///
/// 판정:
///   - `tmux list-sessions` 로 자기 머신 세션 열거 (auto_seed 와 동일 스캔 방식 재사용).
///   - `sv_*` (서브에이전트 worktree 세션) → 부모 pane 공유, 무조건 live 포함.
///   - 그 외 세션 → `tmux_session_runs_llm` 게이트 통과해야 live (운영 shell/placeholder 제외).
///
/// 반환 형식은 peers.session_identifier 와 동일한 `tmux:<session_name>` — 호출부가 LOCAL peer 의
/// session_identifier 와 직접 set-membership 비교한다. tmux 미설치/실패 시 빈 집합 반환
/// (호출부에서 "LOCAL peer 판정 자체를 못 함" → 보수적으로 필터하지 않도록 호출부가 처리).
pub(crate) fn local_live_tmux_agent_idents() -> std::collections::HashSet<String> {
    let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
    // auto_seed_local_tmux_agents 와 동일한 tmux list-sessions 스캔 (Windows = wsl tmux).
    let local_sessions: Vec<String> = {
        let (cmd, base_arg) = if cfg!(windows) {
            ("wsl", Some("tmux"))
        } else {
            ("tmux", None)
        };
        let mut c = std::process::Command::new(cmd);
        if let Some(a) = base_arg {
            c.arg(a);
        }
        match c.args(["list-sessions", "-F", "#{session_name}"]).output() {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => return live, // tmux 없음/실패 → 빈 집합 (호출부가 보수적으로 처리).
        }
    };
    for sn in &local_sessions {
        // sv_* 서브에이전트 worktree 세션 — 부모 LLM pane 공유. live 로 인정.
        if sn.starts_with("sv_") {
            live.insert(format!("tmux:{sn}"));
            continue;
        }
        // 그 외 — pane 에 실제 LLM 이 도는 세션만 live (운영 shell/placeholder 제외).
        if tmux_session_runs_llm(sn) {
            live.insert(format!("tmux:{sn}"));
        }
    }
    live
}

/// rc.201 — auto-seed: 자기 머신 의 tmux session 을 agent_capabilities 자동 등록.
/// 마스터의 본질: peer = 터미널 (각 tmux). daemon startup 시 자기 머신 tmux session list
/// 가져와서 각 session_name 의 alias 추출 → agent_capabilities INSERT OR IGNORE.
/// 그 다음 retroactive_register_agents 가 sub-keystore + peer 자동 등록.
/// rc.232 — LLM 검증 게이트: LLM 안 도는 tmux (shell/placeholder) 는 seed skip.
fn auto_seed_local_tmux_agents(data_dir: &std::path::Path) -> anyhow::Result<usize> {
    // 자기 머신 tmux list-sessions
    let local_sessions: Vec<String> = {
        let (cmd, base_arg) = if cfg!(windows) {
            ("wsl", Some("tmux"))
        } else {
            ("tmux", None)
        };
        let mut c = std::process::Command::new(cmd);
        if let Some(a) = base_arg {
            c.arg(a);
        }
        match c.args(["list-sessions", "-F", "#{session_name}"]).output() {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => return Ok(0),
        }
    };
    if local_sessions.is_empty() {
        return Ok(0);
    }

    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;
    // 아키텍처 수정 — auto-seed 는 더 이상 명부(agent_capabilities) 행을 만들지 않는다.
    // seeded 는 0 으로 고정(로스터 등록 없음). peers.session_identifier 갱신만 수행.
    let seeded = 0;
    for sn in &local_sessions {
        // alias 추출: 'aoe_<alias>_<id>' → alias / 그 외 → session_name 그대로
        let alias: String = if let Some(s) = sn.strip_prefix("aoe_") {
            match s.rsplit_once('_') {
                Some((a, _id)) => a.to_string(),
                None => s.to_string(),
            }
        } else {
            sn.clone()
        };
        if alias.is_empty() || alias == "null" || alias.contains('[') {
            continue;
        }
        // rc.232 — subagent worktree session (sv_aoe_*, sv_*) 와 순수 숫자 alias 는 peer 아님.
        // 이들은 부모 LLM 의 pane 을 공유하므로 LLM 검증을 통과해도 독립 peer 가 아님 → skip.
        if sn.starts_with("sv_") || alias.chars().all(|c| c.is_ascii_digit()) {
            tracing::debug!(alias = %alias, tmux = %sn, "rc.232 auto-seed skip — subagent/numeric (peer 아님)");
            continue;
        }
        // rc.232 — LLM 검증 게이트: pane 에 실제 LLM 안 도는 tmux session 은 peer 아님 → seed skip.
        // term_*, starian, xgramd 같은 운영 shell 부활 방지. silent X — debug log.
        if !tmux_session_runs_llm(sn) {
            tracing::debug!(alias = %alias, tmux = %sn, "rc.232 auto-seed skip — pane 에 LLM 미검출 (shell/placeholder)");
            continue;
        }
        // 아키텍처 수정 — tmux 세션을 명부(agent_capabilities) 에이전트로 자동 등록하지 않는다.
        // 마스터 결정: 에이전트는 마스터가 "에이전트 추가"로 의도적으로 생성한다(agent_profiles).
        // tmux 는 에이전트가 도는 장소일 뿐 — DETAIL 패널(/v1/gui/sessions)에서 alias/project_path
        // 로 매핑되어 보이며, 명부 로스터에는 나타나지 않는다.
        // 따라서 여기서 agent_capabilities INSERT 를 더 이상 하지 않는다(로스터 오염 방지).
        // 단, 아래 peers.session_identifier UPDATE 는 통신/세션-매핑 경로용이므로 유지한다
        // (이미 존재하는 peer 가 있을 때만 no-op 갱신 — 새 명부 행을 만들지 않음).
        // rc.245 — 결정적 세션 매핑: peer row 에 명시적 session_identifier 저장.
        // format 은 collect_sessions(/v1/gui/sessions) 의 local tmux entry 와 동일 ("tmux:<name>").
        // capture_session 이 이 식별자를 바로 resolve → Messenger.tsx normalizeAlias 추정 불필요.
        // peer 가 아직 없으면 no-op (retroactive_register_agents 가 peer 생성 후 다음 startup 에 set).
        // 사용자가 UI 에서 override 한 경우 (session_identifier IS NOT NULL) 는 덮어쓰지 않음.
        let sid = format!("tmux:{sn}");
        let _ = db.conn().execute(
            "UPDATE peers SET session_identifier = ?1 \
             WHERE alias = ?2 AND (session_identifier IS NULL OR session_identifier = '')",
            rusqlite::params![&sid, &alias],
        );
    }
    tracing::info!(seeded = seeded, total_sessions = local_sessions.len(), "rc.201 auto-seed 완료");
    Ok(seeded)
}

/// 기본 동봉(built-in) 특수에이전트를 seed. xgram-ops 등 OpenXgram 운영 에이전트를
/// agent_capabilities + agent_profiles 양 테이블에 INSERT OR IGNORE (idempotent —
/// 재시작·활성화 상태 보존). source='built_in', activated=0 으로 설치 → GUI 활성화 버튼이 1로.
/// 패키지 정본은 repo `agents/<name>/agent.json` + `instructions.md` — 컴파일 시 동봉.
fn seed_builtin_agents(data_dir: &std::path::Path) -> anyhow::Result<usize> {
    // (agent.json, instructions.md) 쌍. 신규 built-in 추가 시 여기에 한 줄.
    const BUILTINS: &[(&str, &str)] = &[(
        include_str!("../../../agents/xgram-ops/agent.json"),
        include_str!("../../../agents/xgram-ops/instructions.md"),
    )];
    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut seeded = 0usize;
    for (meta_json, instructions) in BUILTINS {
        let meta: serde_json::Value = match serde_json::from_str(meta_json) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "built-in agent.json 파싱 실패 — skip");
                continue;
            }
        };
        let alias = meta.get("alias").and_then(|v| v.as_str()).unwrap_or("");
        if alias.is_empty() {
            continue;
        }
        let role = meta.get("role").and_then(|v| v.as_str()).unwrap_or("agent");
        let description = meta.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let ai_type = meta.get("ai_type").and_then(|v| v.as_str()).unwrap_or("claude");
        let classification = meta.get("classification").and_then(|v| v.as_str()).unwrap_or("special");
        let execution_mode = meta.get("execution_mode").and_then(|v| v.as_str()).unwrap_or("on_demand");
        let display_name = meta.get("display_name").and_then(|v| v.as_str());
        let capabilities = meta.get("capabilities").map(|c| c.to_string()).unwrap_or_else(|| "[]".to_string());
        // 지침 주입 — ACP session/new 은 instructions 필드가 없으므로, cwd 의 CLAUDE.md 를
        // Claude Code 가 자동 로드하는 네이티브 경로를 쓴다. <data_dir>/agents/<alias>/CLAUDE.md
        // 를 materialize 하고 project_path 를 그 디렉토리로 설정 → 활성화 후 ACP spawn 시
        // 그 cwd 에서 지침이 자동 적용된다. 매 startup 덮어써서 동봉 지침과 동기화.
        let agent_dir = data_dir.join("agents").join(alias);
        if let Err(e) = std::fs::create_dir_all(&agent_dir) {
            tracing::warn!(error = %e, alias = %alias, "built-in agent 디렉토리 생성 실패");
        }
        let claude_md = agent_dir.join("CLAUDE.md");
        if let Err(e) = std::fs::write(&claude_md, instructions) {
            tracing::warn!(error = %e, alias = %alias, "built-in agent CLAUDE.md 작성 실패");
        }
        let project_path = agent_dir.to_string_lossy().to_string();
        // agent_capabilities — messenger_enabled=0 (활성화 전엔 peer 통신 비활성), special_instructions=지침 본문.
        let c1 = db.conn().execute(
            "INSERT OR IGNORE INTO agent_capabilities (alias, role, description, capabilities, messenger_enabled, special_instructions, project_path, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7)",
            rusqlite::params![alias, role, description, capabilities, instructions, project_path, now],
        )?;
        // 기존 행(이전 startup seed)에 project_path/special_instructions 가 비어있을 수 있으니 동기화.
        db.conn().execute(
            "UPDATE agent_capabilities SET project_path = ?2, special_instructions = ?3, updated_at = ?4 \
             WHERE alias = ?1 AND (project_path IS NULL OR project_path = '')",
            rusqlite::params![alias, project_path, instructions, now],
        )?;
        // agent_profiles — source='built_in', activated=0 (설치됨·미활성).
        let c2 = db.conn().execute(
            "INSERT OR IGNORE INTO agent_profiles (alias, ai_type, classification, execution_mode, display_name, source, activated, is_public, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'built_in', 0, 0, ?6, ?6)",
            rusqlite::params![alias, ai_type, classification, execution_mode, display_name, now],
        )?;
        if c1 > 0 || c2 > 0 {
            seeded += 1;
            tracing::info!(alias = %alias, "built-in 에이전트 seed 완료 (미활성)");
        }
    }
    Ok(seeded)
}

/// rc.315 — 머신 식별자(alias/hostname)를 안전한 slug 로 변환.
/// 소문자화 → 영숫자 아닌 문자는 '-' → 양끝 '-' 트림 → 연속 '-' 축약.
/// 예: "server-seoul.internal" → "server-seoul", "zalman_WSL" → "zalman-wsl".
/// 빈 결과면 "unknown" 폴백 (no silent fallback — 호출측에서 로그).
fn machine_slug(raw: &str) -> String {
    // rc.315 — 호스트명 첫 라벨만 사용해 깔끔한 머신명.
    //   예: "server-seoul.c.teeup-492907.internal" → "server-seoul", "whitegun-win.local" → "whitegun-win".
    let raw = raw.trim().split('.').next().unwrap_or(raw);
    let mut slug = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            for lc in ch.to_lowercase() {
                slug.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed
    }
}

/// rc.315 — per-머신 운영 에이전트 `<machine_slug>-master` 보장 (idempotent UPSERT).
///
/// 각 머신 daemon 이 boot 시 자기 머신의 워크플로우/운영 소유 에이전트가 존재함을 보장한다.
/// 머신 prefix 로 cross-machine name collision 방지 (seoul-master / zalman-master).
/// seed_builtin_agents 와 동일한 agent_capabilities + agent_profiles 양 테이블 UPSERT 패턴 재사용.
///
/// 마이그레이션: 레거시 `xgram-ops` 가 존재하고 `<slug>-master` 가 아직 없으면 rename.
/// 둘 다 있으면 xgram-ops 를 건드리지 않고 경고만 로그 (clobber 방지).
///
/// 생성된 alias 를 반환.
fn ensure_machine_master(data_dir: &std::path::Path, machine_slug: &str) -> anyhow::Result<String> {
    let alias = format!("{machine_slug}-master");
    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;
    // rc.315 — fresh install 에선 agent_capabilities(0035) 미생성 상태일 수 있어 먼저 migrate.
    // (마스터 요건: 머신-master 운영 에이전트는 '무조건' 존재 → 빈 DB 에서도 보장.)
    db.migrate()?;
    // 시간대 KST (CLAUDE.md 절대규칙 #4).
    let now = openxgram_core::time::kst_now().to_rfc3339();

    // ── 레거시 xgram-ops → <slug>-master 마이그레이션 ──
    // 안전: 대상 alias 가 이미 있으면 절대 clobber 안 함 (경고만).
    let legacy_exists: bool = db
        .conn()
        .query_row(
            "SELECT 1 FROM agent_capabilities WHERE alias = 'xgram-ops'",
            [],
            |_| Ok(()),
        )
        .is_ok();
    if legacy_exists {
        let master_exists: bool = db
            .conn()
            .query_row(
                "SELECT 1 FROM agent_capabilities WHERE alias = ?1",
                rusqlite::params![alias],
                |_| Ok(()),
            )
            .is_ok();
        if master_exists {
            // rc.322 — <slug>-master 가 이미 있고 레거시 xgram-ops 도 남아 있으면
            //   중복 ops 에이전트다. 머신당 ops 는 1개여야 하므로 레거시 xgram-ops
            //   행을 양 테이블에서 삭제하여 self-heal (master 가 정본).
            let del_caps = db.conn().execute(
                "DELETE FROM agent_capabilities WHERE alias = 'xgram-ops'",
                [],
            )?;
            let del_prof = db.conn().execute(
                "DELETE FROM agent_profiles WHERE alias = 'xgram-ops'",
                [],
            )?;
            tracing::warn!(
                target_alias = %alias,
                deleted_caps = del_caps,
                deleted_profiles = del_prof,
                "rc.322 중복 ops 정리: xgram-ops 와 {alias} 둘 다 존재 — 레거시 xgram-ops 삭제 (머신당 ops 1개 보장)",
            );
        } else {
            // alias 컬럼만 rename (양 테이블). 다른 컬럼(role/description/capabilities/project_path 등)은 보존.
            db.conn().execute(
                "UPDATE agent_capabilities SET alias = ?1, updated_at = ?2 WHERE alias = 'xgram-ops'",
                rusqlite::params![alias, now],
            )?;
            db.conn().execute(
                "UPDATE agent_profiles SET alias = ?1, updated_at = ?2 WHERE alias = 'xgram-ops'",
                rusqlite::params![alias, now],
            )?;
            tracing::info!(from = "xgram-ops", to = %alias, "rc.315 레거시 ops 에이전트 rename 완료");
        }
    }

    // ── 운영 working dir materialize (ACP spawn cwd) ──
    let agent_dir = data_dir.join("agents").join(&alias);
    if let Err(e) = std::fs::create_dir_all(&agent_dir) {
        tracing::warn!(error = %e, alias = %alias, "rc.315 ops 에이전트 디렉토리 생성 실패");
    }
    let project_path = agent_dir.to_string_lossy().to_string();

    let role = "운영 · 워크플로우 오케스트레이터";
    let description =
        "이 머신의 운영·워크플로우 소유 에이전트. cron·heartbeat·스케줄링·배포 등 머신 운영 전반을 책임진다.";
    let capabilities = r#"["workflow_orchestration","ops","scheduling"]"#;
    let display_name = format!("{machine_slug} 운영");

    // agent_capabilities — messenger_enabled=1 (peer 통신 활성, list_peers 노출).
    db.conn().execute(
        "INSERT INTO agent_capabilities (alias, role, description, capabilities, messenger_enabled, project_path, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6) \
         ON CONFLICT(alias) DO UPDATE SET \
             role = excluded.role, \
             description = excluded.description, \
             capabilities = excluded.capabilities, \
             messenger_enabled = 1, \
             project_path = COALESCE(NULLIF(agent_capabilities.project_path, ''), excluded.project_path), \
             updated_at = excluded.updated_at",
        rusqlite::params![alias, role, description, capabilities, project_path, now],
    )?;

    // agent_profiles — classification='special' (system/ops group), ai_type='claude', source='built_in', activated=1.
    db.conn().execute(
        "INSERT INTO agent_profiles (alias, ai_type, classification, execution_mode, machine, source, activated, is_public, created_at, updated_at) \
         VALUES (?1, 'claude', 'special', 'always', ?2, 'built_in', 1, 0, ?3, ?3) \
         ON CONFLICT(alias) DO UPDATE SET \
             ai_type = 'claude', \
             classification = 'special', \
             machine = excluded.machine, \
             updated_at = excluded.updated_at",
        rusqlite::params![alias, machine_slug, now],
    )?;

    // display_name 컬럼은 migration 0050 으로 agent_profiles 에 추가됨 — 별도 UPDATE 로 동기화.
    let _ = db.conn().execute(
        "UPDATE agent_profiles SET display_name = ?2 WHERE alias = ?1 AND (display_name IS NULL OR display_name = '')",
        rusqlite::params![alias, display_name],
    );

    Ok(alias)
}

/// rc.196 — retroactive register agents.
/// rc.200 — owner 식별: 자기 머신 tmux session 에 매칭되는 agent 만 등록.
/// 마스터의 본질: peer = 머신 X, peer = 터미널 (각 tmux session) O.
/// 각 머신 daemon 가 자기 owner agent (자기 머신 tmux session 에 매칭) 만 sub-keystore generate.
/// 다른 머신 owner 의 agent 는 sender hint (rc.193) 로 자동 upsert (receive 시).
fn retroactive_register_agents(data_dir: &std::path::Path, bind_port: u16) -> anyhow::Result<usize> {
    let pw = match openxgram_core::env::require_password() {
        Ok(p) => p,
        Err(_) => return Ok(0), // keystore password 없음 — skip (CLI mode 등)
    };

    let mut db = openxgram_db::Db::open(openxgram_db::DbConfig {
        path: openxgram_core::paths::db_path(data_dir),
        ..Default::default()
    })?;

    let candidates: Vec<String> = {
        let mut stmt = db.conn().prepare(
            "SELECT alias FROM agent_capabilities WHERE messenger_enabled = 1
             AND alias NOT IN (SELECT alias FROM peers)
             AND alias IS NOT NULL AND alias != ''",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    // rc.196 + 신규: self-heal — 자기 머신 peer 들의 address 가 도달 불가하면 reachable 로 교정.
    // 옛 'http://unknown'/빈값뿐 아니라 rc.196 가 박아둔 'http://127.0.0.1:47300'/0.0.0.0 도
    // 포함. cross-machine 직접 전송이 안 되던 본질 결함(주소가 localhost 로 박힘) 해결.
    // reachable 주소 검출 실패 시에는 self-heal 을 건너뜀 (localhost 유지 — 더 나빠지지 않음).
    let reachable = compute_self_reachable_url(data_dir, bind_port);
    let local_url = match &reachable {
        Some(u) => {
            // idempotent — 이미 reachable 인 row 는 건드리지 않음.
            // localhost/unknown/빈값/unspecified 만 교정 대상.
            let updated = db.conn().execute(
                "UPDATE peers SET address = ?1
                 WHERE eth_address IS NOT NULL AND eth_address != ''
                   AND (address = 'http://unknown' OR address = '' OR address IS NULL
                        OR address LIKE 'http://127.0.0.1:%'
                        OR address LIKE 'http://0.0.0.0:%'
                        OR address LIKE 'http://localhost:%')",
                rusqlite::params![u],
            ).unwrap_or(0);
            if updated > 0 {
                tracing::info!(updated = updated, addr = %u, "self-heal: peer address localhost/unknown → reachable");
            }
            u.clone()
        }
        None => {
            tracing::warn!(
                "self-heal skip — reachable 주소 검출 실패 (tailscale/LAN IP 없음). \
                 신규 peer 는 localhost 로 등록됨 — env XGRAM_TRANSPORT_PUBLIC_URL 또는 manifest tailscale_ip 권장"
            );
            format!("http://127.0.0.1:{bind_port}")
        }
    };

    if candidates.is_empty() {
        return Ok(0);
    }

    // rc.200 — owner 식별: 자기 머신 의 tmux session list 가져옴.
    // 자기 머신 tmux 에 매칭되는 alias 만 sub-keystore generate (owner).
    // 다른 머신 owner agent 는 sender hint receive 시 자동 upsert.
    let local_tmux_sessions: std::collections::HashSet<String> = {
        let mut s = std::collections::HashSet::new();
        // sync 함수에서 async tokio Command 회피 — std::process::Command 사용.
        // Windows daemon 가 wsl tmux 호출 가능하게 cross-platform.
        let (cmd, base_arg) = if cfg!(windows) {
            ("wsl", Some("tmux"))
        } else {
            ("tmux", None)
        };
        let mut c = std::process::Command::new(cmd);
        if let Some(a) = base_arg {
            c.arg(a);
        }
        if let Ok(out) = c
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()
        {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    s.insert(line.trim().to_string());
                }
            }
        }
        s
    };
    tracing::info!(
        local_tmux_count = local_tmux_sessions.len(),
        "rc.200 owner check: 자기 머신 tmux session list 수집"
    );

    tracing::info!(
        count = candidates.len(),
        "rc.196 retroactive: 옛 messenger_enabled=1 agent → sub-keystore + peer 자동 등록 시작 (owner filter 적용)"
    );

    use openxgram_keystore::{FsKeystore, Keystore};
    let ks = FsKeystore::new(openxgram_core::paths::keystore_dir(data_dir));
    let local_addr = local_url;

    let mut registered = 0;
    for alias in &candidates {
        // 'null', 'star [aoe-window]' 같은 invalid alias skip
        if alias == "null" || alias.contains('[') || alias.contains('\n') {
            continue;
        }
        // rc.245 — 매칭된 tmux session_name (peer 생성 후 session_identifier set 용).
        let mut matched_session_name: Option<String> = None;
        // rc.200 owner check — 자기 머신 tmux session 에 매칭되는 alias 만 등록.
        // local_tmux_sessions 가 비어있으면 owner check skip (tmux 없는 머신).
        // rc.232 — 매칭 session 이 실제 LLM 도는지까지 검증. shell/placeholder 부활 방지.
        if !local_tmux_sessions.is_empty() {
            let matched_session: Option<&String> = local_tmux_sessions.iter().find(|sn| {
                sn.as_str() == alias
                    || sn.starts_with(&format!("aoe_{alias}_"))
                    || sn.contains(alias.as_str())
            });
            match matched_session {
                None => {
                    tracing::debug!(alias = %alias, "rc.200 owner check: skip — 자기 머신 tmux 에 없음");
                    continue;
                }
                Some(sn) => {
                    // rc.232 — 매칭된 tmux session 의 pane 에 실제 LLM 이 도는지 검증.
                    // LLM 안 도는 운영 shell (term_*, xgramd, starian) 부활 차단.
                    if !tmux_session_runs_llm(sn) {
                        tracing::info!(alias = %alias, tmux = %sn, "rc.232 retroactive skip — pane 에 LLM 미검출 (shell/placeholder 부활 차단)");
                        continue;
                    }
                    // rc.245 — 매칭된 tmux session_name 을 기억해 peer 생성 직후 session_identifier set.
                    matched_session_name = Some(sn.clone());
                }
            }
        }
        let kp = match ks.load(alias, &pw) {
            Ok(k) => k,
            Err(_) => {
                if let Err(e) = ks.create(alias, &pw) {
                    tracing::warn!(alias = %alias, error = %e, "retroactive: keypair 생성 실패");
                    continue;
                }
                match ks.load(alias, &pw) {
                    Ok(k) => k,
                    Err(e) => {
                        tracing::warn!(alias = %alias, error = %e, "retroactive: keypair load 실패");
                        continue;
                    }
                }
            }
        };
        let pubkey_hex = hex::encode(kp.public_key_bytes());
        let eth_addr = kp.address.to_string();

        let add_ok = {
            let mut peer_store = openxgram_peer::PeerStore::new(&mut db);
            match peer_store.add_with_eth(
                alias,
                &pubkey_hex,
                &local_addr,
                Some(&eth_addr),
                openxgram_peer::PeerRole::Worker,
                Some("rc.196 retroactive (옛 messenger_enabled=1 agent 자동 등록)"),
            ) {
                Ok(_) => {
                    tracing::info!(alias = %alias, eth = %eth_addr, "retroactive: peer 등록 성공");
                    registered += 1;
                    true
                }
                Err(e) => {
                    tracing::debug!(alias = %alias, error = %e, "retroactive: peer add skip (이미 있거나 충돌)");
                    false
                }
            }
        };
        // rc.245 — 결정적 세션 매핑: 새로 만든 peer 에 session_identifier set.
        // format 은 collect_sessions local tmux entry 와 동일 ("tmux:<name>").
        // peer_store drop 후 db 재차용 (borrow 충돌 회피).
        if add_ok {
            if let Some(sn) = &matched_session_name {
                let sid = format!("tmux:{sn}");
                let _ = db.conn().execute(
                    "UPDATE peers SET session_identifier = ?1 \
                     WHERE alias = ?2 AND (session_identifier IS NULL OR session_identifier = '')",
                    rusqlite::params![&sid, &alias],
                );
            }
        }
    }

    tracing::info!(registered = registered, candidates = candidates.len(), "rc.196 retroactive 완료");
    Ok(registered)
}

#[cfg(test)]
mod rc365_tmux_inject_tests {
    use super::*;
    use openxgram_db::{Db, DbConfig};
    use openxgram_peer::{PeerRole, PeerStore};
    use tempfile::TempDir;

    fn fresh_db(tmp: &TempDir) -> Db {
        let cfg = DbConfig {
            path: tmp.path().join("db.sqlite"),
            ..Default::default()
        };
        let mut db = Db::open(cfg).unwrap();
        db.migrate().unwrap();
        db
    }

    // ── should_inject_inbound_to_tmux — 주입 대상 판별 ───────────────────────
    #[test]
    fn normal_peer_message_with_live_tmux_injects() {
        // 일반 대화 + 라이브 tmux → 주입.
        assert!(should_inject_inbound_to_tmux("hello there", true, false));
    }

    #[test]
    fn no_live_tmux_does_not_inject() {
        // tmux 세션 없음 → 주입 안 함(종전 ACP 경로 유지).
        assert!(!should_inject_inbound_to_tmux("hello there", false, false));
    }

    #[test]
    fn disabled_flag_blocks_injection() {
        // XGRAM_DISABLE_PEER_TMUX_INJECT=1 → tmux 살아있어도 주입 안 함.
        assert!(!should_inject_inbound_to_tmux("hello there", true, true));
    }

    #[test]
    fn agent_add_request_control_message_skipped() {
        // 제어 메시지(agent-add 핸드셰이크) → 주입 금지.
        assert!(!should_inject_inbound_to_tmux(
            "[AGENT_ADD_REQUEST] {\"id\":\"x\"}",
            true,
            false
        ));
    }

    #[test]
    fn agent_add_accept_control_message_skipped() {
        assert!(!should_inject_inbound_to_tmux(
            "[AGENT_ADD_ACCEPT] {\"id\":\"x\"}",
            true,
            false
        ));
    }

    #[test]
    fn empty_body_skipped() {
        // 빈 본문(ack/keepalive 류) → 주입 금지(스팸 방지).
        assert!(!should_inject_inbound_to_tmux("", true, false));
        assert!(!should_inject_inbound_to_tmux("   \n  ", true, false));
    }

    // ── is_acp_drivable — 수신자 신원 → 라우팅 분류 ──────────────────────────
    #[test]
    fn acp_drivable_true_for_registered_non_tmux_role() {
        // register_subagent 로 등록된 대화형 에이전트(role≠'tmux') → ACP-drivable=true.
        // (rc.365 의 핵심 근본원인: 이런 신원이 tmux 주입을 억제했다.)
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        db.conn()
            .execute(
                "INSERT INTO agent_capabilities (alias, role, description, capabilities, updated_at) \
                 VALUES ('star', 'portal-dev', 'd', '[]', '2026-01-01')",
                [],
            )
            .expect("insert cap");
        assert!(is_acp_drivable(&mut db, "star"));
    }

    #[test]
    fn acp_drivable_false_for_role_tmux() {
        // role='tmux' 로 명시된 순수 터미널 peer → ACP-drivable=false(tmux 경로).
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        db.conn()
            .execute(
                "INSERT INTO agent_capabilities (alias, role, description, capabilities, updated_at) \
                 VALUES ('term1', 'tmux', 'd', '[]', '2026-01-01')",
                [],
            )
            .expect("insert cap");
        assert!(!is_acp_drivable(&mut db, "term1"));
    }

    #[test]
    fn acp_drivable_false_for_unregistered_alias() {
        // agent_capabilities row 없음 → ACP-drivable=false.
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        assert!(!is_acp_drivable(&mut db, "ghost"));
    }

    // ── 수신자 alias resolve — peer pubkey → alias 매핑 ─────────────────────
    #[test]
    fn receiver_alias_resolves_from_peer_public_key() {
        // recv_alias 해석의 2순위(peer table pubkey lookup) 단위 검증.
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        let mut store = PeerStore::new(&mut db);
        let pubkey = "aa".repeat(33); // 66-hex placeholder pubkey
        store
            .add(
                "star",
                &pubkey,
                "http://127.0.0.1:17321",
                PeerRole::Primary,
                Some("test"),
            )
            .expect("add peer");
        let got = PeerStore::new(&mut db)
            .get_by_public_key(&pubkey)
            .expect("query")
            .map(|p| p.alias);
        assert_eq!(got.as_deref(), Some("star"));
    }

    // ── rc.366 — 죽은 spawnable 수신자 자동 spawn 판별 ──────────────────────────

    /// spawnable agent_capabilities + agent_profiles row 를 심는 헬퍼.
    fn insert_spawnable(db: &mut Db, alias: &str, role: &str, ai_type: &str, project_path: &str) {
        db.conn()
            .execute(
                "INSERT INTO agent_capabilities (alias, role, description, capabilities, project_path, updated_at) \
                 VALUES (?1, ?2, 'd', '[]', ?3, '2026-01-01')",
                rusqlite::params![alias, role, project_path],
            )
            .expect("insert cap");
        db.conn()
            .execute(
                "INSERT INTO agent_profiles (alias, ai_type, classification, execution_mode, is_public, created_at, updated_at) \
                 VALUES (?1, ?2, 'project', 'on_demand', 0, '2026-01-01', '2026-01-01')",
                rusqlite::params![alias, ai_type],
            )
            .expect("insert profile");
    }

    #[test]
    fn spawnable_true_when_ai_type_and_cwd_present() {
        // ai_type=claude + project_path 있음 → spawnable=true (role 무관: tmux 여도 기동 정보 있으면 깨운다).
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        insert_spawnable(&mut db, "bee", "tmux", "claude", "/home/llm/projects/x");
        assert!(is_recipient_spawnable(&mut db, "bee"));
    }

    #[test]
    fn spawnable_false_when_no_project_path() {
        // project_path 비어있음 → spawn 불가(cwd 없음).
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        insert_spawnable(&mut db, "bee", "portal-dev", "claude", "");
        assert!(!is_recipient_spawnable(&mut db, "bee"));
    }

    #[test]
    fn spawnable_false_when_no_profile_row() {
        // agent_profiles row 없음(capabilities 만) → JOIN 실패 → spawn 불가.
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        db.conn()
            .execute(
                "INSERT INTO agent_capabilities (alias, role, description, capabilities, project_path, updated_at) \
                 VALUES ('bee', 'portal-dev', 'd', '[]', '/home/llm/x', '2026-01-01')",
                [],
            )
            .expect("insert cap");
        assert!(!is_recipient_spawnable(&mut db, "bee"));
    }

    #[test]
    fn spawnable_false_for_unknown_ai_type() {
        // ai_type 이 알려진 ACP 어댑터에 매핑 안 됨 → spawn 불가.
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        insert_spawnable(&mut db, "bee", "portal-dev", "mystery-llm", "/home/llm/x");
        assert!(!is_recipient_spawnable(&mut db, "bee"));
    }

    #[test]
    fn spawnable_false_for_unregistered_alias() {
        let tmp = TempDir::new().unwrap();
        let mut db = fresh_db(&tmp);
        assert!(!is_recipient_spawnable(&mut db, "ghost"));
    }

    // ── should_spawn_inbound_recipient — spawn 트리거 판별 ─────────────────────
    #[test]
    fn dead_spawnable_recipient_triggers_spawn() {
        // 죽은(라이브 tmux 없음) + spawnable + rate-limit 미해당 → spawn.
        assert!(should_spawn_inbound_recipient("hi", false, true, false, false));
    }

    #[test]
    fn live_tmux_recipient_does_not_spawn() {
        // 라이브 tmux 있음 → rc.365 주입이 처리하므로 spawn 안 함.
        assert!(!should_spawn_inbound_recipient("hi", true, true, false, false));
    }

    #[test]
    fn non_spawnable_recipient_does_not_spawn() {
        // spawnable 아님(미등록/cwd 없음) → spawn 안 함(인박스 보존).
        assert!(!should_spawn_inbound_recipient("hi", false, false, false, false));
    }

    #[test]
    fn rate_limited_recipient_does_not_spawn() {
        // rate-limit 윈도우 내 → spawn 안 함(storm 방지).
        assert!(!should_spawn_inbound_recipient("hi", false, true, false, true));
    }

    #[test]
    fn disabled_flag_blocks_spawn() {
        // XGRAM_DISABLE_PEER_TMUX_INJECT=1 → spawn 안 함(종전 동작).
        assert!(!should_spawn_inbound_recipient("hi", false, true, true, false));
    }

    #[test]
    fn control_message_does_not_spawn() {
        // 제어 메시지(agent-add) → spawn 안 함.
        assert!(!should_spawn_inbound_recipient(
            "[AGENT_ADD_REQUEST] {}",
            false,
            true,
            false,
            false
        ));
        assert!(!should_spawn_inbound_recipient(
            "[AGENT_ADD_ACCEPT] {}",
            false,
            true,
            false,
            false
        ));
    }

    #[test]
    fn empty_body_does_not_spawn() {
        assert!(!should_spawn_inbound_recipient("", false, true, false, false));
        assert!(!should_spawn_inbound_recipient("   \n ", false, true, false, false));
    }

    // ── spawn rate-limit guard — check-and-set ────────────────────────────────
    #[test]
    fn spawn_ratelimit_blocks_second_attempt() {
        // 고유 alias — 다른 테스트와 전역 맵 공유하므로 충돌 회피.
        let alias = "rc366_ratelimit_probe_alias_unique";
        // 첫 시도 — 미마킹이므로 false(=진행 가능) + 마킹.
        assert!(!spawn_recently_attempted(alias));
        // 즉시 두 번째 — 윈도우 내이므로 true(=rate-limited).
        assert!(spawn_recently_attempted(alias));
    }
}
