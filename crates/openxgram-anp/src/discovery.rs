//! ANP discovery — did:wba 해석 + service endpoint 추출.
//!
//! 흐름:
//! 1. did:wba 문자열 파싱 → resolver URL 계산
//! 2. HTTPS GET → DID Document JSON
//! 3. document.id 가 입력 did 와 일치 확인
//! 4. service[type=AnpAgent].serviceEndpoint 추출

use openxgram_did::wba::{
    extract_secp256k1_pubkey, extract_service_endpoint, validate_did_document, WbaDid,
};
use serde_json::Value;

use crate::{AnpError, Result, ANP_AGENT_SERVICE_TYPE};

/// did:wba 해석 결과.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    pub did: String,
    pub document: Value,
    /// service.type == "AnpAgent" 의 serviceEndpoint URL.
    pub agent_endpoint: Option<String>,
    /// verificationMethod 첫 secp256k1 키 (compressed 33B).
    pub public_key: Option<Vec<u8>>,
}

/// did:wba 문자열을 HTTPS 로 해석해 DID Document 반환 + 검증.
///
/// 호출자가 reqwest::Client 를 주입 (테스트 시 mockito base URL 가능).
/// async — 호출자는 tokio runtime 필요.
pub async fn resolve_did_document(client: &reqwest::Client, did: &str) -> Result<Value> {
    let parsed = WbaDid::parse(did)?;
    let url = parsed.resolve_url();
    resolve_did_document_at_url(client, did, &url).await
}

/// resolver URL 을 직접 지정 (mockito 등 테스트용 base URL override 가능).
pub async fn resolve_did_document_at_url(
    client: &reqwest::Client,
    did: &str,
    url: &str,
) -> Result<Value> {
    tracing::debug!(did, url, "anp.resolve_did_document");
    let resp = client.get(url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AnpError::Resolution(format!(
            "GET {url} returned {status}"
        )));
    }
    let doc: Value = resp.json().await?;
    validate_did_document(did, &doc).map_err(|e| AnpError::InvalidDocument(e.to_string()))?;
    Ok(doc)
}

/// did:wba 풀-디스커버리 — 문서 + 서명키 + service endpoint 한 번에.
pub async fn discover_endpoint(client: &reqwest::Client, did: &str) -> Result<DiscoveryResult> {
    let doc = resolve_did_document(client, did).await?;
    let agent_endpoint = extract_service_endpoint(&doc, Some(ANP_AGENT_SERVICE_TYPE));
    let public_key = extract_secp256k1_pubkey(&doc);
    Ok(DiscoveryResult {
        did: did.to_string(),
        document: doc,
        agent_endpoint,
        public_key,
    })
}

/// resolver URL override 버전 (mockito 등 테스트용).
pub async fn discover_endpoint_at_url(
    client: &reqwest::Client,
    did: &str,
    url: &str,
) -> Result<DiscoveryResult> {
    let doc = resolve_did_document_at_url(client, did, url).await?;
    let agent_endpoint = extract_service_endpoint(&doc, Some(ANP_AGENT_SERVICE_TYPE));
    let public_key = extract_secp256k1_pubkey(&doc);
    Ok(DiscoveryResult {
        did: did.to_string(),
        document: doc,
        agent_endpoint,
        public_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_did::wba::generate_did_document;
    use openxgram_keystore::Keypair;

    #[tokio::test]
    async fn resolve_via_mockito_well_known() {
        let mut server = mockito::Server::new_async().await;
        let kp = Keypair::from_secret_bytes(&[0x11u8; 32]).unwrap();
        let did = "did:wba:example.com";
        let doc = generate_did_document(did, &kp, Some(("AnpAgent", "https://example.com/anp")))
            .unwrap();
        let body = serde_json::to_string(&doc).unwrap();
        let _m = server
            .mock("GET", "/.well-known/did.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let url = format!("{}/.well-known/did.json", server.url());
        let client = reqwest::Client::new();
        let res = discover_endpoint_at_url(&client, did, &url).await.unwrap();
        assert_eq!(res.did, did);
        assert_eq!(
            res.agent_endpoint.as_deref(),
            Some("https://example.com/anp")
        );
        let pk = res.public_key.expect("pubkey present");
        assert_eq!(pk, kp.public_key_bytes());
    }

    #[tokio::test]
    async fn resolve_path_segment() {
        let mut server = mockito::Server::new_async().await;
        let kp = Keypair::from_secret_bytes(&[0x22u8; 32]).unwrap();
        let did = "did:wba:example.com:alice";
        let doc = generate_did_document(did, &kp, None).unwrap();
        let body = serde_json::to_string(&doc).unwrap();
        let _m = server
            .mock("GET", "/alice/did.json")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;

        let url = format!("{}/alice/did.json", server.url());
        let client = reqwest::Client::new();
        let res = discover_endpoint_at_url(&client, did, &url).await.unwrap();
        assert_eq!(res.did, did);
        assert!(res.agent_endpoint.is_none()); // no service field
    }

    #[tokio::test]
    async fn resolve_rejects_id_mismatch() {
        let mut server = mockito::Server::new_async().await;
        let kp = Keypair::from_secret_bytes(&[0x33u8; 32]).unwrap();
        let other_did = "did:wba:other.com";
        let doc = generate_did_document(other_did, &kp, None).unwrap();
        let _m = server
            .mock("GET", "/.well-known/did.json")
            .with_status(200)
            .with_body(serde_json::to_string(&doc).unwrap())
            .create_async()
            .await;
        let url = format!("{}/.well-known/did.json", server.url());
        let client = reqwest::Client::new();
        let res = discover_endpoint_at_url(&client, "did:wba:example.com", &url).await;
        assert!(matches!(res, Err(AnpError::InvalidDocument(_))));
    }

    #[tokio::test]
    async fn resolve_http_error_propagates() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/.well-known/did.json")
            .with_status(404)
            .create_async()
            .await;
        let url = format!("{}/.well-known/did.json", server.url());
        let client = reqwest::Client::new();
        let res =
            discover_endpoint_at_url(&client, "did:wba:example.com", &url).await;
        assert!(matches!(res, Err(AnpError::Resolution(_))));
    }
}
