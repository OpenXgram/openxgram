//! c) 친구 초대 — `xgram invite` (URL + QR) / `xgram friend accept <url>`.
//!
//! UX:
//!   A: xgram invite                 → oxg-friend://?... + QR (terminal)
//!   B: xgram friend accept <url>    → A peer 자동 등록 + 자기 정보 회신 메시지
//!   A: 회신 받음 → B peer 자동 등록 (양방향 완료)
//!
//! URL 포맷:
//!   oxg-friend://?alias=<>&pubkey=<>&address=<>&handle=<>
//!
//! handshake 메시지 magic prefix (3.4 와 동일):
//!   xgram-friend-accept-v1\n<sender_alias>\n<sender_pubkey>\n<sender_address>

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;

const SCHEME: &str = "oxg-friend://";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteToken {
    pub alias: String,
    pub pubkey_hex: String,
    pub address: String,
    pub handle: Option<String>,
}

impl InviteToken {
    /// URL → 토큰. 잘못된 형식이면 Err.
    pub fn parse(url: &str) -> Result<Self> {
        let rest = url
            .strip_prefix(SCHEME)
            .ok_or_else(|| anyhow!("oxg-friend:// 가 아님"))?;
        let qs = rest.trim_start_matches('?').trim_start_matches('/');
        let mut params: HashMap<String, String> = HashMap::new();
        for pair in qs.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| anyhow!("query 형식 오류: {pair}"))?;
            params.insert(k.into(), url_decode(v));
        }
        Ok(Self {
            alias: params
                .get("alias")
                .cloned()
                .ok_or_else(|| anyhow!("alias 누락"))?,
            pubkey_hex: params
                .get("pubkey")
                .cloned()
                .ok_or_else(|| anyhow!("pubkey 누락"))?,
            address: params
                .get("address")
                .cloned()
                .ok_or_else(|| anyhow!("address 누락"))?,
            handle: params.get("handle").cloned(),
        })
    }

    pub fn to_url(&self) -> String {
        let mut url = format!(
            "{SCHEME}?alias={}&pubkey={}&address={}",
            url_encode(&self.alias),
            url_encode(&self.pubkey_hex),
            url_encode(&self.address)
        );
        if let Some(h) = &self.handle {
            url.push_str(&format!("&handle={}", url_encode(h)));
        }
        url
    }
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `xgram invite` — 자기 master 정보 + transport address 로 초대 토큰 생성.
/// QR 도 함께 출력 (terminal 친화).
pub fn run_invite(data_dir: &Path, alias: &str, address: &str) -> Result<String> {
    use openxgram_core::paths::{keystore_dir, MASTER_KEY_NAME};
    use openxgram_keystore::{FsKeystore, Keystore};

    let pw = std::env::var("XGRAM_KEYSTORE_PASSWORD")
        .context("XGRAM_KEYSTORE_PASSWORD env 필요 (master 키 로드)")?;
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let master = ks.load(MASTER_KEY_NAME, &pw).context("master 로드")?;
    let pubkey_hex = hex::encode(master.public_key_bytes());

    let token = InviteToken {
        alias: alias.into(),
        pubkey_hex,
        address: address.into(),
        handle: None,
    };
    let url = token.to_url();
    println!("{url}");
    println!();
    println!("QR:");
    print_qr_to_terminal(&url)?;
    Ok(url)
}

fn print_qr_to_terminal(text: &str) -> Result<()> {
    use qrcode::QrCode;
    let code = QrCode::new(text).context("QR 생성")?;
    let s = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    println!("{s}");
    Ok(())
}

