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
        // rc.173 — 메신저 본질: unknown peer 도 inbox 에 저장 (Telegram/카카오톡 식).
        // 단 신원 미검증 표시. 검증 정책은 sender_label prefix 로 구분 (peer: vs unverified:).
        let peer_opt = match PeerStore::new(&mut db).get_by_eth_address(&env.from) {
            Ok(opt) => opt,
            Err(e) => {
                tracing::warn!(error = %e, "peer 조회 실패");
                continue;
            }
        };

        // payload decode — 실패 시 placeholder body
        let payload_bytes = hex::decode(&env.payload_hex).unwrap_or_default();
        let sig_bytes = hex::decode(&env.signature_hex).unwrap_or_default();

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
