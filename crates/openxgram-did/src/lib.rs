//! # openxgram-did
//!
//! W3C DID Core + did:key + Verifiable Credentials Data Model 1.1 호환,
//! 한국디지털인증협회 OpenDID(opendid.org) + OmniOne Open DID(opendid.omnione.net) 매핑.
//!
//! 모든 식별자는 OpenXgram master secp256k1 키페어에서 derive — 별도 키 발급 없음.
//!
//! 표준 참조
//! - W3C DID Core: <https://www.w3.org/TR/did-core/>
//! - W3C did:key Method: <https://w3c-ccg.github.io/did-method-key/>
//! - W3C VC Data Model 1.1: <https://www.w3.org/TR/vc-data-model/>
//! - multicodec table: secp256k1-pub = 0xe7
//!
//! did:key 인코딩
//! `did:key:z` + base58btc(varint[0xe7, 0x01] || compressed_secp256k1_pubkey_33B)

use openxgram_keystore::Keypair;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// secp256k1-pub multicodec varint prefix (0xe7 + 0x01)
const SECP256K1_PUB_VARINT: [u8; 2] = [0xe7, 0x01];

#[derive(Debug, Error)]
pub enum DidError {
    #[error("invalid public key length: expected 33 (compressed secp256k1), got {0}")]
    InvalidPubkeyLength(usize),
    #[error("invalid did:key format")]
    InvalidDidKey,
    #[error("invalid network identifier: {0}")]
    InvalidNetwork(String),
    #[error("VC missing required field: {0}")]
    VcMissingField(&'static str),
    #[error("VC signature verification failed")]
    VcSignatureFailed,
    #[error("base58 decode error: {0}")]
    Base58(#[from] bs58::decode::Error),
    #[error("hex decode error: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("base64 decode error: {0}")]
    Base64(String),
    #[error("keystore error: {0}")]
    Keystore(String),
}

/// W3C did:key identifier — `did:key:z<base58btc(0xe7 0x01 || 33B pubkey)>`
pub fn did_key_from_master(master: &Keypair) -> Result<String, DidError> {
    let pk = master.public_key_bytes();
    if pk.len() != 33 {
        return Err(DidError::InvalidPubkeyLength(pk.len()));
    }
    let mut buf = Vec::with_capacity(2 + 33);
    buf.extend_from_slice(&SECP256K1_PUB_VARINT);
    buf.extend_from_slice(&pk);
    let encoded = bs58::encode(&buf).into_string();
    Ok(format!("did:key:z{encoded}"))
}

/// did:key 문자열에서 33바이트 compressed secp256k1 pubkey 복원.
pub fn pubkey_from_did_key(did: &str) -> Result<Vec<u8>, DidError> {
    let rest = did.strip_prefix("did:key:z").ok_or(DidError::InvalidDidKey)?;
    let raw = bs58::decode(rest).into_vec()?;
    if raw.len() != 35 || raw[0] != 0xe7 || raw[1] != 0x01 {
        return Err(DidError::InvalidDidKey);
    }
    Ok(raw[2..].to_vec())
}

/// W3C DID Document (JSON-LD) 생성.
///
/// 필수 필드: `@context`, `id`, `verificationMethod`, `authentication`,
/// `assertionMethod`. did:key 의 verificationMethod 는 `EcdsaSecp256k1VerificationKey2019`
/// (W3C Security Vocab) — publicKeyMultibase 로 표현.
pub fn did_document(did: &str, master: &Keypair) -> Result<Value, DidError> {
    let pk = master.public_key_bytes();
    if pk.len() != 33 {
        return Err(DidError::InvalidPubkeyLength(pk.len()));
    }
    let mut buf = Vec::with_capacity(35);
    buf.extend_from_slice(&SECP256K1_PUB_VARINT);
    buf.extend_from_slice(&pk);
    let pubkey_multibase = format!("z{}", bs58::encode(&buf).into_string());
    let key_id = format!("{did}#keys-1");
    Ok(json!({
        "@context": [
            "https://www.w3.org/ns/did/v1",
            "https://w3id.org/security/suites/secp256k1-2019/v1"
        ],
        "id": did,
        "verificationMethod": [{
            "id": key_id,
            "type": "EcdsaSecp256k1VerificationKey2019",
            "controller": did,
            "publicKeyMultibase": pubkey_multibase,
        }],
        "authentication": [key_id],
        "assertionMethod": [key_id],
    }))
}

/// 한국디지털인증협회 OpenDID(opendid.org) 호환 형식.
/// `did:opendid:{network}:{base58btc(sha256(pubkey))[..22]}`
/// - network: "mainnet" | "testnet" | 사용자 정의
/// - id: pubkey SHA-256 의 base58btc 앞 22자 (안정적·짧은 식별자)
pub fn opendid_kr_format(master: &Keypair, network: &str) -> Result<String, DidError> {
    if network.is_empty() || !network.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(DidError::InvalidNetwork(network.to_string()));
    }
    let pk = master.public_key_bytes();
    let digest = Sha256::digest(&pk);
    let id: String = bs58::encode(&digest[..]).into_string().chars().take(22).collect();
    Ok(format!("did:opendid:{network}:{id}"))
}

/// OmniOne Open DID(opendid.omnione.net) 호환 형식.
/// `did:omn:{base58btc(sha256(pubkey))[..22]}` — RaonSecure/OmniOneID
/// did-doc-architecture 의 관례적 prefix `omn`.
pub fn omnione_format(master: &Keypair) -> Result<String, DidError> {
    let pk = master.public_key_bytes();
    let digest = Sha256::digest(&pk);
    let id: String = bs58::encode(&digest[..]).into_string().chars().take(22).collect();
    Ok(format!("did:omn:{id}"))
}

/// W3C Verifiable Credential 1.1 발급. master 키로 JWS(ES256K) 서명된
/// proof 를 첨부한 JSON-LD VC 반환.
pub fn issue_vc(
    issuer_did: &str,
    subject_did: &str,
    claims: Value,
    master: &Keypair,
) -> Result<Value, DidError> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut subject = match claims {
        Value::Object(map) => Value::Object(map),
        other => json!({ "claim": other }),
    };
    if let Value::Object(ref mut m) = subject {
        m.insert("id".into(), Value::String(subject_did.to_string()));
    }
    let mut vc = json!({
        "@context": [
            "https://www.w3.org/2018/credentials/v1",
            "https://w3id.org/security/suites/secp256k1-2019/v1"
        ],
        "type": ["VerifiableCredential"],
        "issuer": issuer_did,
        "issuanceDate": now,
        "credentialSubject": subject,
    });
    let canonical = canonical_json(&vc);
    let digest = Sha256::digest(canonical.as_bytes());
    let sig_bytes = master.sign(&digest);
    let jws = base64url_encode(&sig_bytes);
    let proof = json!({
        "type": "EcdsaSecp256k1Signature2019",
        "created": now,
        "proofPurpose": "assertionMethod",
        "verificationMethod": format!("{issuer_did}#keys-1"),
        "jws": jws,
    });
    if let Value::Object(ref mut m) = vc {
        m.insert("proof".into(), proof);
    }
    Ok(vc)
}

