//! ANP message envelope.
//!
//! envelope 구조:
//! ```json
//! {
//!   "header": {
//!     "from_did": "did:wba:alice.com",
//!     "to_did":   "did:wba:bob.com",
//!     "type":     "task.request",
//!     "id":       "uuid",
//!     "created":  "2026-05-18T12:00:00+09:00",
//!     "signature": "hex(ECDSA secp256k1(canonical(body)))"
//!   },
//!   "body": { ... }
//! }
//! ```
//!
//! signature 는 body 의 canonical JSON 직렬화 SHA-256 다이제스트에 대한 ECDSA.

use openxgram_keystore::Keypair;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("hex error: {0}")]
    Hex(#[from] hex::FromHexError),

    #[error("missing header field: {0}")]
    MissingField(&'static str),

    #[error("signature missing")]
    UnsignedEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnpHeader {
    pub from_did: String,
    pub to_did: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub id: String,
    pub created: String,
    /// hex 인코딩된 ECDSA secp256k1 서명 (body canonical SHA-256 에 대한).
    /// 빌드 직후엔 None. `sign()` 후 채워짐.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnpEnvelope {
    pub header: AnpHeader,
    pub body: Value,
}

impl AnpEnvelope {
    /// 새 envelope (서명 전).
    pub fn new(
        from_did: impl Into<String>,
        to_did: impl Into<String>,
        msg_type: impl Into<String>,
        body: Value,
    ) -> Self {
        Self {
            header: AnpHeader {
                from_did: from_did.into(),
                to_did: to_did.into(),
                msg_type: msg_type.into(),
                id: new_message_id(),
                created: now_kst_rfc3339(),
                signature: None,
            },
            body,
        }
    }

    /// body 의 canonical JSON SHA-256.
    pub fn body_digest(&self) -> [u8; 32] {
        let canonical = canonical_json(&self.body);
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        hasher.finalize().into()
    }

    /// keypair 로 envelope 서명. header.signature 채움.
    pub fn sign(&mut self, keypair: &Keypair) {
        let digest = self.body_digest();
        let sig = keypair.sign(&digest);
        self.header.signature = Some(hex::encode(sig));
    }

    /// public key (hex, compressed sec1 33 bytes) 로 서명 검증.
    /// signature 가 없으면 `UnsignedEnvelope` 에러.
    pub fn verify_with_pubkey_hex(&self, public_key_hex: &str) -> Result<bool, EnvelopeError> {
        let sig_hex = self
            .header
            .signature
            .as_ref()
            .ok_or(EnvelopeError::UnsignedEnvelope)?;
        let sig = hex::decode(sig_hex)?;
        let digest = self.body_digest();
        match openxgram_keystore::verify_with_pubkey(public_key_hex, &digest, &sig) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// public key bytes 로 검증.
    pub fn verify_with_pubkey(&self, public_key: &[u8]) -> Result<bool, EnvelopeError> {
        self.verify_with_pubkey_hex(&hex::encode(public_key))
    }

    /// JSON 직렬화 (전송 형식).
    pub fn to_json(&self) -> Result<String, EnvelopeError> {
        Ok(serde_json::to_string(self)?)
    }

    /// JSON 역직렬화.
    pub fn from_json(s: &str) -> Result<Self, EnvelopeError> {
        Ok(serde_json::from_str(s)?)
    }
}

fn new_message_id() -> String {
    // RFC 4122 변형 UUID — 외부 의존 최소화를 위해 단순한 timestamp+random hex.
    // uuid crate 가 dev-dependency 가 아니므로 자체 생성.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(now.to_le_bytes());
    // 추가 엔트로피: 프로세스/스레드 id
    hasher.update(std::process::id().to_le_bytes());
    let h = hasher.finalize();
    hex::encode(&h[..16])
}

fn now_kst_rfc3339() -> String {
    // KST(+09:00) 고정.
    use chrono::{FixedOffset, Utc};
    let offset = FixedOffset::east_opt(9 * 3600).expect("KST +09:00");
    Utc::now()
        .with_timezone(&offset)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 결정적 JSON 직렬화 — 객체 키 알파벳 순.
pub fn canonical_json(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    out.insert(k.clone(), sort(&m[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort(v)).expect("sorted Value serialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_keystore::Keypair;
    use serde_json::json;

    #[test]
    fn envelope_round_trip_sign_verify() {
        let kp = Keypair::from_secret_bytes(&[0x42u8; 32]).unwrap();
        let mut env = AnpEnvelope::new(
            "did:wba:alice.com",
            "did:wba:bob.com",
            "task.request",
            json!({"hello": "world", "n": 42}),
        );
        assert!(env.header.signature.is_none());
        env.sign(&kp);
        assert!(env.header.signature.is_some());
        let pk = kp.public_key_bytes();
        assert!(env.verify_with_pubkey(&pk).unwrap());
    }

    #[test]
    fn envelope_verify_fails_on_tamper() {
        let kp = Keypair::from_secret_bytes(&[0x55u8; 32]).unwrap();
        let mut env = AnpEnvelope::new("did:wba:a", "did:wba:b", "ping", json!({"x": 1}));
        env.sign(&kp);
        // tamper body
        env.body = json!({"x": 999});
        let pk = kp.public_key_bytes();
        assert!(!env.verify_with_pubkey(&pk).unwrap());
    }

    #[test]
    fn envelope_json_round_trip() {
        let kp = Keypair::from_secret_bytes(&[0x66u8; 32]).unwrap();
        let mut env = AnpEnvelope::new("did:wba:a", "did:wba:b", "ping", json!({"y": [1, 2, 3]}));
        env.sign(&kp);
        let s = env.to_json().unwrap();
        let back = AnpEnvelope::from_json(&s).unwrap();
        assert_eq!(env, back);
        assert!(back.verify_with_pubkey(&kp.public_key_bytes()).unwrap());
    }

    #[test]
    fn unsigned_envelope_verify_errors() {
        let env = AnpEnvelope::new("did:wba:a", "did:wba:b", "ping", json!({}));
        let res = env.verify_with_pubkey(&[0u8; 33]);
        assert!(matches!(res, Err(EnvelopeError::UnsignedEnvelope)));
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let v = json!({"b": 2, "a": 1, "c": {"y": 2, "x": 1}});
        let s = canonical_json(&v);
        assert_eq!(s, r#"{"a":1,"b":2,"c":{"x":1,"y":2}}"#);
    }
}
