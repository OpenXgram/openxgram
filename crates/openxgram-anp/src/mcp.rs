//! MCP 도구 4개 — ANP 어댑터.
//!
//! - `anp_resolve_did(did)` → DID document JSON
//! - `anp_send_message({to_did, type, body, from_did, endpoint?})` → 응답 envelope/JSON
//! - `anp_verify_signature({did?, public_key_hex?, envelope})` → bool
//! - `anp_announce_self({did, service_endpoint?, service_type?})` → publish-ready DID document (stub)
//!
//! 이 모듈은 `openxgram-mcp` 의 `ToolDispatcher` trait 와 무관하게 자체 dispatch 한다.
//! cli 의 mcp_serve 레벨에서 trait adapter 추가 가능 (현 PR 범위 밖).
//!
//! 모든 도구는 async — tokio runtime 위에서 실행.

use openxgram_did::wba::generate_did_document;
use openxgram_keystore::Keypair;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::client::AnpClient;
use crate::discovery::{discover_endpoint, resolve_did_document};
use crate::message::AnpEnvelope;
use crate::AnpError;

#[derive(Debug, Error)]
pub enum AnpToolError {
    #[error("invalid params: {0}")]
    InvalidParams(String),

    #[error("anp error: {0}")]
    Anp(#[from] AnpError),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("envelope error: {0}")]
    Envelope(#[from] crate::message::EnvelopeError),

    #[error("did error: {0}")]
    Did(#[from] openxgram_did::DidError),

    #[error("keystore error: {0}")]
    Keystore(String),
}

/// MCP 도구 이름 상수.
pub const TOOL_RESOLVE_DID: &str = "anp_resolve_did";
pub const TOOL_SEND_MESSAGE: &str = "anp_send_message";
pub const TOOL_VERIFY_SIGNATURE: &str = "anp_verify_signature";
pub const TOOL_ANNOUNCE_SELF: &str = "anp_announce_self";

/// 4개 도구의 dispatcher.
///
/// `self_keypair` 는 announce/send 시 자기 서명에 사용. None 이면 announce/send 거부.
pub struct AnpTools {
    pub client: AnpClient,
    pub self_did: Option<String>,
    pub self_keypair: Option<Keypair>,
}

impl Default for AnpTools {
    fn default() -> Self {
        Self {
            client: AnpClient::new(),
            self_did: None,
            self_keypair: None,
        }
    }
}

impl AnpTools {
    pub fn new() -> Self {
        Self::default()
    }

    /// self DID + keypair 주입.
    pub fn with_identity(mut self, did: impl Into<String>, kp: Keypair) -> Self {
        self.self_did = Some(did.into());
        self.self_keypair = Some(kp);
        self
    }

    /// 모든 도구 스펙 (MCP tools/list 응답용).
    pub fn tool_specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                name: TOOL_RESOLVE_DID.into(),
                description: "Resolve a did:wba identifier into its DID document via HTTPS.".into(),
                input_schema: json!({
                    "type": "object",
                    "required": ["did"],
                    "properties": {
                        "did": { "type": "string", "description": "did:wba:domain[:path]" }
                    }
                }),
            },
            ToolSpec {
                name: TOOL_SEND_MESSAGE.into(),
                description: "Send a signed ANP envelope to a remote did:wba agent.".into(),
                input_schema: json!({
                    "type": "object",
                    "required": ["to_did", "type", "body"],
                    "properties": {
                        "to_did":    { "type": "string" },
                        "type":      { "type": "string" },
                        "body":      { "type": "object" },
                        "from_did":  { "type": "string", "description": "override self.did" },
                        "endpoint":  { "type": "string", "description": "override resolved endpoint" }
                    }
                }),
            },
            ToolSpec {
                name: TOOL_VERIFY_SIGNATURE.into(),
                description: "Verify an ANP envelope's signature. Provide either did (resolves pubkey) or public_key_hex.".into(),
                input_schema: json!({
                    "type": "object",
                    "required": ["envelope"],
                    "properties": {
                        "envelope":       { "type": "object" },
                        "did":            { "type": "string" },
                        "public_key_hex": { "type": "string" }
                    }
                }),
            },
            ToolSpec {
                name: TOOL_ANNOUNCE_SELF.into(),
                description: "Generate a publishable did:wba DID document for self. STRETCH: actual publishing is host-specific (FTP/HTTP PUT/git push), this tool returns the document only.".into(),
                input_schema: json!({
                    "type": "object",
                    "required": ["did"],
                    "properties": {
                        "did":              { "type": "string", "description": "did:wba:domain[:path] to publish" },
                        "service_endpoint": { "type": "string" },
                        "service_type":     { "type": "string", "description": "default: AnpAgent" }
                    }
                }),
            },
        ]
    }