/// VC 검증 — proof.jws 를 issuer_pubkey (compressed sec1 33 bytes) 로 ECDSA 검증.
pub fn verify_vc(vc: &Value, issuer_pubkey: &[u8]) -> Result<bool, DidError> {
    let obj = vc.as_object().ok_or(DidError::VcMissingField("root"))?;
    let proof = obj.get("proof").ok_or(DidError::VcMissingField("proof"))?.clone();
    let jws = proof
        .get("jws")
        .and_then(|v| v.as_str())
        .ok_or(DidError::VcMissingField("proof.jws"))?;
    let sig = base64url_decode(jws)?;
    let mut without_proof = obj.clone();
    without_proof.remove("proof");
    let body = Value::Object(without_proof);
    let canonical = canonical_json(&body);
    let digest = Sha256::digest(canonical.as_bytes());
    let pubkey_hex = hex::encode(issuer_pubkey);
    openxgram_keystore::verify_with_pubkey(&pubkey_hex, &digest, &sig)
        .map_err(|_| DidError::VcSignatureFailed)?;
    Ok(true)
}

/// 결정적 JSON 직렬화 — 객체 키를 알파벳 순으로 정렬.
fn canonical_json(v: &Value) -> String {
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

fn base64url_encode(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD.encode(bytes)
}

fn base64url_decode(s: &str) -> Result<Vec<u8>, DidError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|e| DidError::Base64(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// did:key secp256k1 인코딩 명세 검증
    /// (multicodec 0xe7 + varint 0x01 + 33B compressed pubkey, base58btc + 'z' prefix)
    /// W3C did:key 의 secp256k1 식별자는 항상 `did:key:zQ3sh` 로 시작.
    #[test]
    fn did_key_secp256k1_zq3sh_prefix() {
        let pk = hex::decode("02b97c30de767f084ce3080168ee293053ba33b235d7116a3263d29f1450936b71")
            .unwrap();
        let mut buf = Vec::new();
        buf.extend_from_slice(&SECP256K1_PUB_VARINT);
        buf.extend_from_slice(&pk);
        let did = format!("did:key:z{}", bs58::encode(&buf).into_string());
        assert!(
            did.starts_with("did:key:zQ3sh"),
            "did:key secp256k1 must have zQ3sh prefix: got {did}"
        );
        let recovered = pubkey_from_did_key(&did).unwrap();
        assert_eq!(recovered, pk);
    }

    #[test]
    fn did_key_round_trip() {
        let secret = [0x11u8; 32];
        let kp = Keypair::from_secret_bytes(&secret).unwrap();
        let did = did_key_from_master(&kp).unwrap();
        assert!(did.starts_with("did:key:z"));
        let pk = pubkey_from_did_key(&did).unwrap();
        assert_eq!(pk, kp.public_key_bytes());
    }

    #[test]
    fn did_document_required_fields() {
        let secret = [0x22u8; 32];
        let kp = Keypair::from_secret_bytes(&secret).unwrap();
        let did = did_key_from_master(&kp).unwrap();
        let doc = did_document(&did, &kp).unwrap();
        assert!(doc.get("@context").is_some());
        assert_eq!(doc["id"].as_str().unwrap(), did);
        assert!(doc["verificationMethod"].is_array());
        assert!(doc["authentication"].is_array());
        assert!(doc["assertionMethod"].is_array());
        let vm0 = &doc["verificationMethod"][0];
        assert_eq!(vm0["type"], "EcdsaSecp256k1VerificationKey2019");
        assert!(vm0["publicKeyMultibase"].as_str().unwrap().starts_with('z'));
    }

    #[test]
    fn opendid_kr_format_validates_network() {
        let kp = Keypair::from_secret_bytes(&[0x33u8; 32]).unwrap();
        let did = opendid_kr_format(&kp, "mainnet").unwrap();
        assert!(did.starts_with("did:opendid:mainnet:"));
        assert!(opendid_kr_format(&kp, "bad net").is_err());
    }

    #[test]
    fn omnione_format_prefix() {
        let kp = Keypair::from_secret_bytes(&[0x44u8; 32]).unwrap();
        let did = omnione_format(&kp).unwrap();
        assert!(did.starts_with("did:omn:"));
    }

    #[test]
    fn vc_issue_verify_round_trip() {
        let kp = Keypair::from_secret_bytes(&[0x55u8; 32]).unwrap();
        let issuer = did_key_from_master(&kp).unwrap();
        let subject = "did:key:zQ3shokFTS3brHcDQrn82RUDfCZESWL1ZdCEJwekUDPQiYBme";
        let claims = json!({"name": "Akashic", "level": 4});
        let vc = issue_vc(&issuer, subject, claims, &kp).unwrap();
        let pk = kp.public_key_bytes();
        assert!(verify_vc(&vc, &pk).unwrap());

        let mut tampered = vc.clone();
        tampered["credentialSubject"]["level"] = json!(99);
        assert!(verify_vc(&tampered, &pk).is_err());
    }
}
