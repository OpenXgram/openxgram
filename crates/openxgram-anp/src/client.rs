//! ANP HTTP 클라이언트.
//!
//! 흐름:
//! 1. 발신자 did + 수신자 did + body 로 envelope 빌드
//! 2. 발신자 keypair 로 envelope 서명
//! 3. discovery 로 수신자의 service endpoint 해석
//! 4. POST endpoint, content-type: application/anp-envelope+json
//! 5. 응답을 envelope 으로 파싱하고 수신자 키로 검증
//!
//! discovery 결과는 호출자가 별도로 캐시할 수 있으므로 client 자체는 stateless.

use openxgram_keystore::Keypair;
use serde_json::Value;

use crate::discovery::{discover_endpoint, discover_endpoint_at_url, DiscoveryResult};
use crate::message::AnpEnvelope;
use crate::{AnpError, Result, ANP_CONTENT_TYPE};

/// ANP HTTP 클라이언트. reqwest::Client 를 wrapping.
pub struct AnpClient {
    http: reqwest::Client,
}

impl Default for AnpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AnpClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    pub fn with_http(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// 내부 reqwest 클라이언트 노출 — discovery 직접 호출 시 사용.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// 메시지 전송.
    ///
    /// `discovery` 가 None 이면 to_did 를 새로 해석.
    pub async fn send_message(
        &self,
        from_did: &str,
        to_did: &str,
        msg_type: &str,
        body: Value,
        from_keypair: &Keypair,
        discovery: Option<&DiscoveryResult>,
    ) -> Result<SendResult> {
        let owned_discovery: DiscoveryResult;
        let discovery = match discovery {
            Some(d) => d,
            None => {
                owned_discovery = discover_endpoint(&self.http, to_did).await?;
                &owned_discovery
            }
        };
        let endpoint = discovery
            .agent_endpoint
            .as_deref()
            .ok_or_else(|| AnpError::NoServiceEndpoint(crate::ANP_AGENT_SERVICE_TYPE.into()))?;
        self.send_to_endpoint(from_did, to_did, msg_type, body, from_keypair, endpoint)
            .await
    }

    /// endpoint URL 직접 지정 버전 (테스트·캐시된 endpoint 재사용용).
    pub async fn send_to_endpoint(
        &self,
        from_did: &str,
        to_did: &str,
        msg_type: &str,
        body: Value,
        from_keypair: &Keypair,
        endpoint: &str,
    ) -> Result<SendResult> {
        let mut env = AnpEnvelope::new(from_did, to_did, msg_type, body);
        env.sign(from_keypair);
        let payload = env.to_json()?;

        tracing::debug!(endpoint, from_did, to_did, msg_type, "anp.send_message");

        let resp = self
            .http
            .post(endpoint)
            .header(reqwest::header::CONTENT_TYPE, ANP_CONTENT_TYPE)
            .header("X-Anp-From", from_did)
            .header("X-Anp-To", to_did)
            .body(payload)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(AnpError::Http(format!(
                "POST {endpoint} → {status}: {text}"
            )));
        }
        // 응답이 envelope 일 수도 있고 plain JSON 일 수도 있음 — 둘 다 허용.
        let response_value: Value = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text)?
        };
        let response_envelope = AnpEnvelope::from_json(&text).ok();
        Ok(SendResult {
            status: status.as_u16(),
            sent: env,
            response_envelope,
            response_value,
        })
    }
}

/// `send_message` 결과.
#[derive(Debug)]
pub struct SendResult {
    pub status: u16,
    pub sent: AnpEnvelope,
    /// 응답이 ANP envelope 으로 파싱되면 채워짐.
    pub response_envelope: Option<AnpEnvelope>,
    /// raw JSON 응답 (envelope 이 아니어도 보존).
    pub response_value: Value,
}

/// `AnpClient` 가 mockito 의 base URL 로 discovery + send 둘 다 하는 헬퍼.
/// 통합 테스트에서 사용.
pub async fn send_with_resolver_url(
    client: &AnpClient,
    from_did: &str,
    to_did: &str,
    msg_type: &str,
    body: Value,
    from_keypair: &Keypair,
    resolver_url: &str,
) -> Result<SendResult> {
    let disc = discover_endpoint_at_url(client.http(), to_did, resolver_url).await?;
    let endpoint = disc
        .agent_endpoint
        .as_deref()
        .ok_or_else(|| AnpError::NoServiceEndpoint(crate::ANP_AGENT_SERVICE_TYPE.into()))?;
    client
        .send_to_endpoint(from_did, to_did, msg_type, body, from_keypair, endpoint)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_did::wba::generate_did_document;
    use openxgram_keystore::Keypair;
    use serde_json::json;

    #[tokio::test]
    async fn send_to_endpoint_signs_and_posts() {
        let mut server = mockito::Server::new_async().await;
        let from_kp = Keypair::from_secret_bytes(&[0xAAu8; 32]).unwrap();

        let _m = server
            .mock("POST", "/anp")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"ok":true}"#)
            .create_async()
            .await;

        let endpoint = format!("{}/anp", server.url());
        let client = AnpClient::new();
        let result = client
            .send_to_endpoint(
                "did:wba:alice.com",
                "did:wba:bob.com",
                "ping",
                json!({"hello": "world"}),
                &from_kp,
                &endpoint,
            )
            .await
            .unwrap();
        assert_eq!(result.status, 200);
        assert!(result.sent.header.signature.is_some());
        // 발신자 키로 sent envelope 검증 가능해야 함.
        assert!(result
            .sent
            .verify_with_pubkey(&from_kp.public_key_bytes())
            .unwrap());
        assert_eq!(result.response_value, json!({"ok": true}));
    }

    #[tokio::test]
    async fn send_full_flow_with_resolver_url() {
        let mut server = mockito::Server::new_async().await;
        let from_kp = Keypair::from_secret_bytes(&[0xBBu8; 32]).unwrap();
        let to_kp = Keypair::from_secret_bytes(&[0xCCu8; 32]).unwrap();

        let to_did = "did:wba:bob.com";
        let endpoint = format!("{}/anp", server.url());
        let doc = generate_did_document(to_did, &to_kp, Some(("AnpAgent", &endpoint))).unwrap();

        let _resolver = server
            .mock("GET", "/.well-known/did.json")
            .with_status(200)
            .with_body(serde_json::to_string(&doc).unwrap())
            .create_async()
            .await;
        let _posted = server
            .mock("POST", "/anp")
            .with_status(202)
            .with_body("{}")
            .create_async()
            .await;

        let resolver_url = format!("{}/.well-known/did.json", server.url());
        let client = AnpClient::new();
        let res = send_with_resolver_url(
            &client,
            "did:wba:alice.com",
            to_did,
            "task.request",
            json!({"n": 1}),
            &from_kp,
            &resolver_url,
        )
        .await
        .unwrap();
        assert_eq!(res.status, 202);
        assert!(res.sent.header.signature.is_some());
    }

    #[tokio::test]
    async fn send_propagates_http_error() {
        let mut server = mockito::Server::new_async().await;
        let from_kp = Keypair::from_secret_bytes(&[0xDDu8; 32]).unwrap();
        let _m = server
            .mock("POST", "/anp")
            .with_status(500)
            .with_body("boom")
            .create_async()
            .await;
        let endpoint = format!("{}/anp", server.url());
        let client = AnpClient::new();
        let res = client
            .send_to_endpoint(
                "did:wba:a",
                "did:wba:b",
                "ping",
                json!({}),
                &from_kp,
                &endpoint,
            )
            .await;
        assert!(matches!(res, Err(AnpError::Http(_))));
    }
}