    /// 단일 도구 호출 dispatch.
    pub async fn call(&self, name: &str, args: &Value) -> Result<Value, AnpToolError> {
        match name {
            TOOL_RESOLVE_DID => self.tool_resolve_did(args).await,
            TOOL_SEND_MESSAGE => self.tool_send_message(args).await,
            TOOL_VERIFY_SIGNATURE => self.tool_verify_signature(args).await,
            TOOL_ANNOUNCE_SELF => self.tool_announce_self(args).await,
            other => Err(AnpToolError::InvalidParams(format!(
                "unknown tool: {other}"
            ))),
        }
    }

    async fn tool_resolve_did(&self, args: &Value) -> Result<Value, AnpToolError> {
        let did = args
            .get("did")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AnpToolError::InvalidParams("did required".into()))?;
        let doc = resolve_did_document(self.client.http(), did).await?;
        Ok(json!({
            "did": did,
            "document": doc,
        }))
    }

    async fn tool_send_message(&self, args: &Value) -> Result<Value, AnpToolError> {
        let to_did = args
            .get("to_did")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AnpToolError::InvalidParams("to_did required".into()))?;
        let msg_type = args
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AnpToolError::InvalidParams("type required".into()))?;
        let body = args
            .get("body")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        let from_did = args
            .get("from_did")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.self_did.clone())
            .ok_or_else(|| AnpToolError::InvalidParams("from_did or self.did required".into()))?;
        let kp = self.self_keypair.as_ref().ok_or_else(|| {
            AnpToolError::InvalidParams("self keypair not configured; cannot sign".into())
        })?;

        let result = if let Some(endpoint) = args.get("endpoint").and_then(|v| v.as_str()) {
            self.client
                .send_to_endpoint(&from_did, to_did, msg_type, body, kp, endpoint)
                .await?
        } else {
            self.client
                .send_message(&from_did, to_did, msg_type, body, kp, None)
                .await?
        };
        Ok(json!({
            "status": result.status,
            "sent": serde_json::to_value(&result.sent)?,
            "response": result.response_value,
        }))
    }

    async fn tool_verify_signature(&self, args: &Value) -> Result<Value, AnpToolError> {
        let env_val = args
            .get("envelope")
            .ok_or_else(|| AnpToolError::InvalidParams("envelope required".into()))?
            .clone();
        let env: AnpEnvelope = serde_json::from_value(env_val)?;
        let pubkey_hex = if let Some(pk) = args.get("public_key_hex").and_then(|v| v.as_str()) {
            pk.to_string()
        } else if let Some(did) = args.get("did").and_then(|v| v.as_str()) {
            let disc = discover_endpoint(self.client.http(), did).await?;
            let pk = disc.public_key.ok_or_else(|| {
                AnpToolError::InvalidParams(format!(
                    "did {did} document has no secp256k1 verificationMethod"
                ))
            })?;
            hex::encode(pk)
        } else {
            return Err(AnpToolError::InvalidParams(
                "either did or public_key_hex required".into(),
            ));
        };
        let ok = env.verify_with_pubkey_hex(&pubkey_hex)?;
        Ok(json!({
            "verified": ok,
            "from_did": env.header.from_did,
            "to_did":   env.header.to_did,
            "type":     env.header.msg_type,
        }))
    }

    async fn tool_announce_self(&self, args: &Value) -> Result<Value, AnpToolError> {
        let did = args
            .get("did")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AnpToolError::InvalidParams("did required".into()))?;
        let service_type = args
            .get("service_type")
            .and_then(|v| v.as_str())
            .unwrap_or(crate::ANP_AGENT_SERVICE_TYPE);
        let service_endpoint = args.get("service_endpoint").and_then(|v| v.as_str());
        let kp = self.self_keypair.as_ref().ok_or_else(|| {
            AnpToolError::InvalidParams("self keypair not configured; cannot generate doc".into())
        })?;
        let service = service_endpoint.map(|ep| (service_type, ep));
        let doc = generate_did_document(did, kp, service)?;
        // STRETCH: 실제 publish 는 호스트 환경별 (HTTP PUT, git push, FTP, S3 등).
        // 현 도구는 publish-ready document 만 반환.
        Ok(json!({
            "did": did,
            "document": doc,
            "published": false,
            "note": "publish step is host-specific; deliver document JSON to https://{domain}/.well-known/did.json or /{path}/did.json out-of-band"
        }))
    }
}

