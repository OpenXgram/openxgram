//! # did:wba — Web-Based Agent DID method
//!
//! ANP(Agent Network Protocol) 커뮤니티 변형 DID 메서드.
//! 도메인 + 경로 기반 식별자를 HTTP(S) 해석으로 DID Document 까지 펼친다.
//!
//! ## 형식
//!
//! - `did:wba:DOMAIN`             → `https://DOMAIN/.well-known/did.json`
//! - `did:wba:DOMAIN:path`        → `https://DOMAIN/path/did.json`
//! - `did:wba:DOMAIN:a:b:c`       → `https://DOMAIN/a/b/c/did.json`
//!
//! ## 참고
//! - W3C DID Core: <https://www.w3.org/TR/did-core/>
//! - ANP Whitepaper: <https://agent-network-protocol.com/specs/white-paper.html>
//! - draft-mahy-did-web-server (참고만)
//!
//! 이 모듈은 **resolver + validator + (보조) generator** 만 제공한다.
//! 실제 HTTP 호출은 호출자가 `resolve_url`을 받아 수행하는 분리 설계.
//! 이렇게 하면 reqwest/blocking/async 어느 클라이언트에서도 재사용 가능 +
//! 단위 테스트가 HTTP 의존성 없이 가능하다.

use openxgram_keystore::Keypair;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::DidError;

/// secp256k1-pub multicodec varint prefix (0xe7 + 0x01)
const SECP256K1_PUB_VARINT: [u8; 2] = [0xe7, 0x01];

/// did:wba 식별자의 구조화된 표현.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WbaDid {
    /// 도메인 부분 (예: `example.com`, `example.com:8443`).
    /// 포트 포함 가능하나 wba 형식에서 `:` 가 path separator 와 충돌하므로
    /// 포트는 percent-encoding 변형 `%3A` 또는 별도 규약 사용 — 현재는 `:` 그대로
    /// 받되 첫 segment 만 도메인으로 취급.
    pub domain: String,
    /// path segments. 비어 있으면 `.well-known/did.json` 으로 해석.
    pub path_segments: Vec<String>,
}

impl WbaDid {
    /// did:wba 문자열 파싱.
    ///
    /// `did:wba:example.com`             → domain=example.com, segments=[]
    /// `did:wba:example.com:alice`       → domain=example.com, segments=[alice]
    /// `did:wba:example.com:agents:alice`→ domain=example.com, segments=[agents, alice]
    pub fn parse(did: &str) -> Result<Self, DidError> {
        let rest = did
            .strip_prefix("did:wba:")
            .ok_or(DidError::InvalidDidKey)?;
        if rest.is_empty() {
            return Err(DidError::InvalidDidKey);
        }
        let mut parts = rest.split(':');
        let domain = parts.next().ok_or(DidError::InvalidDidKey)?.to_string();
        if domain.is_empty() || !is_valid_domain(&domain) {
            return Err(DidError::InvalidDidKey);
        }
        let path_segments: Vec<String> = parts
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        for seg in &path_segments {
            if !is_valid_path_segment(seg) {
                return Err(DidError::InvalidDidKey);
            }
        }
        Ok(Self {
            domain,
            path_segments,
        })
    }

    /// 정규 did 문자열로 재조립.
    pub fn to_did_string(&self) -> String {
        if self.path_segments.is_empty() {
            format!("did:wba:{}", self.domain)
        } else {
            format!("did:wba:{}:{}", self.domain, self.path_segments.join(":"))
        }
    }

    /// resolver URL 계산.
    ///
    /// - segments 비어있으면 `https://{domain}/.well-known/did.json`
    /// - segments 존재하면 `https://{domain}/{seg1}/{seg2}/.../did.json`
    pub fn resolve_url(&self) -> String {
        if self.path_segments.is_empty() {
            format!("https://{}/.well-known/did.json", self.domain)
        } else {
            format!(
                "https://{}/{}/did.json",
                self.domain,
                self.path_segments.join("/")
            )
        }
    }
}

/// 도메인 문자열의 매우 느슨한 검증. (`/`, 공백, 제어문자 등은 금지)
fn is_valid_domain(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && !s.starts_with('.')
        && !s.ends_with('.')
}

/// path segment 의 매우 느슨한 검증. RFC 3986 의 `pchar` 부분집합.
fn is_valid_path_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
}

