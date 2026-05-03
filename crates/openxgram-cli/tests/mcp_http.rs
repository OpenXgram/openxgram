//! MCP HTTP transport 통합 테스트 — initialize / tools/list / tools/call round-trip.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::mcp_serve;
use openxgram_manifest::MachineRole;
use serde_json::{json, Value};
use tempfile::tempdir;

fn init_opts(data_dir: PathBuf) -> InitOpts {
    InitOpts {
        alias: "mcp-http-test".into(),
        role: MachineRole::Primary,
        data_dir,
        force: false,
        dry_run: false,
        import: false,
    }
}

fn set_env() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", "test-password-12345");
        std::env::remove_var("XGRAM_SEED");
    }
}

/// 임의 빈 포트로 서버 띄우고 base URL 반환. shutdown 은 task 가 drop 될 때 자동.
async fn spawn_http_server(data_dir: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    // 0.0.0.0:0 → OS 가 빈 포트 할당. 단, run_http_serve 는 println 으로 bound 출력만 하고
    // 호출자에게 실제 포트를 돌려주지 않음 → 테스트는 명시 포트 사용 (race risk 작음).
    let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    // 테스트는 실 포트 결정 후 사용해야 하므로, 직접 바인딩한 다음 axum 띄우기.
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let bound = listener.local_addr().unwrap();

    let dispatcher = mcp_serve::OpenxgramDispatcher::open(&data_dir).unwrap();
    let state = std::sync::Arc::new(tokio::sync::Mutex::new(dispatcher));

    use axum::{extract::State, routing::post, Json, Router};
    use openxgram_mcp::{handle_request, JsonRpcRequest, JsonRpcResponse};

    type SharedDispatcher = std::sync::Arc<tokio::sync::Mutex<mcp_serve::OpenxgramDispatcher>>;
    async fn rpc_handler(
        State(state): State<SharedDispatcher>,
        headers: axum::http::HeaderMap,
        Json(req): Json<JsonRpcRequest>,
    ) -> Json<JsonRpcResponse> {
        let bearer = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(str::to_string);
        let mut d = state.lock().await;
        let agent = match bearer.as_deref() {
            Some(token) => match d.verify_bearer(token) {
                Ok(Some(a)) => Some(a),
                Ok(None) => {
                    return Json(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req.id,
                        result: None,
                        error: Some(openxgram_mcp::JsonRpcError {
                            code: openxgram_mcp::ERR_INVALID_PARAMS,
                            message: "invalid bearer token".into(),
                        }),
                    });
                }
                Err(e) => {
                    return Json(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req.id,
                        result: None,
                        error: Some(openxgram_mcp::JsonRpcError {
                            code: openxgram_mcp::ERR_INTERNAL,
                            message: format!("token verify 실패: {e}"),
                        }),
                    });
                }
            },
            None => {
                if std::env::var("XGRAM_MCP_REQUIRE_AUTH").as_deref() == Ok("1") {
                    return Json(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req.id,
                        result: None,
                        error: Some(openxgram_mcp::JsonRpcError {
                            code: openxgram_mcp::ERR_INVALID_PARAMS,
                            message: "Authorization Bearer 토큰 필요".into(),
                        }),
                    });
                }
                None
            }
        };
        d.set_current_agent(agent);
        Json(handle_request(req, &mut *d))
    }

    let app = Router::new()
        .route("/rpc", post(rpc_handler))
        .with_state(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // 워밍업 — 첫 connect 안정성
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (format!("http://{bound}/rpc"), handle)
}

async fn rpc(url: &str, method: &str, params: Value) -> Value {
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
    });
    client
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
}

#[tokio::test]
async fn http_initialize_returns_protocol_info() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let (url, handle) = spawn_http_server(data_dir).await;
    let resp = rpc(&url, "initialize", json!({})).await;
    assert_eq!(resp["jsonrpc"], "2.0");
    let result = &resp["result"];
    assert!(result["protocolVersion"].is_string());
    assert_eq!(result["serverInfo"]["name"], "openxgram-mcp");

    handle.abort();
}

#[tokio::test]
async fn http_tools_list_includes_db_tools() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let (url, handle) = spawn_http_server(data_dir).await;
    let resp = rpc(&url, "tools/list", json!({})).await;
    let names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"list_sessions".to_string()));
    assert!(names.contains(&"recall_messages".to_string()));
    assert!(names.contains(&"list_memories_by_kind".to_string()));

    handle.abort();
}

#[tokio::test]
async fn http_tools_call_list_sessions_returns_zero_count() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let (url, handle) = spawn_http_server(data_dir).await;
    let resp = rpc(
        &url,
        "tools/call",
        json!({"name": "list_sessions", "arguments": {}}),
    )
    .await;
    assert_eq!(resp["result"]["count"], 0);

    handle.abort();
}

#[tokio::test]
async fn http_invalid_bearer_returns_invalid_params() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let (url, handle) = spawn_http_server(data_dir).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", "Bearer nonexistent-token")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32602);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("invalid bearer"));

    handle.abort();
}

#[tokio::test]
async fn http_require_auth_blocks_no_bearer() {
    set_env();
    unsafe { std::env::set_var("XGRAM_MCP_REQUIRE_AUTH", "1") };

    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let (url, handle) = spawn_http_server(data_dir).await;
    let resp = rpc(&url, "initialize", json!({})).await;
    assert_eq!(resp["error"]["code"], -32602);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Bearer"));

    handle.abort();
    unsafe { std::env::remove_var("XGRAM_MCP_REQUIRE_AUTH") };
}

#[tokio::test]
async fn http_unknown_method_returns_jsonrpc_error() {
    set_env();
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&init_opts(data_dir.clone())).unwrap();

    let (url, handle) = spawn_http_server(data_dir).await;
    let resp = rpc(&url, "nonexistent_method", json!({})).await;
    assert_eq!(resp["error"]["code"], -32601);

    handle.abort();
}
