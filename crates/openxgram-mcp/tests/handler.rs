//! mcp handle_request 통합 테스트.

use openxgram_mcp::{
    handle_request, EchoDispatcher, JsonRpcRequest, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND,
};
use serde_json::json;

fn req(method: &str, params: serde_json::Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: method.into(),
        params,
    }
}

#[test]
fn initialize_returns_server_info() {
    let mut d = EchoDispatcher;
    let resp = handle_request(req("initialize", json!({})), &mut d);
    let result = resp.result.expect("ok response");
    assert_eq!(result["serverInfo"]["name"], "openxgram-mcp");
    assert!(result["serverInfo"]["version"].is_string());
    assert!(result["protocolVersion"].is_string());
}

#[test]
fn tools_list_includes_echo() {
    let mut d = EchoDispatcher;
    let resp = handle_request(req("tools/list", json!({})), &mut d);
    let tools = &resp.result.unwrap()["tools"];
    assert_eq!(tools.as_array().unwrap().len(), 1);
    assert_eq!(tools[0]["name"], "echo");
}

#[test]
fn tools_call_echo_returns_text() {
    let mut d = EchoDispatcher;
    let resp = handle_request(
        req("tools/call", json!({"name": "echo", "arguments": {"text": "안녕"}})),
        &mut d,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["content"][0]["type"], "text");
    assert_eq!(result["content"][0]["text"], "안녕");
}

#[test]
fn tools_call_unknown_returns_method_not_found() {
    let mut d = EchoDispatcher;
    let resp = handle_request(
        req("tools/call", json!({"name": "nonexistent", "arguments": {}})),
        &mut d,
    );
    assert_eq!(resp.error.as_ref().unwrap().code, ERR_METHOD_NOT_FOUND);
}

#[test]
fn tools_call_missing_args_returns_invalid_params() {
    let mut d = EchoDispatcher;
    let resp = handle_request(
        req("tools/call", json!({"name": "echo", "arguments": {}})),
        &mut d,
    );
    assert_eq!(resp.error.as_ref().unwrap().code, ERR_INVALID_PARAMS);
}

#[test]
fn unknown_method_returns_method_not_found() {
    let mut d = EchoDispatcher;
    let resp = handle_request(req("foo/bar", json!({})), &mut d);
    assert_eq!(resp.error.as_ref().unwrap().code, ERR_METHOD_NOT_FOUND);
}
