//! peer-aware send — alias 로 peer 조회 → 주소 scheme 별 라우팅 → last_seen touch.
//! 지원 scheme: http(s):// (transport /v1/message), nostr(s):// (NostrSink publish).

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_memory::{default_embedder, MessageStore, SessionStore};
use openxgram_nostr::{
    encrypt_for_peer, keys_from_master, NostrKeys, NostrKind, NostrSink, NostrTag, PublicKey,
};
use openxgram_peer::PeerStore;
use openxgram_transport::{send_envelope, Envelope};

/// rc.204 — sender 측 outbox INSERT.
/// 송신 성공 직후 호출. session_title=`outbox-to-{alias}` (kind=`outbound`),
/// sender_label=`self:{sender_alias}` (receiver 측 `peer:*`/`unverified:*` 와 명확히 구분).
/// 실패해도 send 자체는 이미 성공 — outbox INSERT 실패는 WARN 로깅 후 진행 (PRD-2.0.2).
fn record_outbox(
    data_dir: &Path,
    peer_alias: &str,
    sender_alias_for_log: &str,
    body: &str,
    signature_hex: &str,
    conversation_id: Option<&str>,
) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let embedder = default_embedder().context("embedder init 실패")?;

    let session_title = format!("outbox-to-{}", peer_alias);
    let session = SessionStore::new(&mut db)
        .ensure_by_title(&session_title, "outbound")
        .with_context(|| format!("outbox session ensure 실패: {session_title}"))?;

    let sender_label = format!("self:{}", sender_alias_for_log);
    MessageStore::new(&mut db, embedder.as_ref())
        .insert(
            &session.id,
            &sender_label,
            body,
            signature_hex,
            conversation_id,
        )
        .with_context(|| format!("outbox L0 insert 실패 (session={})", session.id))?;
    tracing::info!(
        session_id = %session.id,
        sender = %sender_label,
        body_len = body.len(),
        "record_outbox: 송신 메시지 outbox 저장 완료"
    );
    Ok(())
}

/// rc.219 — outbound_queue 에 sent_at=now 인 row INSERT (ACK 추적용).
/// peer_send 의 직접 HTTP 송신 path 가 outbound_queue 를 거치지 않으므로 별도 INSERT.
/// msg_ulid = envelope.nonce (== receiver 측 ACK 의 ack_for_ulid 매칭 키).
/// 이후 v6_outbound_drain worker 가 ack_at IS NULL + sent_at threshold 도달 시 자동 재발송.
fn record_outbound_queue_sent(
    data_dir: &Path,
    msg_ulid: &str,
    target_alias: &str,
    envelope_json: &str,
) -> Result<()> {
    let mut db = open_db(data_dir)?;
    let conn = db.conn();
    let now_str = chrono::Utc::now().to_rfc3339();
    // 이미 송신 직후 — sent_at = now, attempts = 0 (ACK 카운터 시작점).
    conn.execute(
        "INSERT OR IGNORE INTO outbound_queue \
         (msg_ulid, target_machine, target_alias, body, attempts, enqueued_at, sent_at) \
         VALUES (?1, '', ?2, ?3, 0, ?4, ?4)",
        rusqlite::params![msg_ulid, target_alias, envelope_json, now_str],
    )
    .with_context(|| format!("outbound_queue INSERT 실패 (msg_ulid={msg_ulid})"))?;
    tracing::info!(
        msg_ulid = %msg_ulid,
        target_alias = %target_alias,
        "rc.219 record_outbound_queue_sent: ACK 추적용 row INSERT (sent_at=now, ack_at=NULL)"
    );
    Ok(())
}

/// 주소 scheme 별 라우트.
#[derive(Debug, Clone)]
pub enum SendRoute {
    Http(String),
    /// (relay ws URL, peer pubkey hex)
    Nostr {
        relay_ws: String,
        peer_pubkey: String,
    },
}

/// ADR-NOSTR-FALLBACK 정책 — http 실패 시 명시적 opt-in 환경변수 + relay URL 둘 다 있어야 fallback.
/// 둘 중 하나라도 없으면 None (silent fallback 절대 금지).
pub fn http_fallback_nostr_relay() -> Option<String> {
    if std::env::var("XGRAM_PEER_FALLBACK_NOSTR").ok().as_deref() != Some("1") {
        return None;
    }
    let relay = std::env::var("XGRAM_PEER_FALLBACK_NOSTR_RELAY").ok()?;
    if relay.trim().is_empty() {
        return None;
    }
    Some(relay)
}

