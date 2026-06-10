//! ACP wire types (serde) — `docs/research/acp-core-integration.md` §1.3–§1.6.
//!
//! Field names follow the spec exactly via `#[serde(rename_all = "camelCase")]`
//! (the JSON-RPC wire uses camelCase). Unknown fields are tolerated on inbound
//! types so forward-compatible agents do not break us, but every *failure* is
//! still explicit elsewhere (절대 규칙 1) — tolerance here is forward-compat,
//! not silent fallback.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `initialize` request params (Client → Agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    /// Protocol MAJOR version we speak.
    pub protocol_version: u32,
    /// Capabilities the client exposes back to the agent.
    pub client_capabilities: ClientCapabilities,
    /// Identifying info for the client.
    pub client_info: ClientInfo,
}

/// `initialize` response (Agent → Client).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    /// Protocol version the agent agrees to (or its latest if it cannot match).
    pub protocol_version: u32,
    /// Capabilities the agent advertises.
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    /// Identifying info for the agent.
    #[serde(default)]
    pub agent_info: Option<AgentInfo>,
    /// Authentication methods; non-empty means auth is required.
    #[serde(default)]
    pub auth_methods: Vec<Value>,
}

/// Client identity (`clientInfo`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Machine-readable name.
    pub name: String,
    /// Human-readable title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Client version string.
    pub version: String,
}

/// Agent identity (`agentInfo`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Machine-readable name.
    #[serde(default)]
    pub name: String,
    /// Agent version string.
    #[serde(default)]
    pub version: String,
}

/// Capabilities the client advertises to the agent (§3.3).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientCapabilities {
    /// File-system callback capabilities.
    #[serde(default)]
    pub fs: FsCapabilities,
    /// Whether `terminal/*` callbacks are accepted.
    #[serde(default)]
    pub terminal: bool,
}

/// `fs/*` callback capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FsCapabilities {
    /// Accept `fs/read_text_file` callbacks.
    #[serde(default)]
    pub read_text_file: bool,
    /// Accept `fs/write_text_file` callbacks.
    #[serde(default)]
    pub write_text_file: bool,
}

/// Capabilities the agent advertises (§1.3).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Whether `session/load` is supported.
    #[serde(default)]
    pub load_session: bool,
    /// Which prompt ContentBlock variants the agent accepts.
    #[serde(default)]
    pub prompt_capabilities: PromptCapabilities,
    /// MCP server transport capabilities (free-form).
    #[serde(default)]
    pub mcp_capabilities: Value,
}

/// Prompt content capabilities gating optional ContentBlock variants (§1.4).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    /// Agent accepts `image` blocks.
    #[serde(default)]
    pub image: bool,
    /// Agent accepts `audio` blocks.
    #[serde(default)]
    pub audio: bool,
    /// Agent accepts inline `resource` (embeddedContext) blocks.
    #[serde(default)]
    pub embedded_context: bool,
}

/// `session/new` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewRequest {
    /// Working directory for the session.
    pub cwd: String,
    /// MCP servers to expose to the agent (may be empty).
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
}

/// `session/new` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewResponse {
    /// The newly created session id.
    pub session_id: String,
}

/// `session/prompt` request params.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptRequest {
    /// Target session.
    pub session_id: String,
    /// Prompt content blocks.
    pub prompt: Vec<ContentBlock>,
}

/// `session/prompt` response — resolves only at turn end.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptResponse {
    /// Why the turn ended.
    pub stop_reason: StopReason,
}

