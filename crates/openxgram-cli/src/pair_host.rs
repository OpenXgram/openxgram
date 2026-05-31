//! rc.209 — `xgram pair-host` : WSL ↔ Windows host daemon 자동 peer-pair.
//!
//! 본질:
//! - WSL standalone daemon 과 Windows host daemon 이 양립할 때, 둘 사이를 자동으로
//!   양방향 peer 등록한다. 사용자 manual 0.
//!
//! 동작 (peer-per-environment 패턴):
//! 1) Windows host daemon 검출 — 다음 URL 후보 순회 `GET /v1/health`:
//!    - http://127.0.0.1:47300              (WSL2 mirrored networking)
//!    - http://<resolv.conf nameserver>:47300 (전형적 WSL2 NAT — host IP)
//!    - http://host.docker.internal:47300    (Hyper-V fallback)
//! 2) 자기 (WSL) peer DB 에 host 를 명시 등록 (`xgram peer add` 동등 호출).
//!    host pubkey 를 직접 얻을 수 없으므로 placeholder (sec1 형식 dummy) 로 unverified add.
//!    Windows daemon 가 reply 시 sender_pubkey_hex 로 자동 upgrade (rc.193 mechanism).
//! 3) Windows host 측에 announce envelope POST — sender hint 포함.
//!    Windows daemon 의 process_inbound 가 sender hint 로 자동 peer upsert →
//!    Windows peers 에 WSL alias 등록 (이쪽도 사용자 manual 0).

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::{keystore_dir, manifest_path, MASTER_KEY_NAME};
use openxgram_core::time::kst_now;
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_peer::PeerRole;
use openxgram_transport::{send_envelope, Envelope};

use crate::peer::{run_peer, PeerAction};

/// 검출 후보 URL 목록 빌드 — host_url 우선순위 보존.
fn candidate_urls() -> Vec<String> {
    let mut urls: Vec<String> = vec!["http://127.0.0.1:47300".to_string()];

    // /etc/resolv.conf 의 nameserver (WSL2 host IP) — 두 번째 우선순위.
    if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("nameserver") {
                let ip = rest.trim();
                if !ip.is_empty() {
                    let url = format!("http://{ip}:47300");
                    if !urls.contains(&url) {
                        urls.push(url);
                    }
                    break;
                }
            }
        }
    }

    // host.docker.internal — Hyper-V fallback.
    urls.push("http://host.docker.internal:47300".to_string());
    urls
}

/// 후보 URL 의 /v1/health 확인. 첫 200 OK 응답 URL 을 반환.
async fn detect_host_daemon(candidates: &[String]) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    for url in candidates {
        let health_url = format!("{}/v1/health", url.trim_end_matches('/'));
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(url = %url, "pair-host: host daemon detected");
                return Some(url.clone());
            }
            Ok(resp) => {
                tracing::debug!(url = %url, status = %resp.status(), "pair-host: health non-200");
            }
            Err(e) => {
                tracing::debug!(url = %url, error = %e, "pair-host: health probe failed");
            }
        }
    }
    None
}

/// 자기 측 (WSL) WSL alias 결정 — install-manifest 의 machine.alias 우선,
/// fallback 으로 `wsl-<hostname>`.
fn resolve_self_alias(data_dir: &Path) -> String {
    if let Ok(manifest) = openxgram_manifest::InstallManifest::read(manifest_path(data_dir)) {
        if !manifest.machine.alias.is_empty() {
            return manifest.machine.alias;
        }
    }
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "wsl".to_string());
    format!("wsl-{host}")
}

/// 자기 측 transport URL — install-manifest tailscale_ip 우선, 없으면 loopback.
fn resolve_self_transport_url(data_dir: &Path) -> String {
    if let Ok(url) = std::env::var("XGRAM_TRANSPORT_PUBLIC_URL") {
        if !url.is_empty() {
            return url;
        }
    }
    if let Ok(manifest) = openxgram_manifest::InstallManifest::read(manifest_path(data_dir)) {
        if let Some(ip) = manifest.machine.tailscale_ip {
            return format!("http://{ip}:47300");
        }
    }
    "http://127.0.0.1:47300".to_string()
}

/// 자기 측 DB 에 windows-host peer add (placeholder pubkey 로 unverified).
/// 이미 등록되어 있으면 silent skip (PeerStore::add 에서 충돌 시 Err — 무시).
fn register_host_peer_local(data_dir: &Path, host_url: &str) -> Result<()> {
    // placeholder secp256k1 sec1 compressed pubkey — 0x02 prefix + 32 bytes 0x01.
    // 실제 verify 는 reply 받는 시점에 sender_pubkey_hex 로 자동 upgrade.
    let placeholder_pubkey = format!("02{}", "01".repeat(32));
    let action = PeerAction::Add {
        alias: "windows-host".to_string(),
        public_key_hex: placeholder_pubkey,
        address: host_url.to_string(),
        role: PeerRole::Primary,
        notes: Some("rc.209 auto pair-host (placeholder pubkey — upgrades on first reply)".into()),
    };
    match run_peer(data_dir, action) {
        Ok(_) => {
            println!("  ✓ local peer add: windows-host @ {host_url}");
            Ok(())
        }
        Err(e) => {
            // 이미 등록되어 있거나 alias 충돌 시 — 본질적으로 idempotent 하게 처리.
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("이미") || msg.contains("conflict") {
                println!("  (windows-host peer 이미 등록됨 — skip)");
                Ok(())
            } else {
                Err(e).context("local peer add (windows-host)")
            }
        }
    }
}