/// `nostr://relay.example.com[:port]` → `ws://relay.example.com[:port]`,
/// `nostrs://...` → `wss://...`. http(s) 는 그대로 통과.
pub fn parse_route(address: &str, peer_pubkey_hex: &str) -> Result<SendRoute> {
    if let Some(rest) = address.strip_prefix("nostr://") {
        return Ok(SendRoute::Nostr {
            relay_ws: format!("ws://{rest}"),
            peer_pubkey: peer_pubkey_hex.to_string(),
        });
    }
    if let Some(rest) = address.strip_prefix("nostrs://") {
        return Ok(SendRoute::Nostr {
            relay_ws: format!("wss://{rest}"),
            peer_pubkey: peer_pubkey_hex.to_string(),
        });
    }
    if address.starts_with("http://") || address.starts_with("https://") {
        return Ok(SendRoute::Http(address.to_string()));
    }
    Err(anyhow!(
        "address scheme 미지원: {address} (지원: http(s)://, nostr(s)://)"
    ))
}

/// envelope 을 NIP-44 v2 로 peer pubkey 로 암호화 후 nostr event 로 publish.
/// content = ciphertext (NIP-44 wrap), kind = L0Message (30500), p-tag = peer_pubkey.
/// sender_keys.secret_key() 로 conversation_key 산출.
async fn send_via_nostr(
    sink: &NostrSink,
    sender_keys: &NostrKeys,
    relay_ws: &str,
    peer_pubkey_hex: &str,
    envelope: &Envelope,
) -> Result<()> {
    sink.add_relays([relay_ws])
        .await
        .map_err(|e| anyhow!("relay 추가 실패: {e}"))?;
    let body = serde_json::to_string(envelope).context("envelope 직렬화 실패")?;
    let peer_pk =
        PublicKey::from_hex(peer_pubkey_hex).map_err(|e| anyhow!("peer pubkey 파싱 실패: {e}"))?;
    let ciphertext = encrypt_for_peer(sender_keys.secret_key(), &peer_pk, &body)
        .map_err(|e| anyhow!("nip44 wrap 실패: {e}"))?;
    let p_tag = NostrTag::public_key(peer_pk);
    // NIP-33 addressable kind (30000~39999) 는 d-tag 필수.
    // envelope.nonce 가 있으면 그 값, 없으면 timestamp 사용 — 동일 envelope idempotent replay 가능.
    let d = envelope
        .nonce
        .clone()
        .unwrap_or_else(|| envelope.timestamp.timestamp_millis().to_string());
    sink.publish(NostrKind::L0Message, &ciphertext, Some(&d), vec![p_tag])
        .await
        .map_err(|e| anyhow!("nostr publish 실패: {e}"))?;
    Ok(())
}

pub async fn run_peer_send(
    data_dir: &Path,
    alias: &str,
    sender: Option<&str>,
    body: &str,
    password: &str,
) -> Result<()> {
    run_peer_send_with_conv(data_dir, alias, sender, body, password, None).await
}

/// rc.219 — `--wait-ack` 지원. `30s`, `60`, `500ms` 형식 파싱.
pub fn parse_wait_ack_duration(spec: &str) -> Result<std::time::Duration> {
    let s = spec.trim().to_lowercase();
    if let Some(rest) = s.strip_suffix("ms") {
        let n: u64 = rest
            .trim()
            .parse()
            .with_context(|| format!("--wait-ack ms 파싱 실패: '{spec}'"))?;
        return Ok(std::time::Duration::from_millis(n));
    }
    let stripped = s.strip_suffix('s').unwrap_or(&s);
    let n: u64 = stripped
        .trim()
        .parse()
        .with_context(|| format!("--wait-ack 초 파싱 실패: '{spec}' (지원 형식: '30s', '60', '500ms')"))?;
    Ok(std::time::Duration::from_secs(n))
}

