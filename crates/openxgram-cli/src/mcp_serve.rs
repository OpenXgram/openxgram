//! xgram mcp serve — stdio JSON-RPC 서버 + db/memory 통합 tools.
//!
//! Phase 1 first PR: line-based stdin/stdout. tools 3종:
//!   - list_sessions
//!   - recall_messages (KNN, DummyEmbedder)
//!   - list_memories_by_kind
//!
//! 후속: HTTP transport, fastembed 활용, signature 검증 tool.

use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_mcp::{
    handle_request, JsonRpcError, JsonRpcRequest, ToolDispatcher, ToolSpec, ERR_INTERNAL,
    ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND,
};
use openxgram_memory::{default_embedder, MemoryKind, MemoryStore, MessageStore, SessionStore};
use openxgram_vault::VaultStore;
use serde_json::{json, Value};

pub struct OpenxgramDispatcher {
    db: Db,
    /// peer_send 등 keystore 접근 도구가 master 키 로드할 때 사용.
    data_dir: std::path::PathBuf,
    /// XGRAM_KEYSTORE_PASSWORD 환경변수가 있으면 저장. vault tools 활성 여부의 키.
    vault_password: Option<String>,
    /// HTTP transport 측에서 Bearer 토큰 검증 후 주입. None 이면 master 호출 가정.
    current_agent: Option<String>,
}

impl OpenxgramDispatcher {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let path = db_path(data_dir);
        if !path.exists() {
            bail!("DB 미존재 ({}). `xgram init` 먼저 실행.", path.display());
        }
        let mut db = Db::open(DbConfig {
            path,
            ..Default::default()
        })
        .context("DB open 실패")?;
        db.migrate().context("DB migrate 실패")?;
        let vault_password = openxgram_core::env::require_password().ok();
        Ok(Self {
            db,
            data_dir: data_dir.to_path_buf(),
            vault_password,
            current_agent: None,
        })
    }

    pub fn set_current_agent(&mut self, agent: Option<String>) {
        self.current_agent = agent;
    }

    /// Bearer 토큰 검증 — 매칭 시 agent 반환. None 이면 폐기/미발급 토큰.
    pub fn verify_bearer(&mut self, token: &str) -> Result<Option<String>> {
        crate::mcp_tokens::verify_token(&mut self.db, token)
    }

    /// 현재 호출자 — Bearer 검증된 agent 또는 fallback master.
    fn caller_agent(&self) -> &str {
        self.current_agent
            .as_deref()
            .unwrap_or(openxgram_vault::MASTER_AGENT)
    }
}

