//! xgram daemon — Phase 1 first PR: scheduler + transport server foreground.
//!
//! Phase 1 first PR: 단일 binary 가 foreground 로 실행. 종료 신호(Ctrl-C)
//! 까지 대기. systemd unit·fork·pid 파일 등은 후속 PR.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use openxgram_core::paths::db_path;
use openxgram_scheduler::{add_reflection_job, build_scheduler, NIGHTLY_REFLECTION_CRON};
use openxgram_transport::{spawn_server_with_metrics, MetricsProvider};
use std::sync::Arc;

const DEFAULT_BIND: &str = "127.0.0.1:47300";
const DEFAULT_GUI_BIND: &str = "127.0.0.1:47302";

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
        SocketAddr::new(std::net::IpAddr::V4(ip), 47300)
    } else {
        DEFAULT_BIND.parse().expect("DEFAULT_BIND parses")
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
    // UI-MESSENGER-SPEC v1.4 §20 W-4/W-9 — workflows.cron_expr 자동 등록.
    let dbp = db_path(&opts.data_dir);
    let wf_count = match register_workflow_cron_jobs(&mut scheduler, &dbp).await {
        Ok(n) => n,
        Err(e) => { tracing::warn!("workflow cron 등록 실패: {e}"); 0 }
    };
    scheduler.start().await.context("scheduler 시작 실패")?;
    println!("  ✓ reflection scheduler started + {wf_count} workflow cron job(s)");

    // Prometheus metrics provider — DB 카운트 매 scrape 마다 조회
    let data_dir_for_metrics = opts.data_dir.clone();
    let metrics: MetricsProvider = Arc::new(move || gather_db_metrics(&data_dir_for_metrics));
    let server = spawn_server_with_metrics(bind, Some(metrics))
        .await
        .context("transport server bind 실패")?;
    println!("  ✓ transport server bound: http://{}", server.bound_addr);

    // GUI HTTP API (`/v1/gui/*`) — Tauri 데스크톱 앱이 원격 daemon 데이터에 접근.
    // 별도 axum 서버, transport 와 다른 포트. 토큰 인증 (mcp_tokens 재사용).
    let gui_bind = opts
        .gui_bind
        .unwrap_or_else(|| DEFAULT_GUI_BIND.parse().expect("DEFAULT_GUI_BIND parses"));
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

    // rc.196 — retroactive register: messenger_enabled=1 인데 peer entry 없는 옛 agent 들의
    // sub-keystore + peer 자동 생성. 마스터의 portal/akashic 등 ui 토글만 켜고 rc.192 fix 이전
    // 등록된 agent 들이 mock 상태였던 본질 결함 해결.
    match retroactive_register_agents(&opts.data_dir) {
        Ok(n) if n > 0 => println!("  ✓ rc.196 retroactive: {n} 옛 agent → verified peer 자동 등록"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "rc.196 retroactive register 실패 (계속 진행)"),
    }

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
            let envelopes = received_for_task.drain_received();
            if tick_count % 60 == 0 {
                tracing::debug!(tick=tick_count, "inbound_processor: heartbeat (no envelopes recently)");
            }
            if envelopes.is_empty() {
                continue;
            }
            tracing::info!(count=envelopes.len(), tick=tick_count, "inbound_processor: envelopes drained, calling process_inbound");
            match process_inbound(&data_dir_clone, &envelopes) {
                Ok(_) => tracing::info!(count=envelopes.len(), "inbound_processor: process_inbound 성공"),
                Err(e) => tracing::warn!(error = %e, "inbound_processor: process_inbound 실패"),
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
    use openxgram_memory::{default_embedder, MessageStore, SessionStore};
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
    let embedder = default_embedder().context("embedder init 실패")?;
    tracing::debug!("process_inbound: embedder init ok");

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
        if peer_opt.is_none() {
            if let (Some(alias), Some(pubkey_hex)) =
                (env.sender_alias.as_deref(), env.sender_pubkey_hex.as_deref())
            {
                if verify_with_pubkey(pubkey_hex, &payload_bytes, &sig_bytes).is_ok() {
                    let addr = env
                        .sender_transport_url
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("http://unknown");
                    let mut peer_store = PeerStore::new(&mut db);
                    match peer_store.add_with_eth(
                        alias,
                        pubkey_hex,
                        addr,
                        Some(&env.from),
                        openxgram_peer::PeerRole::Worker,
                        Some("auto-registered via envelope sender hint (rc.193)"),
                    ) {
                        Ok(p) => {
                            tracing::info!(alias = %alias, eth = %env.from, "rc.193 auto-peer-upsert 성공 (sender hint + signature 검증 OK)");
                            peer_opt = Some(p);
                        }
                        Err(e) => {
                            // UNIQUE alias 충돌 등 — silent
                            tracing::debug!(alias = %alias, error = %e, "auto-peer-upsert skip (이미 있거나 alias 충돌)");
                            if let Ok(Some(p)) = PeerStore::new(&mut db).get_by_eth_address(&env.from) {
                                peer_opt = Some(p);
                            }
                        }
                    }
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
            // process_inbound 는 sync — block_in_place + block_on 으로 async tmux send-keys 호출
            // rc.219 — return bool 로 tmux inject 성공/실패 명시 (silent debug 제거).
            tmux_injected = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
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
                })
            });
        } else {
            tracing::warn!(
                envelope_to_pubkey_prefix = %env_to_short,
                "rc.219 recv_alias 미정 — recipient_alias hint X + peer table lookup 실패 + manifest 의 self alias 도 없음. tmux inject skip"
            );
        }

        // rc.219 — ACK envelope 송신. sender 측 outbound_queue.ack_at UPDATE 가능하도록.
        // ack_status: inbox_stored 는 항상 (위에서 insert 성공 후 도달).
        // tmux_injected 면 tmux_injected 로 격상.
        // nonce 는 envelope 의 것 (== outbound_queue.msg_ulid 와 다른 sender 측 generator. envelope.nonce 가 msg_ulid 매칭 키).
        // sender 측 outbound_queue.msg_ulid 는 sender 가 record 한 ulid. envelope.nonce 와 별개.
        // → 따라서 sender 가 outbound_queue INSERT 시 사용한 ulid 를 envelope 의 어떤 필드로 운반해야 매칭 가능.
        // 본 envelope 의 nonce 를 ulid 로 활용 (sender 가 record_outbox 시 동일 값 사용).
        let ack_for_ulid = env.nonce.clone();
        let ack_status_val = if tmux_injected { "tmux_injected" } else { "inbox_stored" };
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
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
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

