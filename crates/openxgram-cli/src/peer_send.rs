//! peer-aware send — alias 로 peer 조회 → 주소 scheme 별 라우팅 → last_seen touch.
//! 지원 scheme: http(s):// (transport /v1/message), nostr(s):// (NostrSink publish).

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_nostr::{keys_from_master, NostrKind, NostrSink, NostrTag, PublicKey};
use openxgram_peer::PeerStore;
use openxgram_transport::{send_envelope, Envelope};

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

/// envelope 을 nostr event 로 wrap 하여 relay 에 publish.
/// content = envelope JSON, kind = L0Message (30500), p-tag = peer_pubkey.
async fn send_via_nostr(
    sink: &NostrSink,
    relay_ws: &str,
    peer_pubkey_hex: &str,
    envelope: &Envelope,
) -> Result<()> {
    sink.add_relays([relay_ws])
        .await
        .map_err(|e| anyhow!("relay 추가 실패: {e}"))?;
    let body = serde_json::to_string(envelope).context("envelope 직렬화 실패")?;
    let p_tag = NostrTag::public_key(
        PublicKey::from_hex(peer_pubkey_hex).map_err(|e| anyhow!("peer pubkey 파싱 실패: {e}"))?,
    );
    sink.publish(NostrKind::L0Message, &body, None, vec![p_tag])
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
    let mut db = open_db(data_dir)?;
    let mut store = PeerStore::new(&mut db);
    let peer = store
        .get_by_alias(alias)?
        .ok_or_else(|| anyhow!("peer 없음: {alias}"))?;
    let address = peer.address.clone();

    // sender 미지정 시 master 주소 사용
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let master = ks
        .load(MASTER_KEY_NAME, password)
        .context("master 키 로드 실패")?;
    let sender_addr = sender
        .map(str::to_string)
        .unwrap_or_else(|| master.address.to_string());

    // body ECDSA 서명 (master)
    let signature_hex = hex::encode(master.sign(body.as_bytes()));
    let payload_hex = hex::encode(body.as_bytes());

    let envelope = Envelope {
        from: sender_addr,
        to: peer.public_key_hex.clone(),
        payload_hex,
        timestamp: kst_now(),
        signature_hex,
        nonce: Some(uuid::Uuid::new_v4().to_string()),
    };

    match parse_route(&address, &peer.public_key_hex)? {
        SendRoute::Http(url) => {
            send_envelope(&url, &envelope)
                .await
                .with_context(|| format!("/v1/message POST 실패 ({url})"))?;
        }
        SendRoute::Nostr {
            relay_ws,
            peer_pubkey,
        } => {
            let nostr_keys =
                keys_from_master(&master).map_err(|e| anyhow!("nostr keys 변환 실패: {e}"))?;
            let sink = NostrSink::new(nostr_keys);
            send_via_nostr(&sink, &relay_ws, &peer_pubkey, &envelope).await?;
            sink.shutdown().await;
        }
    }

    // 통신 성공 → last_seen 갱신
    store.touch(alias)?;
    println!(
        "✓ peer {alias} 에 메시지 전송 (size={} bytes)",
        envelope.payload_hex.len() / 2
    );
    Ok(())
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
                joinset.spawn(async move {
                    let res = send_via_nostr(&sink, &relay_ws, &peer_pubkey, &env)
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

    #[test]
    fn parse_route_http() {
        let r = parse_route("http://127.0.0.1:7300", PK).unwrap();
        assert!(matches!(r, SendRoute::Http(ref u) if u == "http://127.0.0.1:7300"));
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
    async fn send_via_nostr_publishes_to_mock_relay() {
        // 실제 nostr 라우팅이 MockRelay 와 통신하는지 검증.
        let relay = nostr_relay_builder::MockRelay::run().await.unwrap();
        let url = relay.url();
        let ws_url = url.clone();

        // 가짜 master key 로 sink 생성
        let keys = openxgram_nostr::NostrKeys::generate();
        let peer_pubkey = openxgram_nostr::NostrKeys::generate().public_key().to_hex();
        let sink = NostrSink::new(keys);

        let env = Envelope {
            from: "0xAAA".into(),
            to: peer_pubkey.clone(),
            payload_hex: "deadbeef".into(),
            timestamp: kst_now(),
            signature_hex: "00".repeat(64),
            nonce: Some("n1".into()),
        };
        send_via_nostr(&sink, &ws_url, &peer_pubkey, &env)
            .await
            .unwrap();
        sink.shutdown().await;
    }
}
