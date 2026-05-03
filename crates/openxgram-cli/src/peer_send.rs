//! peer-aware send — alias 로 peer 조회 → /v1/message POST → last_seen touch.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_peer::PeerStore;
use openxgram_transport::{send_envelope, Envelope};

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

    if !address.starts_with("http://") && !address.starts_with("https://") {
        return Err(anyhow!(
            "address scheme 미지원: {} — 현재 http(s)://host:port 만 지원 (xmtp:// 등은 후속)",
            address
        ));
    }

    send_envelope(&address, &envelope)
        .await
        .with_context(|| format!("/v1/message POST 실패 ({address})"))?;

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

    // 2. concurrent send (각 send 는 reqwest blocking-async). JoinSet 으로 결과 수집
    let mut joinset = tokio::task::JoinSet::new();
    for (alias, address, public_key_hex) in targets {
        if !address.starts_with("http://") && !address.starts_with("https://") {
            joinset.spawn(async move {
                (
                    alias,
                    Err(format!("scheme 미지원: {address} (현재 http(s):// 만)")),
                )
            });
            continue;
        }
        let env = Envelope {
            from: sender_addr.clone(),
            to: public_key_hex,
            payload_hex: payload_hex.clone(),
            timestamp: now,
            signature_hex: signature_hex.clone(),
            nonce: Some(uuid::Uuid::new_v4().to_string()),
        };
        joinset.spawn(async move {
            let res = send_envelope(&address, &env)
                .await
                .map_err(|e| format!("{e}"));
            (alias, res)
        });
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