/// announce envelope 송신 — Windows daemon 의 process_inbound 가 sender hint 로
/// 자동 peer upsert (rc.193 mechanism).
async fn send_announce(
    data_dir: &Path,
    password: &str,
    host_url: &str,
    self_alias: &str,
    self_transport_url: &str,
) -> Result<()> {
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let signer = ks
        .load(MASTER_KEY_NAME, password)
        .context("pair-host: master keystore 로드 실패")?;
    let sender_addr = signer.address.to_string();
    let sender_pubkey_hex = hex::encode(signer.public_key_bytes());

    let body = format!(
        "xgr-pair-announce-v1\n{{\"alias\":\"{}\",\"transport_url\":\"{}\",\"pubkey\":\"{}\",\"role\":\"primary\"}}",
        self_alias, self_transport_url, sender_pubkey_hex
    );
    let signature_hex = hex::encode(signer.sign(body.as_bytes()));
    let payload_hex = hex::encode(body.as_bytes());

    // `to` 필드는 host 의 pubkey 인데 모름 — placeholder (수신측은 자기 pubkey 인지 확인 X,
    // rc.193 자동 upsert 는 sender hint 기반).
    let placeholder_to = format!("02{}", "01".repeat(32));

    let envelope = Envelope {
        from: sender_addr,
        to: placeholder_to,
        payload_hex,
        timestamp: kst_now(),
        signature_hex,
        nonce: Some(uuid::Uuid::new_v4().to_string()),
        conversation_id: Some(format!("pair-host-{}", uuid::Uuid::new_v4())),
        sender_alias: Some(self_alias.to_string()),
        sender_transport_url: Some(self_transport_url.to_string()),
        sender_pubkey_hex: Some(sender_pubkey_hex),
        recipient_alias: Some("windows-host".to_string()),
        envelope_type: None,
        ack_for_ulid: None,
        ack_status: None,
    };

    send_envelope(host_url, &envelope)
        .await
        .with_context(|| format!("announce POST 실패 ({host_url}/v1/message)"))?;
    println!("  ✓ announce envelope sent → {host_url} (Windows daemon 자동 upsert)");
    Ok(())
}

/// `xgram pair-host` entry — install.sh 가 WSL detect 시 자동 호출.
pub async fn run_pair_host(data_dir: &Path, password: &str) -> Result<()> {
    println!("==> xgram pair-host — WSL ↔ Windows host daemon 자동 peer-pair");

    let candidates = candidate_urls();
    println!("  검출 후보:");
    for c in &candidates {
        println!("    - {c}");
    }

    let host_url = match detect_host_daemon(&candidates).await {
        Some(u) => u,
        None => {
            bail!(
                "Windows host daemon 검출 실패. \n\
                 해결: (1) install.ps1 실행 후 Windows OpenXgram-Daemon 가동 확인 \n\
                       (2) 또는 수동 등록: xgram peer add --alias windows-host \\\n\
                       --public-key <hex 66> --address http://<host-ip>:47300"
            );
        }
    };

    let self_alias = resolve_self_alias(data_dir);
    let self_transport_url = resolve_self_transport_url(data_dir);
    println!("  self alias        : {self_alias}");
    println!("  self transport_url: {self_transport_url}");
    println!("  host url          : {host_url}");

    // 1) 자기 측 (WSL) DB 에 windows-host peer 등록.
    register_host_peer_local(data_dir, &host_url)?;

    // 2) Windows host 에 announce envelope 송신 → 수신측 자동 upsert.
    if let Err(e) = send_announce(
        data_dir,
        password,
        &host_url,
        &self_alias,
        &self_transport_url,
    )
    .await
    {
        // announce 실패해도 local peer add 는 이미 성공 — partial 성공으로 보고.
        eprintln!("  [경고] announce 실패: {e}");
        eprintln!("        → WSL 측 windows-host peer 는 등록됨. Windows 측은 수동 등록 필요.");
        return Err(anyhow!(
            "pair-host partial: local add ok, announce failed ({e})"
        ));
    }

    println!(
        "✓ paired with windows-host ({host_url}) — both peers registered (Windows side auto-upsert pending first reply)"
    );
    Ok(())
}