impl ToolDispatcher for OpenxgramDispatcher {
    fn tools(&self) -> Vec<ToolSpec> {
        let mut tools = vec![
            ToolSpec {
                name: "list_sessions".into(),
                description: "OpenXgram session 목록".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "recall_messages".into(),
                description: "쿼리와 가장 유사한 메시지 K 개 (sqlite-vec KNN)".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "k": {"type": "integer", "minimum": 1, "default": 5}
                    },
                    "required": ["query"]
                }),
            },
            ToolSpec {
                name: "list_memories_by_kind".into(),
                description: "L2 memories 를 kind 별로 조회".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "enum": ["fact", "decision", "reference", "rule"]}
                    },
                    "required": ["kind"]
                }),
            },
            ToolSpec {
                name: "list_peers".into(),
                description: "등록된 peer (다른 봇/노드) 목록 — alias / address / public_key".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "list_bots".into(),
                description: "이 머신에 등록된 OpenXgram 봇 목록 (xgram bot list)".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
        ];

        // peer_send — keystore 패스워드 필요 (서명용). vault 패스워드와 동일 가정.
        if self.vault_password.is_some() {
            tools.push(ToolSpec {
                name: "peer_send".into(),
                description: "지정한 peer alias 에게 message 송신 (master 키로 서명)".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "alias": {"type": "string"},
                        "body": {"type": "string"},
                        "conversation_id": {"type": "string"}
                    },
                    "required": ["alias", "body"]
                }),
            });
        }

        // vault tools — XGRAM_KEYSTORE_PASSWORD 환경에 있을 때만 노출
        if self.vault_password.is_some() {
            tools.extend([
                ToolSpec {
                    name: "vault_list".into(),
                    description: "Vault entries 메타데이터 list (값 노출 안 함)".into(),
                    input_schema: json!({"type": "object", "properties": {}}),
                },
                ToolSpec {
                    name: "vault_get".into(),
                    description: "Vault entry 평문 값 조회".into(),
                    input_schema: json!({
                        "type": "object",
                        "properties": { "key": {"type": "string"} },
                        "required": ["key"]
                    }),
                },
                ToolSpec {
                    name: "vault_set".into(),
                    description: "Vault entry 저장 (ChaCha20-Poly1305 암호화)".into(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "key": {"type": "string"},
                            "value": {"type": "string"},
                            "tags": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["key", "value"]
                    }),
                },
            ]);
        }
        tools
    }

    fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value, JsonRpcError> {
        match name {
            "list_sessions" => {
                let sessions = SessionStore::new(&mut self.db).list().map_err(internal)?;
                let items: Vec<Value> = sessions
                    .iter()
                    .map(|s| {
                        json!({
                            "id": s.id, "title": s.title,
                            "home_machine": s.home_machine,
                            "created_at": s.created_at.to_rfc3339(),
                            "last_active": s.last_active.to_rfc3339(),
                        })
                    })
                    .collect();
                Ok(json!({"sessions": items, "count": items.len()}))
            }
            "recall_messages" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'query'"))?;
                let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let embedder = default_embedder().map_err(internal)?;
                let hits = MessageStore::new(&mut self.db, embedder.as_ref())
                    .recall_top_k(query, k)
                    .map_err(internal)?;
                let items: Vec<Value> = hits
                    .iter()
                    .map(|h| {
                        json!({
                            "session_id": h.message.session_id,
                            "sender": h.message.sender,
                            "body": h.message.body,
                            "timestamp": h.message.timestamp.to_rfc3339(),
                            "distance": h.distance,
                        })
                    })
                    .collect();
                Ok(json!({"hits": items, "count": items.len()}))
            }
            "list_memories_by_kind" => {
                let kind_str = args
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'kind'"))?;
                let kind = MemoryKind::parse(kind_str)
                    .map_err(|e| invalid(&format!("invalid kind: {e}")))?;
                let memories = MemoryStore::new(&mut self.db)
                    .list_by_kind(kind)
                    .map_err(internal)?;
                let items: Vec<Value> = memories
                    .iter()
                    .map(|m| {
                        json!({
                            "id": m.id, "kind": m.kind.as_str(),
                            "content": m.content, "pinned": m.pinned,
                            "importance": m.importance,
                            "access_count": m.access_count,
                        })
                    })
                    .collect();
                Ok(json!({"memories": items, "count": items.len()}))
            }
            "list_peers" => {
                use openxgram_peer::PeerStore;
                let peers = PeerStore::new(&mut self.db).list().map_err(internal)?;
                let items: Vec<Value> = peers
                    .iter()
                    .map(|p| {
                        json!({
                            "alias": p.alias,
                            "public_key_hex": p.public_key_hex,
                            "address": p.address,
                            "role": p.role.as_str(),
                            "eth_address": p.eth_address,
                        })
                    })
                    .collect();
                Ok(json!({"peers": items, "count": items.len()}))
            }
            "list_bots" => {
                let root = crate::bot::xgram_root().map_err(internal)?;
                let reg = crate::bot::BotRegistry::load(&root).map_err(internal)?;
                let items: Vec<Value> = reg
                    .bots
                    .iter()
                    .map(|b| {
                        json!({
                            "name": b.name,
                            "alias": b.alias,
                            "transport_port": b.transport_port,
                            "gui_port": b.gui_port,
                            "data_dir": b.data_dir.display().to_string(),
                            "status": if crate::bot::pid_alive(&b.data_dir) { "running" } else { "stopped" },
                        })
                    })
                    .collect();
                Ok(json!({"bots": items, "count": items.len()}))
            }
            "peer_send" => {
                let pw = self.require_vault()?.to_string();
                let alias = args
                    .get("alias")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'alias'"))?
                    .to_string();
                let body = args
                    .get("body")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'body'"))?
                    .to_string();
                let conv = args
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let data_dir = self.data_dir.clone();
                let handle = tokio::runtime::Handle::current();
                handle
                    .block_on(crate::peer_send::run_peer_send_with_conv(
                        &data_dir, &alias, None, &body, &pw, conv,
                    ))
                    .map_err(|e| internal(e))?;
                Ok(json!({"sent": true, "alias": alias}))
            }
            "vault_list" => {
                self.require_vault()?;
                let entries = VaultStore::new(&mut self.db).list().map_err(internal)?;
                let items: Vec<Value> = entries
                    .iter()
                    .map(|e| {
                        json!({
                            "id": e.id, "key": e.key, "tags": e.tags,
                            "created_at": e.created_at.to_rfc3339(),
                            "last_accessed": e.last_accessed.to_rfc3339(),
                        })
                    })
                    .collect();
                Ok(json!({"entries": items, "count": items.len()}))
            }
            "vault_get" => {
                let pw = self.require_vault()?.to_string();
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'key'"))?
                    .to_string();
                let mfa = args
                    .get("mfa_code")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let agent = self.caller_agent().to_string();
                let bytes = VaultStore::new(&mut self.db)
                    .get_as_authed(&key, &pw, &agent, mfa.as_deref())
                    .map_err(internal)?;
                let value = std::str::from_utf8(&bytes)
                    .map(str::to_string)
                    .unwrap_or_else(|_| hex::encode(&bytes));
                Ok(json!({"key": key, "value": value}))
            }
            "vault_set" => {
                let pw = self.require_vault()?.to_string();
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'key'"))?
                    .to_string();
                let value = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'value'"))?
                    .to_string();
                let tags: Vec<String> = args
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let mfa = args
                    .get("mfa_code")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let agent = self.caller_agent().to_string();
                let entry = VaultStore::new(&mut self.db)
                    .set_as_authed(&key, value.as_bytes(), &pw, &tags, &agent, mfa.as_deref())
                    .map_err(internal)?;
                Ok(json!({"id": entry.id, "key": entry.key, "tags": entry.tags}))
            }
            other => Err(JsonRpcError {
                code: ERR_METHOD_NOT_FOUND,
                message: format!("unknown tool: {other}"),
            }),
        }
    }
}