/// UI-MESSENGER-SPEC v1.4 §20 W-4/W-9 — workflows.cron_expr 자동 trigger.
async fn register_workflow_cron_jobs(
    scheduler: &mut tokio_cron_scheduler::JobScheduler,
    db_path: &std::path::Path,
) -> Result<usize> {
    use openxgram_db::{Db, DbConfig};
    let mut db = Db::open(DbConfig { path: db_path.to_path_buf(), ..Default::default() })?;
    let mut stmt = db.conn().prepare(
        "SELECT id, name, cron_expr, yaml_body FROM workflows WHERE enabled=1 AND cron_expr IS NOT NULL AND cron_expr != ''"
    )?;
    let rows: Vec<(String, String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    let dbp_arc = std::sync::Arc::new(db_path.to_path_buf());
    let mut registered = 0usize;
    for (id, name, cron_expr, yaml_body) in rows {
        let dbp = dbp_arc.clone();
        let id_c = id.clone();
        let yaml = yaml_body.clone();
        let job = match tokio_cron_scheduler::Job::new_async(cron_expr.as_str(), move |_uuid, _l| {
            let dbp = dbp.clone();
            let id_c = id_c.clone();
            let yaml = yaml.clone();
            Box::pin(async move {
                let run_id = uuid::Uuid::new_v4().to_string();
                match openxgram_db::Db::open(openxgram_db::DbConfig {
                    path: dbp.as_ref().clone(), ..Default::default()
                }) {
                    Ok(mut db) => {
                        let _ = db.conn().execute(
                            "INSERT INTO workflow_runs (id, workflow_id, started_at, status, trigger_source) VALUES (?1, ?2, datetime('now'), 'running', 'cron')",
                            rusqlite::params![run_id, id_c],
                        );
                        let _ = crate::workflow_engine::run_workflow(&mut db, &id_c, &run_id, &yaml).await;
                    }
                    Err(e) => tracing::error!(error = %e, "workflow cron: DB open 실패"),
                }
            })
        }) {
            Ok(j) => j,
            Err(e) => { tracing::warn!("workflow '{}' cron 등록 실패: {}", name, e); continue; }
        };
        if scheduler.add(job).await.is_ok() {
            registered += 1;
            tracing::info!("workflow cron 등록: {} ({})", name, cron_expr);
        }
    }
    Ok(registered)
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
    let now = chrono::Utc::now().to_rfc3339();
    let mut seeded = 0;
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
        // INSERT OR IGNORE — 이미 있으면 messenger_enabled 만 1 로 update (auto-enable).
        let affected = db.conn().execute(
            "INSERT INTO agent_capabilities (alias, role, description, messenger_enabled, updated_at) \
             VALUES (?1, 'tmux', ?2, 1, ?3) \
             ON CONFLICT(alias) DO UPDATE SET messenger_enabled=1, updated_at=excluded.updated_at",
            rusqlite::params![&alias, &format!("auto-seed from tmux: {sn}"), &now],
        ).unwrap_or(0);
        if affected > 0 {
            seeded += 1;
            tracing::info!(alias = %alias, tmux = %sn, "rc.201 auto-seed: agent_capabilities");
        }
    }
    tracing::info!(seeded = seeded, total_sessions = local_sessions.len(), "rc.201 auto-seed 완료");
    Ok(seeded)
}

/// rc.196 — retroactive register agents.
/// rc.200 — owner 식별: 자기 머신 tmux session 에 매칭되는 agent 만 등록.
/// 마스터의 본질: peer = 머신 X, peer = 터미널 (각 tmux session) O.
/// 각 머신 daemon 가 자기 owner agent (자기 머신 tmux session 에 매칭) 만 sub-keystore generate.
/// 다른 머신 owner 의 agent 는 sender hint (rc.193) 로 자동 upsert (receive 시).
fn retroactive_register_agents(data_dir: &std::path::Path) -> anyhow::Result<usize> {
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

    // rc.196 추가 fix — self-peer (master alias) 의 address 가 'http://unknown' 또는 빈값이면
    // 실제 transport URL 로 update. retroactive 가 reply 못 보내던 본질 결함 해결.
    let local_url = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:47300".to_string());
    let updated = db.conn().execute(
        "UPDATE peers SET address = ?1
         WHERE (address = 'http://unknown' OR address = '' OR address IS NULL)
           AND eth_address IS NOT NULL AND eth_address != ''",
        rusqlite::params![local_url],
    ).unwrap_or(0);
    if updated > 0 {
        tracing::info!(updated = updated, addr = %local_url, "rc.196 self-peer address fix (http://unknown → real)");
    }

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
            }
            Err(e) => {
                tracing::debug!(alias = %alias, error = %e, "retroactive: peer add skip (이미 있거나 충돌)");
            }
        }
    }

    tracing::info!(registered = registered, candidates = candidates.len(), "rc.196 retroactive 완료");
    Ok(registered)
}
