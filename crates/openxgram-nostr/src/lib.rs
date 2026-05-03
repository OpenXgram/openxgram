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

use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use openxgram_keystore::Keypair;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
