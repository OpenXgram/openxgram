//! openxgram-mcp — Model Context Protocol JSON-RPC server.
//!
//! Phase 1: pure handle_request + ToolDispatcher trait. EchoDispatcher 는
//! baseline 예시. db/memory 통합 dispatcher 는 cli/src/mcp_serve.rs.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "openxgram-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
pub const ERR_INVALID_PARAMS: i32 = -32602;
pub const ERR_INTERNAL: i32 = -32603;

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

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// MCP tools 의 단일 dispatch 인터페이스. 각 환경(echo / db+memory) 별로 구현.
pub trait ToolDispatcher: Send {
    fn tools(&self) -> Vec<ToolSpec>;
    fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value, JsonRpcError>;
}

pub fn handle_request<D: ToolDispatcher + ?Sized>(req: JsonRpcRequest, dispatcher: &mut D) -> JsonRpcResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        "initialize" => ok(id, initialize_result()),
        "tools/list" => ok(id, tools_list_value(dispatcher)),
        "tools/call" => match call_tool(dispatcher, &req.params) {
            Ok(v) => ok(id, v),
            Err(e) => error(id, e),
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

fn tools_list_value<D: ToolDispatcher + ?Sized>(dispatcher: &D) -> Value {
    let tools: Vec<Value> = dispatcher
        .tools()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();
    json!({ "tools": tools })
}

fn call_tool<D: ToolDispatcher + ?Sized>(dispatcher: &mut D, params: &Value) -> Result<Value, JsonRpcError> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JsonRpcError {
            code: ERR_INVALID_PARAMS,
            message: "missing 'name'".into(),
        })?;
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    dispatcher.dispatch(name, &args)
}

/// baseline dispatcher — echo tool 만. 통합 테스트·예시용.
pub struct EchoDispatcher;

impl ToolDispatcher for EchoDispatcher {
    fn tools(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "echo".into(),
            description: "단순 echo — Phase 1 baseline".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
            }),
        }]
    }

    fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value, JsonRpcError> {
        if name != "echo" {
            return Err(JsonRpcError {
                code: ERR_METHOD_NOT_FOUND,
                message: format!("unknown tool: {name}"),
            });
        }
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: ERR_INVALID_PARAMS,
                message: "echo: missing arguments.text".into(),
            })?;
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
        }))
    }
}
