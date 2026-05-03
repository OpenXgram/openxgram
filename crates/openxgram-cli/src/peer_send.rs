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