/// ACP ContentBlock (§1.4) — tagged union on `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text (baseline — all agents support).
    Text {
        /// The text payload.
        text: String,
    },
    /// A link to a resource (baseline).
    ResourceLink {
        /// Resource URI.
        uri: String,
        /// Optional display name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Image content (gated by `promptCapabilities.image`).
    Image {
        /// Base64 data.
        data: String,
        /// MIME type.
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    /// Audio content (gated by `promptCapabilities.audio`).
    Audio {
        /// Base64 data.
        data: String,
        /// MIME type.
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
    /// Inline embedded resource (gated by `promptCapabilities.embeddedContext`).
    Resource {
        /// The embedded resource object (`{uri, mimeType, text}`).
        resource: Value,
    },
}

impl ContentBlock {
    /// Convenience constructor for a text block.
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }
}

/// `session/update` notification (§1.3) — tagged on `sessionUpdate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    /// A chunk of the agent's message.
    AgentMessageChunk {
        /// The content chunk.
        content: ContentBlock,
    },
    /// A chunk of the (replayed) user message.
    UserMessageChunk {
        /// The content chunk.
        content: ContentBlock,
    },
    /// A chunk of the agent's internal reasoning.
    AgentThoughtChunk {
        /// The content chunk.
        content: ContentBlock,
    },
    /// A plan update.
    Plan {
        /// Plan entries (`{content, priority, status}`).
        #[serde(default)]
        entries: Vec<Value>,
    },
    /// A new tool call.
    ToolCall(ToolCall),
    /// Progress/final update for an existing tool call.
    ToolCallUpdate {
        /// The tool call id being updated.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// New status if present.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ToolCallStatus>,
        /// Updated content if present.
        #[serde(default)]
        content: Vec<Value>,
    },
}

/// A tool call surfaced via `session/update` (§1.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    /// Stable id for this tool call.
    pub tool_call_id: String,
    /// Human-readable title.
    #[serde(default)]
    pub title: String,
    /// Tool kind (read|edit|delete|move|search|execute|think|fetch|other).
    #[serde(default)]
    pub kind: String,
    /// Current status.
    pub status: ToolCallStatus,
    /// Content blocks / diffs / terminal refs (free-form per variant).
    #[serde(default)]
    pub content: Vec<Value>,
    /// File locations touched.
    #[serde(default)]
    pub locations: Vec<Value>,
    /// Raw tool input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<Value>,
    /// Raw tool output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<Value>,
}

/// Tool call status (§1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// Not yet started.
    Pending,
    /// Running.
    InProgress,
    /// Finished successfully.
    Completed,
    /// Finished with failure.
    Failed,
}

/// Reason a `session/prompt` turn ended (§1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Normal completion.
    EndTurn,
    /// Hit a max-tokens limit.
    MaxTokens,
    /// Hit a max-turn-requests limit.
    MaxTurnRequests,
    /// The agent refused.
    Refusal,
    /// The turn was cancelled (in response to `session/cancel`).
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_block_text_roundtrip() {
        let b = ContentBlock::text("hi");
        let j = serde_json::to_value(&b).expect("serialize");
        assert_eq!(j["type"], "text");
        assert_eq!(j["text"], "hi");
        let back: ContentBlock = serde_json::from_value(j).expect("deserialize");
        matches!(back, ContentBlock::Text { .. });
    }

    #[test]
    fn session_update_tag_is_session_update() {
        let u = SessionUpdate::AgentMessageChunk {
            content: ContentBlock::text("yo"),
        };
        let j = serde_json::to_value(&u).expect("serialize");
        assert_eq!(j["sessionUpdate"], "agent_message_chunk");
        assert_eq!(j["content"]["text"], "yo");
    }

    #[test]
    fn stop_reason_serde() {
        let v = serde_json::to_value(StopReason::EndTurn).expect("ser");
        assert_eq!(v, serde_json::json!("end_turn"));
        let p: StopReason = serde_json::from_value(serde_json::json!("cancelled")).expect("de");
        assert_eq!(p, StopReason::Cancelled);
    }

    #[test]
    fn initialize_request_is_camel_case() {
        let req = InitializeRequest {
            protocol_version: 1,
            client_capabilities: ClientCapabilities::default(),
            client_info: ClientInfo {
                name: "openxgram".into(),
                title: None,
                version: "0".into(),
            },
        };
        let j = serde_json::to_value(&req).expect("ser");
        assert_eq!(j["protocolVersion"], 1);
        assert!(j.get("clientCapabilities").is_some());
        assert!(j.get("clientInfo").is_some());
    }

    #[test]
    fn tool_call_update_camel_case() {
        let j = serde_json::json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "tc-1",
            "status": "completed",
            "content": []
        });
        let u: SessionUpdate = serde_json::from_value(j).expect("de");
        match u {
            SessionUpdate::ToolCallUpdate {
                tool_call_id,
                status,
                ..
            } => {
                assert_eq!(tool_call_id, "tc-1");
                assert_eq!(status, Some(ToolCallStatus::Completed));
            }
            _ => panic!("wrong variant"),
        }
    }
}