/// 해석된 DID Document 의 정합성 검증.
///
/// - `id` 필드가 입력 did 와 일치하는지
/// - `verificationMethod` 가 비어있지 않은지
/// - 각 verificationMethod 의 `controller` 가 did 와 일치하는지 (있는 경우)
pub fn validate_did_document(did: &str, doc: &Value) -> Result<(), DidError> {
    let obj = doc.as_object().ok_or(DidError::VcMissingField("root"))?;
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or(DidError::VcMissingField("id"))?;
    if id != did {
        return Err(DidError::InvalidDidKey);
    }
    let vm = obj
        .get("verificationMethod")
        .and_then(|v| v.as_array())
        .ok_or(DidError::VcMissingField("verificationMethod"))?;
    if vm.is_empty() {
        return Err(DidError::VcMissingField("verificationMethod"));
    }
    for entry in vm {
        // id 필수, type 필수
        entry
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or(DidError::VcMissingField("verificationMethod.id"))?;
        entry
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or(DidError::VcMissingField("verificationMethod.type"))?;
        // controller 가 있는 경우 did 와 일치해야 함
        if let Some(controller) = entry.get("controller").and_then(|v| v.as_str()) {
            if controller != did {
                return Err(DidError::InvalidDidKey);
            }
        }
    }
    Ok(())
}

/// DID Document 에서 첫 번째 (또는 type 지정된) service endpoint 추출.
///
/// `service_type` = None 이면 첫 service entry 의 `serviceEndpoint` 반환.
pub fn extract_service_endpoint(doc: &Value, service_type: Option<&str>) -> Option<String> {
    let services = doc.get("service")?.as_array()?;
    for s in services {
        if let Some(want) = service_type {
            if s.get("type").and_then(|v| v.as_str()) != Some(want) {
                continue;
            }
        }
        if let Some(endpoint) = s.get("serviceEndpoint").and_then(|v| v.as_str()) {
            return Some(endpoint.to_string());
        }
    }
    None
}

/// DID Document 에서 첫 verificationMethod 의 publicKeyMultibase 추출 후
/// secp256k1 압축 33바이트 공개키 복원.
///
/// 다른 키 타입(Ed25519 등) 은 `None` 반환.
pub fn extract_secp256k1_pubkey(doc: &Value) -> Option<Vec<u8>> {
    let vm = doc.get("verificationMethod")?.as_array()?;
    for entry in vm {
        let key_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if key_type != "EcdsaSecp256k1VerificationKey2019" {
            continue;
        }
        let multibase = entry.get("publicKeyMultibase").and_then(|v| v.as_str())?;
        let rest = multibase.strip_prefix('z')?;
        let raw = bs58::decode(rest).into_vec().ok()?;
        if raw.len() == 35 && raw[0] == SECP256K1_PUB_VARINT[0] && raw[1] == SECP256K1_PUB_VARINT[1]
        {
            return Some(raw[2..].to_vec());
        }
    }
    None
}

/// master keypair 로부터 did:wba DID Document 생성 (publish 용).
///
/// `did` 는 호출자가 결정한 `did:wba:domain[:path]` 문자열.
/// service endpoint 는 옵션 — `(type, endpoint_url)`.
pub fn generate_did_document(
    did: &str,
    master: &Keypair,
    service: Option<(&str, &str)>,
) -> Result<Value, DidError> {
    // did 형식 검증 (구조 파싱만)
    let _ = WbaDid::parse(did)?;

    let pk = master.public_key_bytes();
    if pk.len() != 33 {
        return Err(DidError::InvalidPubkeyLength(pk.len()));
    }
    let mut buf = Vec::with_capacity(35);
    buf.extend_from_slice(&SECP256K1_PUB_VARINT);
    buf.extend_from_slice(&pk);
    let pubkey_multibase = format!("z{}", bs58::encode(&buf).into_string());
    let key_id = format!("{did}#key-1");
    let mut doc = json!({
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
        "authentication": [key_id.clone()],
        "assertionMethod": [key_id],
    });
    if let Some((stype, endpoint)) = service {
        let service_id = format!("{did}#service-1");
        doc.as_object_mut().unwrap().insert(
            "service".into(),
            json!([{
                "id": service_id,
                "type": stype,
                "serviceEndpoint": endpoint,
            }]),
        );
    }
    Ok(doc)
}

