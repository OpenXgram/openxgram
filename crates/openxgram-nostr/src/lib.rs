//! openxgram-nostr — Nostr 프로토콜 통합 (PRD-NOSTR-*).
//!
//! 핵심 가치: secp256k1 마스터 키 0-friction. OpenXgram 의 BIP44 derived
//! master keypair 를 그대로 Nostr Keys 로 사용. 별도 신원 발급 없음.
//!
//! Phase 2 baseline:
//! - NostrKind enum (kind 30050 ratchet / 30100 traits / 30200 patterns / 30300 memories /
//!   30400 episodes / 30500 messages / 30600 vault meta / 30700 peer update)
//! - Keys conversion (master keypair → nostr Keys)
//! - Event builder (kind + content + tags)
//! - Event signature 검증 헬퍼
//!
//! 후속 PR:
//! - PRD-NOSTR-03 NostrSink::publish (nostr-sdk client)
//! - PRD-NOSTR-04 NostrSource::subscribe
//! - PRD-NOSTR-05 application-layer ratchet
//! - PRD-NOSTR-06 self-host relay (nostr-relay-builder)

use nostr::nips::nip44::{self, Version};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use openxgram_keystore::Keypair;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// 외부 crate 가 nostr 타입을 직접 import 하지 않아도 되도록 re-export
pub use nostr::{
    Event as NostrEvent, Filter, Keys as NostrKeys, Kind as NostrKindRaw, PublicKey, SecretKey,
    Tag as NostrTag,
};
pub use nostr_sdk::RelayPoolNotification;

mod ratchet;
mod relay;
mod sink;
mod source;
pub use ratchet::{Ratchet, RatchetKey};
pub use relay::{NostrRelay, RelayConfig, DEFAULT_RELAY_PORT};
pub use sink::NostrSink;
pub use source::NostrSource;

#[derive(Debug, Error)]
pub enum NostrError {
    #[error("nostr crate error: {0}")]
    Nostr(String),

    #[error("invalid secret key: {0}")]
    InvalidSecret(String),

    #[error("hex decode: {0}")]
    Hex(#[from] hex::FromHexError),

    #[error("event signature verification failed")]
    SignatureVerify,
}

pub type Result<T> = std::result::Result<T, NostrError>;

/// 5층 메모리 → Nostr kind 매핑 (PRD-NOSTR-02). addressable kind 30000~ 사용.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NostrKind {
    RatchetKey, // 30050
    L4Trait,    // 30100
    L3Pattern,  // 30200
    L2Memory,   // 30300
    L1Episode,  // 30400
    L0Message,  // 30500
    VaultMeta,  // 30600
    PeerUpdate, // 30700
}

impl NostrKind {
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::RatchetKey => 30050,
            Self::L4Trait => 30100,
            Self::L3Pattern => 30200,
            Self::L2Memory => 30300,
            Self::L1Episode => 30400,
            Self::L0Message => 30500,
            Self::VaultMeta => 30600,
            Self::PeerUpdate => 30700,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RatchetKey => "ratchet_key",
            Self::L4Trait => "l4_trait",
            Self::L3Pattern => "l3_pattern",
            Self::L2Memory => "l2_memory",
            Self::L1Episode => "l1_episode",
            Self::L0Message => "l0_message",
            Self::VaultMeta => "vault_meta",
            Self::PeerUpdate => "peer_update",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ratchet_key" => Self::RatchetKey,
            "l4_trait" => Self::L4Trait,
            "l3_pattern" => Self::L3Pattern,
            "l2_memory" => Self::L2Memory,
            "l1_episode" => Self::L1Episode,
            "l0_message" => Self::L0Message,
            "vault_meta" => Self::VaultMeta,
            "peer_update" => Self::PeerUpdate,
            _ => return None,
        })
    }
}

impl From<NostrKind> for Kind {
    fn from(k: NostrKind) -> Self {
        Kind::from(k.as_u16())
    }
}

/// OpenXgram 마스터 keypair → Nostr Keys 변환. 동일 secp256k1 사용 → pubkey 일치.
pub fn keys_from_master(master: &Keypair) -> Result<Keys> {
    let secret_bytes = master.secret_key_bytes();
    let secret_hex = hex::encode(&secret_bytes);
    Keys::parse(&secret_hex).map_err(|e| NostrError::InvalidSecret(e.to_string()))
}

/// content + custom tags 로 Event 빌드 후 master keypair 로 서명.
/// addressable identifier (NIP-33) 는 `d` tag.
pub fn build_event(
    keys: &Keys,
    kind: NostrKind,
    content: &str,
    addressable_id: Option<&str>,
    extra_tags: Vec<Tag>,
) -> Result<Event> {
    let mut tags = Vec::new();
    if let Some(d) = addressable_id {
        tags.push(Tag::identifier(d));
    }
    tags.extend(extra_tags);
    let builder = EventBuilder::new(Kind::from(kind), content).tags(tags);
    builder
        .sign_with_keys(keys)
        .map_err(|e| NostrError::Nostr(e.to_string()))
}