/// MCP tool spec (openxgram-mcp 와 무관한 자체 형식).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_keystore::Keypair;
    use serde_json::json;

    #[test]
    fn tool_specs_have_four() {
        let specs = AnpTools::tool_specs();
        assert_eq!(specs.len(), 4);
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&TOOL_RESOLVE_DID));
        assert!(names.contains(&TOOL_SEND_MESSAGE));
        assert!(names.contains(&TOOL_VERIFY_SIGNATURE));
        assert!(names.contains(&TOOL_ANNOUNCE_SELF));
    }

    #[tokio::test]
    async fn announce_self_returns_document() {
        let kp = Keypair::from_secret_bytes(&[0x10u8; 32]).unwrap();
        let tools = AnpTools::new().with_identity("did:wba:example.com", kp);
        let res = tools
            .call(
                TOOL_ANNOUNCE_SELF,
                &json!({
                    "did": "did:wba:example.com",
                    "service_endpoint": "https://example.com/anp"
                }),
            )
            .await
            .unwrap();
        assert_eq!(res["did"], "did:wba:example.com");
        assert_eq!(res["published"], false);
        assert!(res["document"]["verificationMethod"].is_array());
        assert_eq!(
            res["document"]["service"][0]["serviceEndpoint"],
            "https://example.com/anp"
        );
    }

    #[tokio::test]
    async fn announce_self_requires_keypair() {
        let tools = AnpTools::new(); // no identity
        let res = tools
            .call(TOOL_ANNOUNCE_SELF, &json!({"did":"did:wba:example.com"}))
            .await;
        assert!(matches!(res, Err(AnpToolError::InvalidParams(_))));
    }

    #[tokio::test]
    async fn verify_signature_with_public_key_hex() {
        let kp = Keypair::from_secret_bytes(&[0x20u8; 32]).unwrap();
        let mut env = AnpEnvelope::new("did:wba:a", "did:wba:b", "ping", json!({"x": 1}));
        env.sign(&kp);
        let pk_hex = hex::encode(kp.public_key_bytes());

        let tools = AnpTools::new();
        let res = tools
            .call(
                TOOL_VERIFY_SIGNATURE,
                &json!({
                    "envelope": serde_json::to_value(&env).unwrap(),
                    "public_key_hex": pk_hex
                }),
            )
            .await
            .unwrap();
        assert_eq!(res["verified"], true);
        assert_eq!(res["from_did"], "did:wba:a");
    }

    #[tokio::test]
    async fn verify_signature_returns_false_on_wrong_pubkey() {
        let kp = Keypair::from_secret_bytes(&[0x30u8; 32]).unwrap();
        let other = Keypair::from_secret_bytes(&[0x31u8; 32]).unwrap();
        let mut env = AnpEnvelope::new("did:wba:a", "did:wba:b", "ping", json!({"y": 2}));
        env.sign(&kp);

        let tools = AnpTools::new();
        let res = tools
            .call(
                TOOL_VERIFY_SIGNATURE,
                &json!({
                    "envelope": serde_json::to_value(&env).unwrap(),
                    "public_key_hex": hex::encode(other.public_key_bytes())
                }),
            )
            .await
            .unwrap();
        assert_eq!(res["verified"], false);
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let tools = AnpTools::new();
        let res = tools.call("not_a_tool", &json!({})).await;
        assert!(matches!(res, Err(AnpToolError::InvalidParams(_))));
    }
}