/// rc.219 — `xgram peer send --wait-ack` 진입점. send + outbound_queue.ack_at 폴링 (1s 간격).
/// 반환: process exit code (0=ACK 수신, 1=timeout, 2=send 자체 실패).
pub async fn run_peer_send_with_ack_wait(
    data_dir: &Path,
    alias: &str,
    sender: Option<&str>,
    body: &str,
    password: &str,
    timeout: std::time::Duration,
) -> Result<i32> {
    let start = std::time::Instant::now();
    // 1. 일반 send — 내부에서 outbound_queue 에 ACK 추적용 row INSERT.
    if let Err(e) = run_peer_send_with_conv(data_dir, alias, sender, body, password, None).await {
        eprintln!("✗ send 실패: {e}");
        return Ok(2);
    }
    // 2. 가장 최근 outbound_queue row (alias 매칭) 의 msg_ulid 조회.
    //    record_outbound_queue_sent 가 직전에 INSERT 한 row.
    let msg_ulid = {
        let mut db = open_db(data_dir)?;
        let conn = db.conn();
        conn.query_row(
            "SELECT msg_ulid FROM outbound_queue \
             WHERE target_alias = ?1 ORDER BY enqueued_at DESC LIMIT 1",
            rusqlite::params![alias],
            |r| r.get::<_, String>(0),
        )
        .with_context(|| format!("outbound_queue 의 최근 row 조회 실패 (alias={alias})"))?
    };
    println!("⏳ ACK 대기 중 (msg_ulid={msg_ulid}, timeout={:?})", timeout);

    // 3. 1초 간격 폴링.
    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            println!(
                "⚠ ACK timeout (sent OK but no inbox confirmation in {:?}). msg_ulid={msg_ulid}",
                timeout
            );
            return Ok(1);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let ack: Option<(Option<String>, Option<String>)> = {
            let mut db = open_db(data_dir)?;
            let conn = db.conn();
            conn.query_row(
                "SELECT ack_at, ack_status FROM outbound_queue WHERE msg_ulid = ?1",
                rusqlite::params![msg_ulid],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .ok()
        };
        if let Some((Some(ack_at), status_opt)) = ack {
            let latency = start.elapsed();
            let status = status_opt.unwrap_or_else(|| "(unknown)".to_string());
            println!(
                "✓ delivered (ack_status={status}, latency={:.1}s, ack_at={ack_at})",
                latency.as_secs_f64()
            );
            return Ok(0);
        }
    }
}