/// 정규 JSON canonicalization 후 SHA-256. did:wba document fingerprint 용.
pub fn document_fingerprint(doc: &Value) -> String {
    let canonical = canonical_json(doc);
    let digest = Sha256::digest(canonical.as_bytes());
    hex::encode(digest)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_domain_only() {
        let did = WbaDid::parse("did:wba:example.com").unwrap();
        assert_eq!(did.domain, "example.com");
        assert!(did.path_segments.is_empty());
        assert_eq!(
            did.resolve_url(),
            "https://example.com/.well-known/did.json"
        );
        assert_eq!(did.to_did_string(), "did:wba:example.com");
    }

    #[test]
    fn parse_single_path() {
        let did = WbaDid::parse("did:wba:example.com:alice").unwrap();
        assert_eq!(did.domain, "example.com");
        assert_eq!(did.path_segments, vec!["alice".to_string()]);
        assert_eq!(did.resolve_url(), "https://example.com/alice/did.json");
    }

    #[test]
    fn parse_multi_path() {
        let did = WbaDid::parse("did:wba:example.com:agents:alice").unwrap();
        assert_eq!(did.domain, "example.com");
        assert_eq!(
            did.path_segments,
            vec!["agents".to_string(), "alice".to_string()]
        );
        assert_eq!(
            did.resolve_url(),
            "https://example.com/agents/alice/did.json"
        );
        assert_eq!(did.to_did_string(), "did:wba:example.com:agents:alice");
    }

    #[test]
    fn parse_rejects_non_wba() {
        assert!(WbaDid::parse("did:key:zABC").is_err());
        assert!(WbaDid::parse("did:wba:").is_err());
        assert!(WbaDid::parse("").is_err());
    }

    #[test]
    fn parse_rejects_bad_domain() {
        assert!(WbaDid::parse("did:wba:bad domain.com").is_err());
        assert!(WbaDid::parse("did:wba:.example.com").is_err());
        assert!(WbaDid::parse("did:wba:example.com.").is_err());
    }

    #[test]
    fn parse_rejects_bad_segment() {
        assert!(WbaDid::parse("did:wba:example.com:bad/path").is_err());
        assert!(WbaDid::parse("did:wba:example.com:bad path").is_err());
    }

    #[test]
    fn generate_doc_round_trip() {
        let kp = Keypair::from_secret_bytes(&[0x77u8; 32]).unwrap();
        let did = "did:wba:example.com:alice";
        let doc = generate_did_document(did, &kp, None).unwrap();
        validate_did_document(did, &doc).unwrap();
        let recovered = extract_secp256k1_pubkey(&doc).expect("pubkey");
        assert_eq!(recovered, kp.public_key_bytes());
    }

    #[test]
    fn generate_doc_with_service() {
        let kp = Keypair::from_secret_bytes(&[0x88u8; 32]).unwrap();
        let did = "did:wba:example.com";
        let doc =
            generate_did_document(did, &kp, Some(("AnpAgent", "https://example.com/anp"))).unwrap();
        validate_did_document(did, &doc).unwrap();
        let endpoint = extract_service_endpoint(&doc, Some("AnpAgent"));
        assert_eq!(endpoint.as_deref(), Some("https://example.com/anp"));
        let any_endpoint = extract_service_endpoint(&doc, None);
        assert!(any_endpoint.is_some());
    }

    #[test]
    fn validate_rejects_id_mismatch() {
        let kp = Keypair::from_secret_bytes(&[0x99u8; 32]).unwrap();
        let doc = generate_did_document("did:wba:example.com:alice", &kp, None).unwrap();
        assert!(validate_did_document("did:wba:example.com:bob", &doc).is_err());
    }

    #[test]
    fn validate_rejects_empty_verification_method() {
        let doc = json!({
            "id": "did:wba:example.com",
            "verificationMethod": []
        });
        assert!(validate_did_document("did:wba:example.com", &doc).is_err());
    }

    #[test]
    fn validate_rejects_controller_mismatch() {
        let doc = json!({
            "id": "did:wba:example.com:alice",
            "verificationMethod": [{
                "id": "did:wba:example.com:alice#key-1",
                "type": "EcdsaSecp256k1VerificationKey2019",
                "controller": "did:wba:other.com",
                "publicKeyMultibase": "zABC"
            }]
        });
        assert!(validate_did_document("did:wba:example.com:alice", &doc).is_err());
    }

    #[test]
    fn fingerprint_deterministic() {
        let kp = Keypair::from_secret_bytes(&[0xAAu8; 32]).unwrap();
        let doc1 = generate_did_document("did:wba:example.com", &kp, None).unwrap();
        let doc2 = generate_did_document("did:wba:example.com", &kp, None).unwrap();
        assert_eq!(document_fingerprint(&doc1), document_fingerprint(&doc2));
    }
}