/// Event 서명 검증 — pubkey + sig 일관성.
pub fn verify_event(event: &Event) -> Result<()> {
    event.verify().map_err(|_| NostrError::SignatureVerify)
}

/// NIP-44 v2 로 plaintext 를 peer 의 pubkey 로 암호화.
/// content 가 빈 문자열이면 명시적으로 InvalidSecret 으로 raise (silent 통과 금지).
pub fn encrypt_for_peer(
    sender_secret: &SecretKey,
    recipient: &PublicKey,
    content: &str,
) -> Result<String> {
    if content.is_empty() {
        return Err(NostrError::InvalidSecret("empty plaintext".into()));
    }
    nip44::encrypt(sender_secret, recipient, content, Version::V2)
        .map_err(|e| NostrError::Nostr(format!("nip44 encrypt: {e}")))
}

/// NIP-44 v2 ciphertext 를 sender 의 pubkey + 자신의 secret 으로 복호.
pub fn decrypt_from_peer(
    receiver_secret: &SecretKey,
    sender: &PublicKey,
    ciphertext: &str,
) -> Result<String> {
    nip44::decrypt(receiver_secret, sender, ciphertext)
        .map_err(|e| NostrError::Nostr(format!("nip44 decrypt: {e}")))
}

/// peer event content 를 복호 + WARN 로그 (실패 시) 까지 책임 — daemon 통합 단일 진입점.
/// ciphertext 가 NIP-44 v2 가 아니거나 ratchet/master 모두 실패 시 None 반환 (drop semantics).
pub fn try_unwrap_with_warn(
    receiver_master: &SecretKey,
    sender_master_pubkey: &PublicKey,
    sender_ratchet_pubkeys: &[PublicKey],
    ciphertext: &str,
) -> Option<String> {
    match unwrap_ciphertext_from_peer(
        receiver_master,
        sender_master_pubkey,
        sender_ratchet_pubkeys,
        ciphertext,
    ) {
        Ok(pt) => Some(pt),
        Err(e) => {
            tracing::warn!(target: "openxgram_nostr", error = %e, "incoming ciphertext decrypt 실패 — drop");
            None
        }
    }
}

/// peer 로부터 받은 NIP-44 v2 ciphertext 를 복호 — ratchet retained keys 우선 시도,
/// 실패 시 master secret 으로 fallback. 모두 실패하면 NostrError 반환.
/// sender_master_pubkey 는 Event.pubkey, sender_ratchet_pubkeys 는 kind 30050 announce 에서 lookup.
pub fn unwrap_ciphertext_from_peer(
    receiver_master: &SecretKey,
    sender_master_pubkey: &PublicKey,
    sender_ratchet_pubkeys: &[PublicKey],
    ciphertext: &str,
) -> Result<String> {
    for rpk in sender_ratchet_pubkeys {
        if let Ok(pt) = nip44::decrypt(receiver_master, rpk, ciphertext) {
            return Ok(pt);
        }
    }
    decrypt_from_peer(receiver_master, sender_master_pubkey, ciphertext)
}