/// 1.9.1.3 / 2.3.4 — conversation_id 동봉 버전. 메인 진입점은 `run_peer_send` (None) 호출.
/// rc.207 — 호출자가 conversation_id 미지정 시 자동 UUID 부여 (reply auto-correlate 보장).
pub async fn run_peer_send_with_conv(
    data_dir: &Path,
    alias: &str,
    sender: Option<&str>,
    body: &str,
    password: &str,
    conversation_id: Option<String>,
) -> Result<()> {
    // rc.207 — None 이면 자동 UUID 부여. 모든 outbound envelope 가 conversation_id 보유 →
    // 수신측 inject 형식에 conv:<id> 포함 → LLM 가 자기 send 와 시각적 link (polling 무의미).
    let conversation_id = Some(
        conversation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
    );
    let mut db = open_db(data_dir)?;
    let mut store = PeerStore::new(&mut db);
    let peer = store
        .get_by_alias(alias)?
        .ok_or_else(|| anyhow!("peer 없음: {alias}"))?;
    let address = peer.address.clone();

    // sender 명시되면 그 alias 의 sub-keystore 로 sign (rc.192 본질 fix).
    // 미지정 시 master keystore.
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let signer = match sender.filter(|s| !s.is_empty()) {
        Some(alias) => ks
            .load(alias, password)
            .with_context(|| format!("sender keystore '{alias}' 로드 실패"))?,
        None => ks
            .load(MASTER_KEY_NAME, password)
            .context("master 키 로드 실패")?,
    };
    let sender_addr = signer.address.to_string();
    let master = signer; // nostr fallback 호환 (변수명 유지)

    // body ECDSA 서명 — sender 의 keystore 로
    let signature_hex = hex::encode(master.sign(body.as_bytes()));
    let payload_hex = hex::encode(body.as_bytes());

    // rc.193 — sender 자동 등록 hint. 수신측 process_inbound 가 unknown sender 자동 peer upsert.
    // sender_alias: sender 명시된 경우 그 alias, 아니면 install-manifest 의 machine.alias (master).
    // sender_transport_url: env XGRAM_TRANSPORT_PUBLIC_URL 우선, 없으면 install-manifest tailscale_ip + port.
    // sender_pubkey_hex: signer 의 압축 공개키 (서명 검증 + 자동 peer 등록 용).
    let manifest_opt = openxgram_manifest::InstallManifest::read(
        openxgram_core::paths::manifest_path(data_dir),
    )
    .ok();
    let sender_alias = sender
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| manifest_opt.as_ref().map(|m| m.machine.alias.clone()));
    let sender_transport_url = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            manifest_opt
                .as_ref()
                .and_then(|m| m.machine.tailscale_ip.clone())
                .map(|ip| format!("http://{ip}:47300"))
        });
    let sender_pubkey_hex = Some(hex::encode(master.public_key_bytes()));

    let envelope = Envelope {
        from: sender_addr,
        to: peer.public_key_hex.clone(),
        payload_hex,
        timestamp: kst_now(),
        signature_hex,
        nonce: Some(uuid::Uuid::new_v4().to_string()),
        conversation_id,
        sender_alias,
        sender_transport_url,
        sender_pubkey_hex,
        recipient_alias: Some(alias.to_string()),
        envelope_type: None,
        ack_for_ulid: None,
        ack_status: None,
    };

    match parse_route(&address, &peer.public_key_hex)? {
        SendRoute::Http(url) => match send_envelope(&url, &envelope).await {
            Ok(()) => {}
            Err(e) => {
                // ADR-NOSTR-FALLBACK: 명시적 opt-in 일 때만 nostr 재시도
                if let Some(relay_ws) = http_fallback_nostr_relay() {
                    tracing::info!(error = %e, relay = %relay_ws, "http 실패 — XGRAM_PEER_FALLBACK_NOSTR opt-in 으로 nostr 재시도");
                    let nostr_keys = keys_from_master(&master)
                        .map_err(|e| anyhow!("nostr keys 변환 실패: {e}"))?;
                    let sink = NostrSink::new(nostr_keys.clone());
                    send_via_nostr(
                        &sink,
                        &nostr_keys,
                        &relay_ws,
                        &peer.public_key_hex,
                        &envelope,
                    )
                    .await?;
                    sink.shutdown().await;
                } else {
                    return Err(e).with_context(|| format!("/v1/message POST 실패 ({url})"));
                }
            }
        },
        SendRoute::Nostr {
            relay_ws,
            peer_pubkey,
        } => {
            let nostr_keys =
                keys_from_master(&master).map_err(|e| anyhow!("nostr keys 변환 실패: {e}"))?;
            let sink = NostrSink::new(nostr_keys.clone());
            send_via_nostr(&sink, &nostr_keys, &relay_ws, &peer_pubkey, &envelope).await?;
            sink.shutdown().await;
        }
    }

    // 통신 성공 → last_seen 갱신
    store.touch(alias)?;

    // 통신 성공 → outbox INSERT (send 성공 시만, partial 상태 회피)
    // sender_alias_for_log: 명시된 sender alias 또는 "master".
    // PeerStore lock 충돌 회피 위해 outbox INSERT 전 store drop 필요 — 다른 open_db 호출.
    drop(store);
    drop(db);
    let log_alias = envelope
        .sender_alias
        .clone()
        .unwrap_or_else(|| "master".to_string());
    if let Err(e) = record_outbox(
        data_dir,
        alias,
        &log_alias,
        body,
        &envelope.signature_hex,
        envelope.conversation_id.as_deref(),
    ) {
        tracing::warn!(error = %e, alias = %alias, "record_outbox 실패 (send 자체는 성공)");
    }

    // rc.219 — ACK 추적용 outbound_queue INSERT.
    // msg_ulid = envelope.nonce (receiver 측 ACK 의 ack_for_ulid 매칭 키).
    // sent_at = now (이미 송신 성공), ack_at = NULL.
    if let Some(ulid) = envelope.nonce.as_deref() {
        let envelope_json = serde_json::to_string(&envelope).unwrap_or_default();
        if let Err(e) = record_outbound_queue_sent(data_dir, ulid, alias, &envelope_json) {
            tracing::warn!(error = %e, msg_ulid = %ulid, "record_outbound_queue_sent 실패 (ACK 추적 불가, send 자체는 성공)");
        }
    }

    // rc.217 V11 — routing_rules apply: sender alias matches from_pattern AND
    // body 가 forward marker prefix 없을 때만 (loop 방지). action=forward 만 구현.
    // 매칭 syntax: simple glob — `*` (모두), `prefix*` (prefix match), `*suffix` (suffix),
    // `exact` (exact match). action=summarize_and_send / block 은 stub (forward 처럼 동작 or skip).
    if !body.starts_with("[forwarded-via:") {
        if let Err(e) = apply_routing_rules(
            data_dir,
            &log_alias,
            alias,
            body,
            password,
        )
        .await
        {
            tracing::warn!(error = %e, "routing_rules apply 실패 (send 자체는 성공)");
        }
    }

    println!(
        "✓ peer {alias} 에 메시지 전송 (size={} bytes)",
        envelope.payload_hex.len() / 2
    );
    Ok(())
}