/// `xgram friend accept <oxg-friend://...>` —
/// 1) URL 파싱
/// 2) 자기 DB 에 peer add (idempotent)
/// 3) (옵션) 상대측에 accept handshake 메시지 송신 → 양방향 자동 완료
pub fn run_friend_accept(data_dir: &Path, url: &str) -> Result<()> {
    use openxgram_core::paths::{db_path, keystore_dir, MASTER_KEY_NAME};
    use openxgram_db::{Db, DbConfig};
    use openxgram_keystore::{FsKeystore, Keystore};
    use openxgram_peer::{PeerRole, PeerStore};

    let token = InviteToken::parse(url).context("invite URL 파싱")?;
    eprintln!("[friend] accept: {} ({})", token.alias, token.address);

    // 1) 자기 DB 에 peer add
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .context("DB open")?;
    db.migrate().context("DB migrate")?;
    let mut store = PeerStore::new(&mut db);
    if store.get_by_alias(&token.alias)?.is_some() {
        eprintln!("[friend] 이미 등록된 alias — peer add skip (idempotent)");
    } else {
        store
            .add(
                &token.alias,
                &token.pubkey_hex,
                &token.address,
                PeerRole::Worker,
                Some("via friend invite"),
            )
            .context("peer add")?;
        eprintln!("[friend] peer 등록 완료: {}", token.alias);
    }
    drop(store);
    drop(db);

    // 2) 자기 정보로 accept handshake 메시지 송신 (옵션 — keystore 패스워드 있을 때만).
    if let Ok(pw) = std::env::var("XGRAM_KEYSTORE_PASSWORD") {
        let ks = FsKeystore::new(keystore_dir(data_dir));
        let master = ks
            .load(MASTER_KEY_NAME, &pw)
            .context("master 로드 (handshake 송신용)")?;
        let my_pubkey = hex::encode(master.public_key_bytes());
        // 자기 address 는 manifest 에서 — 간단히 env 또는 default
        let my_address =
            std::env::var("XGRAM_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:47300".into());
        let my_alias = std::env::var("XGRAM_AGENT_ALIAS").unwrap_or_else(|_| "me".into());
        let body = build_accept_message(&my_alias, &my_pubkey, &my_address);

        let handle = tokio::runtime::Handle::try_current();
        let send_res = match handle {
            Ok(h) => h.block_on(crate::peer_send::run_peer_send(
                data_dir,
                &token.alias,
                None,
                &body,
                &pw,
            )),
            Err(_) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(crate::peer_send::run_peer_send(
                    data_dir,
                    &token.alias,
                    None,
                    &body,
                    &pw,
                ))
            }
        };
        match send_res {
            Ok(()) => eprintln!("[friend] handshake 메시지 송신 완료 (상대가 자동으로 우리 peer 추가)"),
            Err(e) => eprintln!("[friend] handshake 송신 실패 (상대가 수동 invite 필요): {e}"),
        }
    } else {
        eprintln!("[friend] XGRAM_KEYSTORE_PASSWORD 미설정 — handshake 송신 skip (단방향만 등록)");
    }
    Ok(())
}

pub fn build_accept_message(my_alias: &str, my_pubkey: &str, my_address: &str) -> String {
    format!("xgram-friend-accept-v1\n{my_alias}\n{my_pubkey}\n{my_address}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAccept {
    pub alias: String,
    pub pubkey_hex: String,
    pub address: String,
}

/// inbox 메시지 body 가 accept handshake 인지 파싱. None 이면 일반 메시지.
pub fn parse_accept_message(body: &str) -> Option<ParsedAccept> {
    let mut lines = body.lines();
    if lines.next()? != "xgram-friend-accept-v1" {
        return None;
    }
    let alias = lines.next()?.trim().to_string();
    let pubkey_hex = lines.next()?.trim().to_string();
    let address = lines.next()?.trim().to_string();
    if alias.is_empty() || pubkey_hex.is_empty() || address.is_empty() {
        return None;
    }
    Some(ParsedAccept {
        alias,
        pubkey_hex,
        address,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trip_via_url() {
        let t = InviteToken {
            alias: "starian".into(),
            pubkey_hex: "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798".into(),
            address: "http://1.2.3.4:47300".into(),
            handle: Some("starian.base.eth".into()),
        };
        let url = t.to_url();
        assert!(url.starts_with(SCHEME));
        let back = InviteToken::parse(&url).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn token_parse_rejects_wrong_scheme() {
        assert!(InviteToken::parse("https://example.com").is_err());
        assert!(InviteToken::parse("oxg-friend://?pubkey=x&address=y").is_err()); // missing alias
    }

    #[test]
    fn url_encode_handles_special_chars() {
        let t = InviteToken {
            alias: "한글-bot".into(),
            pubkey_hex: "abc".into(),
            address: "http://x:1/path?q=1".into(),
            handle: None,
        };
        let url = t.to_url();
        let back = InviteToken::parse(&url).unwrap();
        assert_eq!(back.alias, "한글-bot");
        assert_eq!(back.address, "http://x:1/path?q=1");
    }

    #[test]
    fn accept_message_round_trip() {
        let body = build_accept_message("alice", "02abc", "http://1.2.3.4:47300");
        let parsed = parse_accept_message(&body).unwrap();
        assert_eq!(parsed.alias, "alice");
        assert_eq!(parsed.pubkey_hex, "02abc");
        assert_eq!(parsed.address, "http://1.2.3.4:47300");
    }

    #[test]
    fn accept_parse_rejects_non_handshake() {
        assert!(parse_accept_message("just a normal message").is_none());
        assert!(parse_accept_message("xgram-friend-accept-v1\n").is_none()); // empty alias
    }
}
