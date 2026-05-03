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
    /// XGRAM_KEYSTORE_PASSWORD 환경변수가 있으면 저장. vault tools 활성 여부의 키.
    vault_password: Option<String>,
}

impl OpenxgramDispatcher {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let path = db_path(data_dir);
        if !path.exists() {
            bail!(
                "DB 미존재 ({}). `xgram init` 먼저 실행.",
                path.display()
            );
        }
        let mut db = Db::open(DbConfig {
            path,
            ..Default::default()
        })
        .context("DB open 실패")?;
        db.migrate().context("DB migrate 실패")?;
        let vault_password = openxgram_core::env::require_password().ok();
        Ok(Self { db, vault_password })
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
        ];

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
                    .ok_or_else(|| invalid("missing 'key'"))?;
                let bytes = VaultStore::new(&mut self.db)
                    .get(key, &pw)
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
                let entry = VaultStore::new(&mut self.db)
                    .set(&key, value.as_bytes(), &pw, &tags)
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
        let req: JsonRpcRequest = serde_json::from_str(&line)
            .context(format!("JSON-RPC parse 실패: {line}"))?;
        let resp = handle_request(req, &mut dispatcher);
        let json = serde_json::to_string(&resp).context("response serialize 실패")?;
        writeln!(out, "{json}")?;
        out.flush()?;
    }
    Ok(())
}