/// rc.217 V11 — routing_rules forward 적용. send 성공 후 호출.
/// from_pattern 가 sender alias 매칭 + scope='Internal' + active=1 인 rule 의 to_pattern
/// 매칭 peer 들에 같은 body 를 forward (loop 방지 marker prefix 부착).
/// summarize_and_send, block 은 stub.
async fn apply_routing_rules(
    data_dir: &Path,
    sender_alias: &str,
    original_to: &str,
    body: &str,
    password: &str,
) -> Result<()> {
    // 1. rules + peer aliases 조회 (단일 DB scope, 모두 끝나면 drop).
    let rules: Vec<(String, String, String, String)> = {
        let mut db = open_db(data_dir)?;
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, from_pattern, to_pattern, action FROM routing_rules \
                 WHERE scope='Internal' AND active=1",
            )
            .context("routing_rules SELECT 준비 실패")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .context("routing_rules query_map 실패")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("routing_rules row 변환 실패")?);
        }
        out
    };
    if rules.is_empty() {
        return Ok(());
    }

    // 2. all peer aliases (to_pattern 매칭용).
    let all_aliases: Vec<String> = {
        let mut db = open_db(data_dir)?;
        let mut store = PeerStore::new(&mut db);
        store
            .list()
            .context("peer list 조회 실패")?
            .into_iter()
            .map(|p| p.alias)
            .collect()
    };

    for (rule_id, from_pattern, to_pattern, action) in rules {
        if !glob_match(&from_pattern, sender_alias) {
            continue;
        }
        match action.as_str() {
            "forward" => {
                for target in &all_aliases {
                    // self 또는 original 수신자에게 다시 안 보냄.
                    if target == sender_alias || target == original_to {
                        continue;
                    }
                    if !glob_match(&to_pattern, target) {
                        continue;
                    }
                    let fwd_body = format!(
                        "[forwarded-via:{}] from={} original_to={} :: {}",
                        rule_id, sender_alias, original_to, body
                    );
                    tracing::info!(
                        rule = %rule_id,
                        from = %sender_alias,
                        to = %target,
                        "routing_rule forward 적용"
                    );
                    // recursive — marker prefix 가 다음 호출의 routing 차단.
                    if let Err(e) = Box::pin(run_peer_send_with_conv(
                        data_dir, target, None, &fwd_body, password, None,
                    ))
                    .await
                    {
                        tracing::warn!(error = %e, rule = %rule_id, target = %target, "forward send 실패");
                    }
                }
            }
            "summarize_and_send" => {
                // stub — 첫 200자 truncate fallback.
                let truncated: String = body.chars().take(200).collect();
                let sum_body = format!(
                    "[forwarded-via:{}] summary from={} :: {}",
                    rule_id, sender_alias, truncated
                );
                for target in &all_aliases {
                    if target == sender_alias || target == original_to {
                        continue;
                    }
                    if !glob_match(&to_pattern, target) {
                        continue;
                    }
                    if let Err(e) = Box::pin(run_peer_send_with_conv(
                        data_dir, target, None, &sum_body, password, None,
                    ))
                    .await
                    {
                        tracing::warn!(error = %e, rule = %rule_id, target = %target, "summarize_and_send 실패");
                    }
                }
            }
            "block" => {
                tracing::info!(rule = %rule_id, from = %sender_alias, "routing_rule block 매칭 (send 는 이미 완료 — 후속 forward 만 차단)");
            }
            other => {
                tracing::warn!(rule = %rule_id, action = %other, "지원 안 되는 action");
            }
        }
    }
    Ok(())
}

