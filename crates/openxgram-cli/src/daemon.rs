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

#[derive(Debug, Clone)]
pub struct DaemonOpts {
    pub data_dir: PathBuf,
    pub bind_addr: Option<SocketAddr>,
    pub reflection_cron: Option<String>,
    /// true 시 `tailscale ip --4` 결과를 기본 bind IP 로 사용. mTLS 는 WireGuard
    /// 터널이 네트워크 레이어에서 제공.
    pub tailscale: bool,
}

pub async fn run_daemon(opts: DaemonOpts) -> Result<()> {
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
    scheduler.start().await.context("scheduler 시작 실패")?;
    println!("  ✓ reflection scheduler started");

    // Prometheus metrics provider — DB 카운트 매 scrape 마다 조회
    let data_dir_for_metrics = opts.data_dir.clone();
    let metrics: MetricsProvider = Arc::new(move || gather_db_metrics(&data_dir_for_metrics));
    let server = spawn_server_with_metrics(bind, Some(metrics))
        .await
        .context("transport server bind 실패")?;
    println!("  ✓ transport server bound: http://{}", server.bound_addr);

    // inbound processor — 1초 주기로 server.drain_received() 한 후 envelope.from 매칭으로
    // peer.touch_by_eth_address. 매칭 실패는 silent (anonymous envelope 도 정상 도착).
    let data_dir_clone = opts.data_dir.clone();
    let received_arc = std::sync::Arc::new(server);
    let received_for_task = received_arc.clone();
    let processor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let envelopes = received_for_task.drain_received();
            if envelopes.is_empty() {
                continue;
            }
            if let Err(e) = process_inbound(&data_dir_clone, &envelopes) {
                tracing::warn!(error = %e, "inbound processor 처리 실패");
            }
        }
    });
    println!("  ✓ inbound processor running (1s interval)");

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
pub fn process_inbound(
    data_dir: &std::path::Path,
    envelopes: &[openxgram_transport::Envelope],
) -> Result<()> {
    use openxgram_db::{Db, DbConfig};
    use openxgram_keystore::verify_with_pubkey;
    use openxgram_memory::{default_embedder, MessageStore, SessionStore};
    use openxgram_peer::PeerStore;

    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open (inbound) 실패")?;
    db.migrate().context("DB migrate (inbound) 실패")?;
    let embedder = default_embedder().context("embedder init 실패")?;

    for env in envelopes {
        // 1. peer 조회 (envelope.from = eth_address)
        let peer = match PeerStore::new(&mut db).get_by_eth_address(&env.from) {
            Ok(Some(p)) => p,
            Ok(None) => {
                tracing::warn!(from = %env.from, "unknown peer — envelope drop (PRD-2.0.1)");
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "peer 조회 실패");
                continue;
            }
        };

        // 2. 서명 검증 (peer.public_key_hex 로 envelope.payload_hex bytes 검증)
        let payload_bytes = match hex::decode(&env.payload_hex) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, from = %env.from, "payload hex decode 실패");
                continue;
            }
        };
        let sig_bytes = match hex::decode(&env.signature_hex) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, from = %env.from, "signature hex decode 실패");
                continue;
            }
        };
        if let Err(e) = verify_with_pubkey(&peer.public_key_hex, &payload_bytes, &sig_bytes) {
            tracing::warn!(error = %e, from = %env.from, "서명 검증 실패 — drop (PRD-2.0.1)");
            continue;
        }

        // 3. session 매핑 — alias 별 inbox session ensure (PRD-2.0.3)
        let session_title = format!("inbox-from-{}", peer.alias);
        let session = match SessionStore::new(&mut db).ensure_by_title(&session_title, "inbound") {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "session ensure 실패");
                continue;
            }
        };

        // 4. L0 message 자동 저장 (PRD-2.0.2)
        let body = String::from_utf8_lossy(&payload_bytes).into_owned();
        if let Err(e) = MessageStore::new(&mut db, embedder.as_ref()).insert(
            &session.id,
            &env.from,
            &body,
            &env.signature_hex,
        ) {
            tracing::warn!(error = %e, "L0 message insert 실패");
            continue;
        }

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
