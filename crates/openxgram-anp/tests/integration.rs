//! ANP integration tests — did:wba resolve + sign + post + verify 풀 흐름.

use openxgram_anp::client::send_with_resolver_url;
use openxgram_anp::message::AnpEnvelope;
use openxgram_anp::AnpClient;
use openxgram_did::wba::{generate_did_document, WbaDid};
use openxgram_keystore::Keypair;
use serde_json::json;

#[tokio::test]
async fn full_flow_discovery_send_verify() {
    let mut server = mockito::Server::new_async().await;

    // Bob 의 정체성 + DID document publish (well-known).
    let bob_kp = Keypair::from_secret_bytes(&[0xB0u8; 32]).unwrap();
    let bob_did = "did:wba:bob.com";
    let endpoint = format!("{}/anp", server.url());
    let bob_doc = generate_did_document(bob_did, &bob_kp, Some(("AnpAgent", &endpoint))).unwrap();

    let _resolver = server
        .mock("GET", "/.well-known/did.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&bob_doc).unwrap())
        .create_async()
        .await;

    // Bob 의 ANP endpoint: 받은 envelope 의 from_did echo + 새 envelope 응답 (Bob 키로 서명).
    // mockito 는 동적 응답이 제한적이므로 단순히 ack envelope 한 개 미리 빌드해 반환.
    let mut ack = AnpEnvelope::new(bob_did, "did:wba:alice.com", "ack", json!({"ok": true}));
    ack.sign(&bob_kp);

    let _ep = server
        .mock("POST", "/anp")
        .with_status(200)
        .with_header("content-type", "application/anp-envelope+json")
        .with_body(ack.to_json().unwrap())
        .create_async()
        .await;

    // Alice 가 Bob 에게 메시지 전송.
    let alice_kp = Keypair::from_secret_bytes(&[0xA0u8; 32]).unwrap();
    let client = AnpClient::new();
    let resolver_url = format!("{}/.well-known/did.json", server.url());
    let result = send_with_resolver_url(
        &client,
        "did:wba:alice.com",
        bob_did,
        "task.request",
        json!({"task": "hello bob"}),
        &alice_kp,
        &resolver_url,
    )
    .await
    .unwrap();

    assert_eq!(result.status, 200);

    // 발신 envelope 은 Alice 키로 검증되어야 한다.
    assert!(result
        .sent
        .verify_with_pubkey(&alice_kp.public_key_bytes())
        .unwrap());

    // 응답 envelope 은 Bob 키로 검증되어야 한다.
    let resp = result.response_envelope.expect("response envelope");
    assert!(resp.verify_with_pubkey(&bob_kp.public_key_bytes()).unwrap());
    assert_eq!(resp.header.from_did, bob_did);
    assert_eq!(resp.header.msg_type, "ack");
}

#[tokio::test]
async fn discover_endpoint_missing_service() {
    let mut server = mockito::Server::new_async().await;
    let kp = Keypair::from_secret_bytes(&[0xCCu8; 32]).unwrap();
    let did = "did:wba:no-service.com";
    let doc = generate_did_document(did, &kp, None).unwrap(); // no service
    let _m = server
        .mock("GET", "/.well-known/did.json")
        .with_status(200)
        .with_body(serde_json::to_string(&doc).unwrap())
        .create_async()
        .await;

    let resolver_url = format!("{}/.well-known/did.json", server.url());
    let client = AnpClient::new();
    let res = send_with_resolver_url(
        &client,
        "did:wba:alice",
        did,
        "ping",
        json!({}),
        &kp,
        &resolver_url,
    )
    .await;
    assert!(res.is_err());
}

#[test]
fn wba_parser_round_trip() {
    let cases = [
        "did:wba:example.com",
        "did:wba:example.com:alice",
        "did:wba:example.com:agents:alice",
    ];
    for c in cases {
        let parsed = WbaDid::parse(c).unwrap();
        assert_eq!(parsed.to_did_string(), c);
    }
}