/// simple glob — `*` (모두), `prefix*`, `*suffix`, exact.
fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return value.ends_with(suffix);
    }
    pattern == value
}

/// 다중 alias 에 동시 전송. 결과는 (alias, Ok|Err) 리스트로 반환.
/// 각 envelope 은 동일 body·동일 master 서명 — peer 별로 to(public_key)·address 만 달라짐.
pub async fn run_peer_broadcast(
    data_dir: &Path,
    aliases: &[String],
    body: &str,
    password: &str,
) -> Result<Vec<(String, std::result::Result<(), String>)>> {
    let mut db = open_db(data_dir)?;
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let master = ks
        .load(MASTER_KEY_NAME, password)
        .context("master 키 로드 실패")?;
    let sender_addr = master.address.to_string();
    let signature_hex = hex::encode(master.sign(body.as_bytes()));
    let payload_hex = hex::encode(body.as_bytes());
    let now = kst_now();
    // rc.193 sender hint — broadcast 도 자동 등록 hint 제공.
    let manifest_opt = openxgram_manifest::InstallManifest::read(
        openxgram_core::paths::manifest_path(data_dir),
    )
    .ok();
    let bcast_sender_alias = manifest_opt.as_ref().map(|m| m.machine.alias.clone());
    let bcast_sender_transport_url = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            manifest_opt
                .as_ref()
                .and_then(|m| m.machine.tailscale_ip.clone())
                .map(|ip| format!("http://{ip}:47300"))
        });
    let bcast_sender_pubkey_hex = Some(hex::encode(master.public_key_bytes()));

    // 1. db open 1회 — 모든 peer 의 (alias, address, public_key) 미리 resolve
    let mut targets: Vec<(String, String, String)> = Vec::new();
    {
        let mut store = PeerStore::new(&mut db);
        for alias in aliases {
            match store.get_by_alias(alias)? {
                Some(p) => targets.push((alias.clone(), p.address, p.public_key_hex)),
                None => {
                    return Err(anyhow!("peer 없음: {alias}"));
                }
            }
        }
    }
    drop(db);

    // 2. concurrent send (scheme 별 라우팅). JoinSet 으로 결과 수집
    let nostr_keys = keys_from_master(&master).map_err(|e| anyhow!("nostr keys 변환 실패: {e}"))?;
    let mut joinset = tokio::task::JoinSet::new();
    for (alias, address, public_key_hex) in targets {
        let route = match parse_route(&address, &public_key_hex) {
            Ok(r) => r,
            Err(e) => {
                joinset.spawn(async move { (alias, Err(e.to_string())) });
                continue;
            }
        };
        let env = Envelope {
            from: sender_addr.clone(),
            to: public_key_hex.clone(),
            payload_hex: payload_hex.clone(),
            timestamp: now,
            signature_hex: signature_hex.clone(),
            nonce: Some(uuid::Uuid::new_v4().to_string()),
            conversation_id: None,
            sender_alias: bcast_sender_alias.clone(),
            sender_transport_url: bcast_sender_transport_url.clone(),
            sender_pubkey_hex: bcast_sender_pubkey_hex.clone(),
            recipient_alias: Some(alias.clone()),
            envelope_type: None,
            ack_for_ulid: None,
            ack_status: None,
        };
        match route {
            SendRoute::Http(url) => {
                joinset.spawn(async move {
                    let res = send_envelope(&url, &env).await.map_err(|e| format!("{e}"));
                    (alias, res)
                });
            }
            SendRoute::Nostr {
                relay_ws,
                peer_pubkey,
            } => {
                let sink = NostrSink::new(nostr_keys.clone());
                let sender_keys = nostr_keys.clone();
                joinset.spawn(async move {
                    let res = send_via_nostr(&sink, &sender_keys, &relay_ws, &peer_pubkey, &env)
                        .await
                        .map_err(|e| e.to_string());
                    sink.shutdown().await;
                    (alias, res)
                });
            }
        }
    }

    let mut results = Vec::with_capacity(aliases.len());
    while let Some(r) = joinset.join_next().await {
        match r {
            Ok((alias, Ok(()))) => results.push((alias, Ok(()))),
            Ok((alias, Err(e))) => results.push((alias, Err(e))),
            Err(e) => results.push(("(join_error)".into(), Err(e.to_string()))),
        }
    }

    // 3. 성공한 peer 만 touch
    let mut db = open_db(data_dir)?;
    let mut store = PeerStore::new(&mut db);
    for (alias, res) in &results {
        if res.is_ok() {
            let _ = store.touch(alias);
        }
    }
    drop(store);
    drop(db);

    // 4. rc.204 — 성공한 peer 별 outbox INSERT (broadcast 도 sender DB trace 가능)
    let log_alias = bcast_sender_alias
        .clone()
        .unwrap_or_else(|| "master".to_string());
    for (alias, res) in &results {
        if res.is_ok() {
            if let Err(e) = record_outbox(
                data_dir,
                alias,
                &log_alias,
                body,
                &signature_hex,
                None,
            ) {
                tracing::warn!(error = %e, alias = %alias, "broadcast record_outbox 실패");
            }
        }
    }

    Ok(results)
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let path = db_path(data_dir);
    if !path.exists() {
        bail!("DB 미존재 ({}). `xgram init` 먼저 실행.", path.display());
    }
    let mut db = Db::open(DbConfig {
        path,
        ..Default::default()
    })
    .context("DB open 실패")?;
    db.migrate().context("DB migrate 실패")?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PK: &str = "0000000000000000000000000000000000000000000000000000000000000001";

    use std::sync::Mutex;
    static FALLBACK_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn http_fallback_three_cases() {
        let _g = FALLBACK_ENV_LOCK.lock().unwrap();
        // 1. opt-in false → None
        std::env::remove_var("XGRAM_PEER_FALLBACK_NOSTR");
        std::env::set_var("XGRAM_PEER_FALLBACK_NOSTR_RELAY", "ws://x");
        assert!(http_fallback_nostr_relay().is_none(), "opt-in 없으면 None");

        // 2. opt-in true 이지만 relay 미설정 → None
        std::env::set_var("XGRAM_PEER_FALLBACK_NOSTR", "1");
        std::env::remove_var("XGRAM_PEER_FALLBACK_NOSTR_RELAY");
        assert!(http_fallback_nostr_relay().is_none(), "relay 미설정 None");

        // 3. 둘 다 설정 → Some
        std::env::set_var("XGRAM_PEER_FALLBACK_NOSTR_RELAY", "ws://x");
        assert_eq!(http_fallback_nostr_relay().as_deref(), Some("ws://x"));

        std::env::remove_var("XGRAM_PEER_FALLBACK_NOSTR");
        std::env::remove_var("XGRAM_PEER_FALLBACK_NOSTR_RELAY");
    }

    #[test]
    fn parse_route_http() {
        let r = parse_route("http://127.0.0.1:47300", PK).unwrap();
        assert!(matches!(r, SendRoute::Http(ref u) if u == "http://127.0.0.1:47300"));
    }

    #[test]
    fn parse_route_https() {
        let r = parse_route("https://example.com", PK).unwrap();
        assert!(matches!(r, SendRoute::Http(_)));
    }

    #[test]
    fn parse_route_nostr_to_ws() {
        let r = parse_route("nostr://relay.example.com:7400", PK).unwrap();
        match r {
            SendRoute::Nostr {
                relay_ws,
                peer_pubkey,
            } => {
                assert_eq!(relay_ws, "ws://relay.example.com:7400");
                assert_eq!(peer_pubkey, PK);
            }
            _ => panic!("expected nostr route"),
        }
    }

    #[test]
    fn parse_route_nostrs_to_wss() {
        let r = parse_route("nostrs://relay.example.com", PK).unwrap();
        match r {
            SendRoute::Nostr { relay_ws, .. } => assert_eq!(relay_ws, "wss://relay.example.com"),
            _ => panic!("expected nostr route"),
        }
    }

    #[test]
    fn parse_route_unknown_scheme_errors() {
        let err = parse_route("xmtp://foo", PK).unwrap_err();
        assert!(err.to_string().contains("scheme 미지원"));
    }

    #[tokio::test]
    async fn published_event_carries_ciphertext_and_p_tag() {
        // MockRelay 에서 실제 publish 된 event content 가 plaintext 가 아니고 (NIP-44),
        // p-tag 가 peer pubkey 로 보존되는지 확인.
        use openxgram_nostr::{Filter, NostrSource};
        use std::sync::{Arc, Mutex};

        let relay = nostr_relay_builder::MockRelay::run().await.unwrap();
        let url = relay.url();

        let sender_keys = openxgram_nostr::NostrKeys::generate();
        let receiver_keys = openxgram_nostr::NostrKeys::generate();
        let peer_pubkey_hex = receiver_keys.public_key().to_hex();
        let sink = NostrSink::new(sender_keys.clone());

        let env = Envelope {
            from: "0xS".into(),
            to: peer_pubkey_hex.clone(),
            payload_hex: "cafe".into(),
            timestamp: kst_now(),
            signature_hex: "00".repeat(64),
            nonce: Some("nonce-x".into()),
            conversation_id: None,
            sender_alias: None,
            sender_transport_url: None,
            sender_pubkey_hex: None,
            recipient_alias: None,
            envelope_type: None,
            ack_for_ulid: None,
            ack_status: None,
        };

        // sink 가 relay 에 먼저 연결 (안정적인 publish 순서)
        sink.add_relays([url.clone()]).await.unwrap();

        let source = NostrSource::new(receiver_keys.clone());
        source.add_relays([url.clone()]).await.unwrap();

        let captured: Arc<Mutex<Option<openxgram_nostr::NostrEvent>>> = Arc::new(Mutex::new(None));
        let cap = captured.clone();
        let handle = source.spawn_listener(move |event| {
            let mut g = cap.lock().unwrap();
            if g.is_none() {
                *g = Some(event);
            }
        });

        let filter = Filter::new().kind(openxgram_nostr::NostrKindRaw::from(NostrKind::L0Message));
        source.subscribe(filter).await.unwrap();

        send_via_nostr(&sink, &sender_keys, &url, &peer_pubkey_hex, &env)
            .await
            .unwrap();

        let _ = tokio::time::timeout(tokio::time::Duration::from_secs(3), async {
            while captured.lock().unwrap().is_none() {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
        })
        .await;

        sink.shutdown().await;
        source.shutdown().await;
        let _ = handle.await;

        let ev = captured.lock().unwrap().clone().expect("event 수신 실패");
        let plain_body = serde_json::to_string(&env).unwrap();
        assert_ne!(ev.content, plain_body, "content 가 plaintext 면 안 됨");
        let has_p_tag = ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(|x| x.as_str()) == Some("p")
                && s.get(1).map(|x| x.as_str()) == Some(&peer_pubkey_hex)
        });
        assert!(has_p_tag, "p-tag 미존재");
    }

    #[tokio::test]
    async fn send_via_nostr_publishes_to_mock_relay() {
        // 실제 nostr 라우팅이 MockRelay 와 통신 + NIP-44 wrap 라운드트립 검증.
        use openxgram_nostr::decrypt_from_peer;

        let relay = nostr_relay_builder::MockRelay::run().await.unwrap();
        let url = relay.url();
        let ws_url = url.clone();

        // sender + receiver 페어 (recipient secret 으로 라운드트립)
        let sender_keys = openxgram_nostr::NostrKeys::generate();
        let receiver_keys = openxgram_nostr::NostrKeys::generate();
        let peer_pubkey = receiver_keys.public_key().to_hex();
        let sink = NostrSink::new(sender_keys.clone());

        let env = Envelope {
            from: "0xAAA".into(),
            to: peer_pubkey.clone(),
            payload_hex: "deadbeef".into(),
            timestamp: kst_now(),
            signature_hex: "00".repeat(64),
            nonce: Some("n1".into()),
            conversation_id: None,
            sender_alias: None,
            sender_transport_url: None,
            sender_pubkey_hex: None,
            recipient_alias: None,
            envelope_type: None,
            ack_for_ulid: None,
            ack_status: None,
        };
        send_via_nostr(&sink, &sender_keys, &ws_url, &peer_pubkey, &env)
            .await
            .unwrap();
        sink.shutdown().await;

        // 직접 NIP-44 라운드트립 — 같은 envelope JSON 으로 wrap → unwrap 가능 확인
        let body = serde_json::to_string(&env).unwrap();
        let ct = openxgram_nostr::encrypt_for_peer(
            sender_keys.secret_key(),
            &PublicKey::from_hex(&peer_pubkey).unwrap(),
            &body,
        )
        .unwrap();
        let pt =
            decrypt_from_peer(receiver_keys.secret_key(), &sender_keys.public_key(), &ct).unwrap();
        assert_eq!(pt, body);
    }
}