impl OpenxgramDispatcher {
    fn require_vault(&self) -> Result<&str, JsonRpcError> {
        self.vault_password.as_deref().ok_or_else(|| JsonRpcError {
            code: ERR_INVALID_PARAMS,
            message: "vault_* tool 사용 시 XGRAM_KEYSTORE_PASSWORD 환경변수 필요".into(),
        })
    }
}

fn invalid(msg: &str) -> JsonRpcError {
    JsonRpcError {
        code: ERR_INVALID_PARAMS,
        message: msg.into(),
    }
}

fn internal(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: ERR_INTERNAL,
        message: format!("{err}"),
    }
}

/// HTTP transport — POST /rpc 로 JSON-RPC 처리.
/// 동시 요청은 dispatcher 단일 lock 직렬화 (rusqlite Connection 단일 스레드 제약).
pub async fn run_http_serve(data_dir: &Path, addr: std::net::SocketAddr) -> Result<()> {
    use axum::{extract::State, routing::post, Json, Router};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let dispatcher = OpenxgramDispatcher::open(data_dir)?;
    let state: Arc<Mutex<OpenxgramDispatcher>> = Arc::new(Mutex::new(dispatcher));

    async fn rpc_handler(
        State(state): State<Arc<Mutex<OpenxgramDispatcher>>>,
        headers: axum::http::HeaderMap,
        Json(req): Json<JsonRpcRequest>,
    ) -> Json<openxgram_mcp::JsonRpcResponse> {
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
                    // 토큰 형태이나 매칭 없음 — 거부 (master 폴백 X). agent 식별 실패.
                    return Json(openxgram_mcp::JsonRpcResponse {
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
                    return Json(openxgram_mcp::JsonRpcResponse {
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
                // 헤더 없음 → master 폴백 (현재 모드). XGRAM_MCP_REQUIRE_AUTH=1 시 reject.
                if std::env::var("XGRAM_MCP_REQUIRE_AUTH").as_deref() == Ok("1") {
                    return Json(openxgram_mcp::JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: req.id,
                        result: None,
                        error: Some(openxgram_mcp::JsonRpcError {
                            code: openxgram_mcp::ERR_INVALID_PARAMS,
                            message: "Authorization Bearer 토큰 필요 (XGRAM_MCP_REQUIRE_AUTH=1)"
                                .into(),
                        }),
                    });
                }
                None
            }
        };
        d.set_current_agent(agent);
        Json(handle_request(req, &mut *d))
    }

    async fn health_handler() -> &'static str {
        "ok"
    }

    let app = Router::new()
        .route("/rpc", post(rpc_handler))
        .route("/health", axum::routing::get(health_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("HTTP bind 실패")?;
    let bound = listener.local_addr()?;
    tracing::info!(%bound, "MCP HTTP serving");
    println!("MCP HTTP serving on http://{bound}");
    axum::serve(listener, app)
        .await
        .context("MCP HTTP serve 종료 (예기치 못한 에러)")?;
    Ok(())
}

/// stdio loop — line 단위 JSON-RPC.
pub fn run_serve(data_dir: &Path) -> Result<()> {
    let mut dispatcher = OpenxgramDispatcher::open(data_dir)?;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.context("stdin read 실패")?;
        if line.trim().is_empty() {
            continue;
        }
        let req: JsonRpcRequest =
            serde_json::from_str(&line).context(format!("JSON-RPC parse 실패: {line}"))?;
        let resp = handle_request(req, &mut dispatcher);
        let json = serde_json::to_string(&resp).context("response serialize 실패")?;
        writeln!(out, "{json}")?;
        out.flush()?;
    }
    Ok(())
}