/// envelope 를 peer 로 암호화하는 통합 진입점. ratchet 존재 시 ratchet 경로 (forward secrecy),
/// 없으면 master 경로 (단순). 둘 다 NIP-44 v2 ciphertext — receiver 는 단일 복호 경로로 처리.
/// 이중 wrap 은 동일 primitive 라 추가 보안 가치 없으므로 의도적으로 단일 적용.
pub fn wrap_envelope_for_peer(
    sender_master: &SecretKey,
    ratchet: Option<&mut Ratchet>,
    recipient: &PublicKey,
    plaintext: &str,
    unix_ts: u64,
) -> Result<String> {
    if let Some(r) = ratchet {
        return r.wrap(unix_ts, recipient, plaintext);
    }
    encrypt_for_peer(sender_master, recipient, plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_keystore::{FsKeystore, Keystore};
    use tempfile::tempdir;

    fn make_master() -> Keypair {
        let tmp = tempdir().unwrap();
        let ks = FsKeystore::new(tmp.path());
        let _ = ks.create("test", "pw").unwrap();
        ks.load("test", "pw").unwrap()
    }

    #[test]
    fn kind_round_trip() {
        for k in [
            NostrKind::RatchetKey,
            NostrKind::L4Trait,
            NostrKind::L3Pattern,
            NostrKind::L2Memory,
            NostrKind::L1Episode,
            NostrKind::L0Message,
            NostrKind::VaultMeta,
            NostrKind::PeerUpdate,
        ] {
            assert_eq!(NostrKind::parse(k.as_str()), Some(k));
            assert!(k.as_u16() >= 30000);
        }
    }

    #[test]
    fn keys_from_master_consistent() {
        let m = make_master();
        let k1 = keys_from_master(&m).unwrap();
        let k2 = keys_from_master(&m).unwrap();
        assert_eq!(k1.public_key(), k2.public_key());
        assert_eq!(k1.secret_key(), k2.secret_key());
    }

    #[test]
    fn build_and_verify_event() {
        let m = make_master();
        let keys = keys_from_master(&m).unwrap();
        let event = build_event(
            &keys,
            NostrKind::L0Message,
            "hello nostr",
            Some("session-1"),
            vec![],
        )
        .unwrap();
        verify_event(&event).unwrap();
        assert_eq!(event.kind, Kind::from(30500u16));
        // public_key 가 master 와 일치
        assert_eq!(event.pubkey, keys.public_key());
    }

    #[test]
    fn try_unwrap_with_warn_returns_none_on_failure() {
        let s = NostrKeys::generate();
        let r = NostrKeys::generate();
        let res = try_unwrap_with_warn(r.secret_key(), &s.public_key(), &[], "not-a-valid-ct");
        assert!(res.is_none());
    }

    #[test]
    fn try_unwrap_with_warn_returns_plaintext_on_success() {
        let s = NostrKeys::generate();
        let r = NostrKeys::generate();
        let pt = "ok";
        let ct = encrypt_for_peer(s.secret_key(), &r.public_key(), pt).unwrap();
        let res = try_unwrap_with_warn(r.secret_key(), &s.public_key(), &[], &ct);
        assert_eq!(res.as_deref(), Some(pt));
    }

    #[test]
    fn unwrap_ciphertext_master_path() {
        let sender = NostrKeys::generate();
        let receiver = NostrKeys::generate();
        let pt = "hello-master";
        let ct = encrypt_for_peer(sender.secret_key(), &receiver.public_key(), pt).unwrap();
        let back =
            unwrap_ciphertext_from_peer(receiver.secret_key(), &sender.public_key(), &[], &ct)
                .unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn unwrap_ciphertext_ratchet_path_then_master_fallback() {
        let sender = NostrKeys::generate();
        let receiver = NostrKeys::generate();
        let mut ratchet = Ratchet::default();
        let unix_ts = 1_700_000_000u64;
        let pt = "hello-ratchet";
        let ct = wrap_envelope_for_peer(
            sender.secret_key(),
            Some(&mut ratchet),
            &receiver.public_key(),
            pt,
            unix_ts,
        )
        .unwrap();
        let rpk = ratchet.current(unix_ts).public;
        // ratchet pk 알면 unwrap 성공
        let ok =
            unwrap_ciphertext_from_peer(receiver.secret_key(), &sender.public_key(), &[rpk], &ct)
                .unwrap();
        assert_eq!(ok, pt);
        // ratchet pk 미인지 시 master fallback 도 실패 → 명시 에러
        let err =
            unwrap_ciphertext_from_peer(receiver.secret_key(), &sender.public_key(), &[], &ct);
        assert!(err.is_err());
    }

    #[test]
    fn encrypt_for_peer_empty_plaintext_raises() {
        let s = NostrKeys::generate();
        let r = NostrKeys::generate();
        let err = encrypt_for_peer(s.secret_key(), &r.public_key(), "").unwrap_err();
        assert!(format!("{err}").contains("empty plaintext"));
    }

    #[test]
    fn wrap_envelope_master_path_round_trips() {
        let sender = NostrKeys::generate();
        let receiver = NostrKeys::generate();
        let pt = "envelope-json-1";
        let ct = wrap_envelope_for_peer(
            sender.secret_key(),
            None,
            &receiver.public_key(),
            pt,
            1_700_000_000,
        )
        .unwrap();
        let back = decrypt_from_peer(receiver.secret_key(), &sender.public_key(), &ct).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn wrap_envelope_ratchet_path_uses_ephemeral_key() {
        let sender = NostrKeys::generate();
        let receiver = NostrKeys::generate();
        let mut ratchet = Ratchet::default();
        let unix_ts = 1_700_000_000u64;
        let pt = "ratchet-pt";
        let ct = wrap_envelope_for_peer(
            sender.secret_key(),
            Some(&mut ratchet),
            &receiver.public_key(),
            pt,
            unix_ts,
        )
        .unwrap();
        // ratchet 경로 ciphertext 는 master secret 으로 복호 불가
        let master_decrypt = decrypt_from_peer(receiver.secret_key(), &sender.public_key(), &ct);
        assert!(master_decrypt.is_err());
        // ratchet pubkey 로 복호 가능
        let ratchet_pk = ratchet.current(unix_ts).public;
        let ok = decrypt_from_peer(receiver.secret_key(), &ratchet_pk, &ct).unwrap();
        assert_eq!(ok, pt);
    }

    #[test]
    fn addressable_event_has_d_tag() {
        let m = make_master();
        let keys = keys_from_master(&m).unwrap();
        let event = build_event(
            &keys,
            NostrKind::L4Trait,
            "trait body",
            Some("trait-name-1"),
            vec![],
        )
        .unwrap();
        let has_d = event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(|s| s.as_str()) == Some("d")
        });
        assert!(has_d);
    }
}
