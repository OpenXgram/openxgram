//! openxgram-mcp — Model Context Protocol JSON-RPC server (Phase 1 baseline).
//!
//! 자체 JSON-RPC 2.0 구현. stdio·HTTP transport 와 db/memory 통합 tools 는
//! 후속 PR. Phase 1 first PR: pure handle_request + initialize/tools/list/
//! tools/call (echo tool).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "openxgram-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
pub const ERR_INVALID_PARAMS: i32 = -32602;

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

pub fn handle_request(req: JsonRpcRequest) -> JsonRpcResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        "initialize" => ok(id, initialize_result()),
        "tools/list" => ok(id, tools_list_result()),
        "tools/call" => match dispatch_tool(&req.params) {
            Ok(value) => ok(id, value),
            Err(err) => error(id, err),
        },
        other => error(
            id,
            JsonRpcError {
                code: ERR_METHOD_NOT_FOUND,
                message: format!("method not found: {other}"),
            },
        ),
    }
}

fn ok(id: Option<Value>, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn error(id: Option<Value>, err: JsonRpcError) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(err),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        "capabilities": { "tools": {} },
    })
}

fn tools_list_result() -> Value {
    json!({
        "tools": [{
            "name": "echo",
            "description": "단순 echo — Phase 1 baseline (db/memory 통합 tool 은 후속 PR)",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
            },
        }],
    })
}

fn dispatch_tool(params: &Value) -> Result<Value, JsonRpcError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError {
            code: ERR_INVALID_PARAMS,
            message: "missing 'name'".into(),
        })?;
    match name {
        "echo" => {
            let text = params
                .get("arguments")
                .and_then(|a| a.get("text"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| JsonRpcError {
                    code: ERR_INVALID_PARAMS,
                    message: "echo: missing arguments.text".into(),
                })?;
            Ok(json!({
                "content": [{ "type": "text", "text": text }],
            }))
        }
        other => Err(JsonRpcError {
            code: ERR_METHOD_NOT_FOUND,
            message: format!("unknown tool: {other}"),
        }),
    }
}
