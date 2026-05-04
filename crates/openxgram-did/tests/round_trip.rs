//! 통합 테스트 — DID Document JSON-LD 필드 + VC 라운드트립.

use openxgram_did::{did_document, did_key_from_master, issue_vc, omnione_format, opendid_kr_format, verify_vc};
use openxgram_keystore::Keypair;
use serde_json::json;

#[test]
fn full_identity_pipeline() {
    let kp = Keypair::from_secret_bytes(&[0xAAu8; 32]).unwrap();

    let did = did_key_from_master(&kp).unwrap();
    assert!(did.starts_with("did:key:z"));

    let doc = did_document(&did, &kp).unwrap();
    let ctx = doc["@context"].as_array().unwrap();
    assert!(ctx.iter().any(|v| v == "https://www.w3.org/ns/did/v1"));
    assert_eq!(doc["id"].as_str().unwrap(), did);
    assert!(!doc["verificationMethod"].as_array().unwrap().is_empty());
    assert!(!doc["authentication"].as_array().unwrap().is_empty());

    let kr = opendid_kr_format(&kp, "testnet").unwrap();
    assert!(kr.starts_with("did:opendid:testnet:"));

    let omn = omnione_format(&kp).unwrap();
    assert!(omn.starts_with("did:omn:"));

    let subject = "did:key:zQ3shokFTS3brHcDQrn82RUDfCZESWL1ZdCEJwekUDPQiYBme";
    let vc = issue_vc(&did, subject, json!({"role": "agent", "tier": "L4"}), &kp).unwrap();
    assert_eq!(vc["issuer"], did);
    assert_eq!(vc["credentialSubject"]["id"], subject);
    assert!(vc["proof"]["jws"].is_string());

    let pk = kp.public_key_bytes();
    assert!(verify_vc(&vc, &pk).unwrap());
}
