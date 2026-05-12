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
            ToolSpec {
                name: "whoami".into(),
                description: "이 프로젝트의 OpenXgram identity — alias / eth address / public key / linked peers 수. LLM 이 어떤 신원으로 메시지를 보낼지 알기 위한 첫 호출.".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "recv_messages".into(),
                description: "최근 inbox 메시지 (시간 내림차순). 세션 시작 시 자동 호출 권장 — 다른 에이전트가 보낸 게 있는지 확인.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 20},
                        "since_rfc3339": {"type": "string", "description": "이 timestamp 이후만 (선택)"},
                        "sender": {"type": "string", "description": "특정 sender 만 (선택)"}
                    }
                }),
            },
            ToolSpec {
                name: "connect_discord".into(),
                description: "Discord 봇 연결 — bot_token 인자 또는 vault 의 기존 값 사용. Discord API /users/@me 로 봇 검증 → vault 저장 → invite URL 반환. 양방향 통신 (사용자가 Discord 에 글 쓰면 LLM 이 받음) 의 baseline. webhook_url 은 outbound-only fallback (선택).".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_token": {"type": "string", "description": "Discord bot token (양방향). 비워두면 vault 의 기존 값"},
                        "guild_id": {"type": "string", "description": "Discord 서버 ID (서버에 카테고리 생성 시 필요, 선택)"},
                        "webhook_url": {"type": "string", "description": "outbound-only fallback (선택)"}
                    }
                }),
            },
            ToolSpec {
                name: "connect_telegram".into(),
                description: "Telegram bot 연결 — bot_token + chat_id 또는 vault 의 기존 값 사용. 테스트 메시지 발송 + vault 저장.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "bot_token": {"type": "string", "description": "비워두면 vault 의 기존 값 사용"},
                        "chat_id": {"type": "string", "description": "비워두면 vault 의 기존 값 사용"},
                        "test_message": {"type": "string", "description": "테스트 메시지 내용 (선택)"}
                    }
                }),
            },
            ToolSpec {
                name: "create_project_category".into(),
                description: "Discord 서버에 이 프로젝트용 카테고리 + 채널 자동 생성 (메인 + sub-agents). 봇 토큰 + 길드 ID 필요 (vault). 결과 채널들의 webhook URL 도 자동 발급해서 vault 에 저장.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "카테고리 이름 (생략 시 프로젝트 alias)"},
                        "subagents": {"type": "array", "items": {"type": "string"}, "description": "함께 만들 sub-agent 채널 이름들 (선택)"}
                    }
                }),
            },
            ToolSpec {
                name: "install_hooks".into(),
                description: "Claude Code SessionStart hook 설치 — ~/.claude/settings.json 에 자동 등록. 새 세션 시작 시 openxgram identity context 자동 주입 + recv_messages 자동 호출.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "scope": {"type": "string", "enum": ["user", "project"], "default": "user", "description": "user(~/.claude/settings.json) 또는 project(.claude/settings.json)"}
                    }
                }),
            },
            ToolSpec {
                name: "register_subagent".into(),
                description: "이 세션을 OpenXgram peer 로 자동 등록 — alias = role + 짧은 fingerprint. master 머신의 봇 registry 에 새 봇 추가 + auto-link. 기존 동일 alias 있으면 link 만.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "role": {"type": "string", "description": "역할 / 봇 alias (예: claude-code, codex, my-agent)"},
                        "machine": {"type": "string", "description": "머신 별 prefix (선택, 자동으로 hostname)"}
                    },
                    "required": ["role"]
                }),
            },
            ToolSpec {
                name: "send_to_discord".into(),
                description: "Discord 채널로 메시지 push (LLM 의 자연어 응답을 Discord 로). vault 의 notify.discord.webhook_url 사용. 양방향 흐름의 outbound 절반.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "보낼 내용"},
                        "webhook_url": {"type": "string", "description": "특정 채널의 webhook (생략 시 vault 의 기본값)"}
                    },
                    "required": ["content"]
                }),
            },
            ToolSpec {
                name: "send_to_telegram".into(),
                description: "Telegram 채팅으로 메시지 push. vault 의 notify.telegram.bot_token + chat_id 사용.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "보낼 내용"},
                        "chat_id": {"type": "string", "description": "특정 chat (생략 시 vault 의 기본값)"}
                    },
                    "required": ["content"]
                }),
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
            "whoami" => {
                use openxgram_manifest::InstallManifest;
                use openxgram_peer::PeerStore;
                let manifest_path = openxgram_core::paths::manifest_path(&self.data_dir);
                let manifest = InstallManifest::read(&manifest_path).map_err(internal)?;
                let peer_count = PeerStore::new(&mut self.db)
                    .list()
                    .map(|p| p.len())
                    .unwrap_or(0);
                let master = manifest
                    .registered_keys
                    .iter()
                    .find(|k| k.alias == "master")
                    .ok_or_else(|| internal("manifest 에 master key 없음"))?;
                Ok(json!({
                    "alias": manifest.machine.alias,
                    "role": format!("{:?}", manifest.machine.role).to_lowercase(),
                    "hostname": manifest.machine.hostname,
                    "address": master.address,
                    "derivation_path": master.derivation_path,
                    "data_dir": self.data_dir.display().to_string(),
                    "linked_peers_count": peer_count,
                }))
            }
            "recv_messages" => {
                use openxgram_memory::default_embedder;
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    .min(200) as usize;
                let since = args
                    .get("since_rfc3339")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let sender_filter = args
                    .get("sender")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase());
                let embedder = default_embedder().map_err(internal)?;
                let messages = MessageStore::new(&mut self.db, embedder.as_ref())
                    .list_recent(limit * 4)  // filter 적용 후도 충분히 남도록 4배 fetch
                    .map_err(internal)?;
                let items: Vec<Value> = messages
                    .into_iter()
                    .filter(|m| {
                        if let Some(ref s) = since {
                            if m.timestamp.to_rfc3339().as_str() <= s.as_str() {
                                return false;
                            }
                        }
                        if let Some(ref sf) = sender_filter {
                            if m.sender.to_lowercase() != *sf {
                                return false;
                            }
                        }
                        true
                    })
                    .take(limit)
                    .map(|m| {
                        json!({
                            "id": m.id,
                            "session_id": m.session_id,
                            "sender": m.sender,
                            "body": m.body,
                            "timestamp": m.timestamp.to_rfc3339(),
                            "conversation_id": m.conversation_id,
                        })
                    })
                    .collect();
                Ok(json!({"messages": items, "count": items.len()}))
            }
            "connect_discord" => {
                let pw = self.require_vault()?.to_string();
                let token_arg = args.get("bot_token").and_then(|v| v.as_str()).map(str::to_string);
                let guild_arg = args.get("guild_id").and_then(|v| v.as_str()).map(str::to_string);
                let webhook_arg = args.get("webhook_url").and_then(|v| v.as_str()).map(str::to_string);

                // 1. bot_token 결정 — 인자 우선, 없으면 vault.
                let bot_token = if let Some(t) = token_arg.as_ref() {
                    t.clone()
                } else {
                    let bytes = VaultStore::new(&mut self.db)
                        .get("notify.discord.bot_token", &pw)
                        .map_err(|_| invalid(
                            "Discord bot token 미설정 — bot_token 인자로 전달. \
                             webhook 만 쓸 거면 vault 에 notify.discord.webhook_url 직접 set"
                        ))?;
                    String::from_utf8(bytes).map_err(|e| internal(format!("token utf8: {e}")))?
                };

                // 2. Discord API /users/@me 로 봇 검증.
                let token_clone = bot_token.clone();
                let bot_info: Value = std::thread::spawn(move || -> Result<Value, String> {
                    let resp = reqwest::blocking::Client::new()
                        .get("https://discord.com/api/v10/users/@me")
                        .header("Authorization", format!("Bot {}", token_clone))
                        .send()
                        .map_err(|e| format!("Discord API GET /users/@me: {e}"))?;
                    if !resp.status().is_success() {
                        return Err(format!(
                            "bot token 검증 실패: HTTP {} — {}",
                            resp.status(), resp.text().unwrap_or_default()
                        ));
                    }
                    resp.json().map_err(|e| format!("json: {e}"))
                }).join().map_err(|_| internal("HTTP thread panic"))?
                  .map_err(internal)?;

                let bot_id = bot_info.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let bot_username = bot_info.get("username").and_then(|v| v.as_str()).unwrap_or("?").to_string();

                // 3. vault 저장.
                if token_arg.is_some() {
                    VaultStore::new(&mut self.db)
                        .set("notify.discord.bot_token", bot_token.as_bytes(), &pw, &[])
                        .map_err(|e| internal(format!("vault set bot_token: {e}")))?;
                }
                if let Some(g) = guild_arg.as_ref() {
                    VaultStore::new(&mut self.db)
                        .set("notify.discord.guild_id", g.as_bytes(), &pw, &[])
                        .map_err(|e| internal(format!("vault set guild_id: {e}")))?;
                }
                if let Some(w) = webhook_arg.as_ref() {
                    VaultStore::new(&mut self.db)
                        .set("notify.discord.webhook_url", w.as_bytes(), &pw, &[])
                        .map_err(|e| internal(format!("vault set webhook_url: {e}")))?;
                }

                // 4. invite URL — 봇이 서버에 들어가야 양방향 가능.
                //    permissions=536895680 ≈ Send/Read/Manage Channels/Webhooks (개략).
                let invite_url = format!(
                    "https://discord.com/api/oauth2/authorize?client_id={}&scope=bot&permissions=536895680",
                    bot_id
                );

                // 5. vault 갱신됐으면 agent 재시작 — 새 토큰 즉시 효력. 비-blocking.
                let mut agent_restarted = false;
                if token_arg.is_some() || webhook_arg.is_some() {
                    let xgram_bin = std::env::current_exe().ok();
                    if let Some(bin) = xgram_bin {
                        let _ = std::process::Command::new(bin)
                            .arg("agent-restart")
                            .arg("--data-dir")
                            .arg(&self.data_dir)
                            .env("XGRAM_KEYSTORE_PASSWORD", &pw)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .stdin(std::process::Stdio::null())
                            .status();
                        agent_restarted = true;
                    }
                }

                Ok(json!({
                    "ok": true,
                    "bot_id": bot_id,
                    "bot_username": bot_username,
                    "invite_url": invite_url,
                    "vault_saved": {
                        "bot_token": token_arg.is_some(),
                        "guild_id": guild_arg.is_some(),
                        "webhook_url": webhook_arg.is_some()
                    },
                    "agent_restarted": agent_restarted,
                    "next": if guild_arg.is_none() {
                        json!("[1] 위 invite_url 로 봇을 마스터의 Discord 서버에 초대  [2] 그 서버 ID 를 guild_id 로 다시 connect_discord 호출  [3] create_project_category 호출로 카테고리+채널+webhook 자동 생성")
                    } else {
                        json!("create_project_category(name?) 호출로 카테고리 + 채널 + webhook 자동 생성 가능")
                    }
                }))
            }
            "connect_telegram" => {
                let pw = self.require_vault()?.to_string();
                let token_arg = args.get("bot_token").and_then(|v| v.as_str()).map(str::to_string);
                let chat_arg = args.get("chat_id").and_then(|v| v.as_str()).map(str::to_string);
                let test_msg = args
                    .get("test_message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("✓ OpenXgram → Telegram 연결 테스트")
                    .to_string();

                let read_vault_or_arg = |arg: &Option<String>, key: &str, db: &mut Db| -> Result<String, JsonRpcError> {
                    if let Some(v) = arg { return Ok(v.clone()); }
                    let bytes = VaultStore::new(db)
                        .get(key, &pw)
                        .map_err(|_| invalid(&format!("{} 미설정 — 인자로 전달하거나 vault 에 미리 저장", key)))?;
                    String::from_utf8(bytes).map_err(|e| internal(format!("vault utf8: {e}")))
                };

                let token = read_vault_or_arg(&token_arg, "notify.telegram.bot_token", &mut self.db)?;
                let chat_id = read_vault_or_arg(&chat_arg, "notify.telegram.chat_id", &mut self.db)?;

                let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
                let body_str = serde_json::to_string(&json!({"chat_id": chat_id, "text": test_msg})).unwrap();
                let (status_code, err_body): (u16, String) = std::thread::spawn(move || -> Result<(u16, String), String> {
                    let resp = reqwest::blocking::Client::new()
                        .post(&url)
                        .header("content-type", "application/json")
                        .body(body_str)
                        .send()
                        .map_err(|e| format!("Telegram POST: {e}"))?;
                    let status = resp.status().as_u16();
                    let text = if (200..300).contains(&status) {
                        String::new()
                    } else {
                        resp.text().unwrap_or_else(|_| "(body 읽기 실패)".into())
                    };
                    Ok((status, text))
                }).join().map_err(|_| internal("HTTP thread panic"))?
                  .map_err(internal)?;
                if !(200..300).contains(&status_code) {
                    return Err(invalid(&format!(
                        "Telegram API 응답 비정상: HTTP {} — {}",
                        status_code, err_body
                    )));
                }

                // 새 값 받은 경우만 vault 저장.
                if token_arg.is_some() {
                    VaultStore::new(&mut self.db)
                        .set("notify.telegram.bot_token", token.as_bytes(), &pw, &[])
                        .map_err(|e| internal(format!("vault set: {e}")))?;
                }
                if chat_arg.is_some() {
                    VaultStore::new(&mut self.db)
                        .set("notify.telegram.chat_id", chat_id.as_bytes(), &pw, &[])
                        .map_err(|e| internal(format!("vault set: {e}")))?;
                }

                // vault 갱신됐으면 agent 재시작 — connect_discord 와 동일 패턴.
                let mut agent_restarted = false;
                if token_arg.is_some() || chat_arg.is_some() {
                    if let Some(bin) = std::env::current_exe().ok() {
                        let _ = std::process::Command::new(bin)
                            .arg("agent-restart")
                            .arg("--data-dir")
                            .arg(&self.data_dir)
                            .env("XGRAM_KEYSTORE_PASSWORD", &pw)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .stdin(std::process::Stdio::null())
                            .status();
                        agent_restarted = true;
                    }
                }

                Ok(json!({
                    "ok": true,
                    "test_status": status_code,
                    "agent_restarted": agent_restarted,
                    "next": "이제 LLM 이 send_to_telegram 으로 메시지 송신 가능"
                }))
            }
            "send_to_discord" => {
                let pw = self.require_vault()?.to_string();
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'content'"))?
                    .to_string();
                let webhook = if let Some(w) = args.get("webhook_url").and_then(|v| v.as_str()) {
                    w.to_string()
                } else {
                    let bytes = VaultStore::new(&mut self.db)
                        .get("notify.discord.webhook_url", &pw)
                        .map_err(|_| invalid("notify.discord.webhook_url vault 에 없음 — 먼저 connect_discord 또는 create_project_category"))?;
                    String::from_utf8(bytes).map_err(|e| internal(format!("vault utf8: {e}")))?
                };

                let body_str = serde_json::to_string(&json!({"content": content})).unwrap();
                let webhook_clone = webhook.clone();
                let (status, err_body): (u16, String) = std::thread::spawn(move || -> Result<(u16, String), String> {
                    let resp = reqwest::blocking::Client::new()
                        .post(&webhook_clone)
                        .header("content-type", "application/json")
                        .body(body_str)
                        .send()
                        .map_err(|e| format!("Discord POST: {e}"))?;
                    let s = resp.status().as_u16();
                    let t = if (200..300).contains(&s) {
                        String::new()
                    } else {
                        resp.text().unwrap_or_default()
                    };
                    Ok((s, t))
                }).join().map_err(|_| internal("HTTP thread panic"))?
                  .map_err(internal)?;
                if !(200..300).contains(&status) {
                    return Err(invalid(&format!("Discord webhook 응답 HTTP {status}: {err_body}")));
                }
                Ok(json!({"sent": true, "status": status, "content_len": content.len()}))
            }
            "send_to_telegram" => {
                let pw = self.require_vault()?.to_string();
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'content'"))?
                    .to_string();
                let token_bytes = VaultStore::new(&mut self.db)
                    .get("notify.telegram.bot_token", &pw)
                    .map_err(|_| invalid("notify.telegram.bot_token vault 에 없음 — 먼저 connect_telegram"))?;
                let token = String::from_utf8(token_bytes).map_err(|e| internal(format!("vault utf8: {e}")))?;
                let chat_id = if let Some(c) = args.get("chat_id").and_then(|v| v.as_str()) {
                    c.to_string()
                } else {
                    let bytes = VaultStore::new(&mut self.db)
                        .get("notify.telegram.chat_id", &pw)
                        .map_err(|_| invalid("notify.telegram.chat_id vault 에 없음"))?;
                    String::from_utf8(bytes).map_err(|e| internal(format!("vault utf8: {e}")))?
                };

                let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
                let body_str = serde_json::to_string(&json!({"chat_id": chat_id, "text": content})).unwrap();
                let (status, err_body): (u16, String) = std::thread::spawn(move || -> Result<(u16, String), String> {
                    let resp = reqwest::blocking::Client::new()
                        .post(&url)
                        .header("content-type", "application/json")
                        .body(body_str)
                        .send()
                        .map_err(|e| format!("Telegram POST: {e}"))?;
                    let s = resp.status().as_u16();
                    let t = if (200..300).contains(&s) {
                        String::new()
                    } else {
                        resp.text().unwrap_or_default()
                    };
                    Ok((s, t))
                }).join().map_err(|_| internal("HTTP thread panic"))?
                  .map_err(internal)?;
                if !(200..300).contains(&status) {
                    return Err(invalid(&format!("Telegram API 응답 HTTP {status}: {err_body}")));
                }
                Ok(json!({"sent": true, "status": status, "content_len": content.len()}))
            }
            "register_subagent" => {
                let pw = self.require_vault()?.to_string();
                let role = args
                    .get("role")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'role'"))?;
                if !role.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err(invalid("role 은 영숫자/-/_ 만"));
                }

                // 이미 등록된 봇이면 skip + 정보 반환.
                let root = crate::bot::xgram_root().map_err(internal)?;
                let reg = crate::bot::BotRegistry::load(&root).map_err(internal)?;
                if let Some(existing) = reg.get(role) {
                    return Ok(json!({
                        "already_registered": true,
                        "name": existing.name,
                        "alias": existing.alias,
                        "data_dir": existing.data_dir.display().to_string(),
                        "transport_port": existing.transport_port,
                    }));
                }

                // subprocess 로 분리 — bot_register 의 init [1/6] println 이 stdio JSON-RPC 와
                // 섞이는 걸 방지. stdout/stderr 모두 폐기.
                let xgram_bin = std::env::current_exe()
                    .map_err(|e| internal(format!("xgram bin path: {e}")))?;
                let output = std::process::Command::new(&xgram_bin)
                    .args(["bot", "register", role])
                    .env("XGRAM_KEYSTORE_PASSWORD", &pw)
                    .env("XGRAM_SKIP_PORT_PRECHECK", "1")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .stdin(std::process::Stdio::null())
                    .status()
                    .map_err(|e| internal(format!("xgram bot register spawn: {e}")))?;
                if !output.success() {
                    return Err(internal(format!(
                        "xgram bot register {role} 실패 (exit {:?})",
                        output.code()
                    )));
                }

                let reg2 = crate::bot::BotRegistry::load(&root).map_err(internal)?;
                let entry = reg2
                    .get(role)
                    .ok_or_else(|| internal("등록 직후 조회 실패"))?;
                Ok(json!({
                    "registered": true,
                    "name": entry.name,
                    "alias": entry.alias,
                    "data_dir": entry.data_dir.display().to_string(),
                    "transport_port": entry.transport_port,
                    "mcp_port": entry.transport_port + 2,
                }))
            }
            "install_hooks" => {
                let scope = args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user");
                let settings_path = match scope {
                    "user" => {
                        let home = std::env::var("HOME")
                            .or_else(|_| std::env::var("USERPROFILE"))
                            .map_err(|_| internal("HOME/USERPROFILE 미설정"))?;
                        std::path::PathBuf::from(home).join(".claude/settings.json")
                    }
                    "project" => std::env::current_dir().map_err(|e| internal(e))?
                        .join(".claude/settings.json"),
                    _ => return Err(invalid("scope 는 user 또는 project")),
                };

                if let Some(parent) = settings_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| internal(e))?;
                }

                let mut settings: Value = if settings_path.exists() {
                    let raw = std::fs::read_to_string(&settings_path).map_err(|e| internal(e))?;
                    serde_json::from_str(&raw)
                        .map_err(|e| internal(format!("settings.json 파싱 실패: {e}")))?
                } else {
                    json!({})
                };

                let hooks = settings
                    .as_object_mut()
                    .ok_or_else(|| internal("settings.json root 가 object 아님"))?
                    .entry("hooks".to_string())
                    .or_insert_with(|| json!({}));
                let session_start = hooks
                    .as_object_mut()
                    .ok_or_else(|| internal("hooks 가 object 아님"))?
                    .entry("SessionStart".to_string())
                    .or_insert_with(|| json!([]));
                let arr = session_start
                    .as_array_mut()
                    .ok_or_else(|| internal("SessionStart 가 array 아님"))?;

                // 기존 openxgram 훅 있으면 skip.
                let already = arr.iter().any(|h| {
                    h.get("hooks").and_then(|v| v.as_array()).map(|hs| {
                        hs.iter().any(|sub| {
                            sub.get("command")
                                .and_then(|v| v.as_str())
                                .map(|s| s.contains("xgram") && s.contains("identity"))
                                .unwrap_or(false)
                        })
                    }).unwrap_or(false)
                });

                if !already {
                    arr.push(json!({
                        "matcher": "*",
                        "hooks": [{
                            "type": "command",
                            "command": "xgram identity-inject --target CLAUDE.md 2>/dev/null || true"
                        }]
                    }));
                }

                let new_content = serde_json::to_string_pretty(&settings).map_err(|e| internal(e))?;
                std::fs::write(&settings_path, new_content).map_err(|e| internal(e))?;

                Ok(json!({
                    "installed": !already,
                    "already": already,
                    "settings_path": settings_path.display().to_string(),
                    "scope": scope,
                    "next": "다음 Claude Code 세션 시작 시 xgram identity-inject 자동 호출 — CLAUDE.md 항상 최신"
                }))
            }
            "create_project_category" => {
                let pw = self.require_vault()?.to_string();
                let name_arg = args.get("name").and_then(|v| v.as_str()).map(str::to_string);
                let subagents: Vec<String> = args
                    .get("subagents")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();

                // bot_token + guild_id 는 vault 에서.
                let bot_token = VaultStore::new(&mut self.db)
                    .get("notify.discord.bot_token", &pw)
                    .map_err(|_| invalid("notify.discord.bot_token vault 에 없음 — 먼저 connect_discord 또는 xgram setup discord"))?;
                let bot_token = String::from_utf8(bot_token).map_err(|e| internal(format!("token utf8: {e}")))?;
                let guild_id = VaultStore::new(&mut self.db)
                    .get("notify.discord.guild_id", &pw)
                    .map_err(|_| invalid("notify.discord.guild_id vault 에 없음 — vault_set 으로 추가 (Discord 서버 ID)"))?;
                let guild_id = String::from_utf8(guild_id).map_err(|e| internal(format!("guild_id utf8: {e}")))?;

                // 프로젝트 alias 가져오기 (manifest).
                let project_alias = {
                    use openxgram_manifest::InstallManifest;
                    InstallManifest::read(&openxgram_core::paths::manifest_path(&self.data_dir))
                        .map(|m| m.machine.alias.clone())
                        .unwrap_or_else(|_| "openxgram".into())
                };
                let category_name = name_arg.unwrap_or(project_alias);

                let token_clone = bot_token.clone();
                let guild_clone = guild_id.clone();
                let cat_name = category_name.clone();
                let subs = subagents.clone();
                let result = std::thread::spawn(move || -> Result<Value, String> {
                    let client = reqwest::blocking::Client::new();
                    let auth = format!("Bot {}", token_clone);

                    // 1. 카테고리 생성 (type=4).
                    let cat_resp = client
                        .post(format!("https://discord.com/api/v10/guilds/{}/channels", guild_clone))
                        .header("Authorization", &auth)
                        .header("content-type", "application/json")
                        .body(serde_json::to_string(&json!({"name": cat_name, "type": 4})).unwrap())
                        .send()
                        .map_err(|e| format!("category POST: {e}"))?;
                    if !cat_resp.status().is_success() {
                        return Err(format!(
                            "Discord 카테고리 생성 실패: HTTP {} — {}",
                            cat_resp.status(),
                            cat_resp.text().unwrap_or_default()
                        ));
                    }
                    let cat_obj: Value = cat_resp.json().map_err(|e| format!("category json: {e}"))?;
                    let cat_id = cat_obj.get("id").and_then(|v| v.as_str()).ok_or("카테고리 id 누락")?.to_string();

                    // 2. 채널들 생성 — main + sub-agents.
                    let mut channels: Vec<Value> = Vec::new();
                    let names: Vec<String> = std::iter::once("main".to_string()).chain(subs.into_iter()).collect();
                    for ch_name in &names {
                        let ch_resp = client
                            .post(format!("https://discord.com/api/v10/guilds/{}/channels", guild_clone))
                            .header("Authorization", &auth)
                            .header("content-type", "application/json")
                            .body(serde_json::to_string(&json!({
                                "name": ch_name,
                                "type": 0,
                                "parent_id": cat_id,
                            })).unwrap())
                            .send()
                            .map_err(|e| format!("channel {ch_name} POST: {e}"))?;
                        if !ch_resp.status().is_success() {
                            return Err(format!("채널 {ch_name} 생성 실패: HTTP {}", ch_resp.status()));
                        }
                        let ch_obj: Value = ch_resp.json().map_err(|e| format!("channel json: {e}"))?;
                        let ch_id = ch_obj.get("id").and_then(|v| v.as_str()).ok_or("채널 id 누락")?.to_string();

                        // 3. webhook 발급.
                        let wh_resp = client
                            .post(format!("https://discord.com/api/v10/channels/{}/webhooks", ch_id))
                            .header("Authorization", &auth)
                            .header("content-type", "application/json")
                            .body(serde_json::to_string(&json!({"name": format!("openxgram-{ch_name}")})).unwrap())
                            .send()
                            .map_err(|e| format!("webhook {ch_name} POST: {e}"))?;
                        if !wh_resp.status().is_success() {
                            return Err(format!("채널 {ch_name} webhook 발급 실패: HTTP {}", wh_resp.status()));
                        }
                        let wh_obj: Value = wh_resp.json().map_err(|e| format!("webhook json: {e}"))?;
                        let wh_url = wh_obj.get("url").and_then(|v| v.as_str()).ok_or("webhook url 누락")?.to_string();

                        channels.push(json!({"name": ch_name, "channel_id": ch_id, "webhook_url": wh_url}));
                    }

                    Ok(json!({"category_id": cat_id, "channels": channels}))
                }).join().map_err(|_| internal("HTTP thread panic"))?
                  .map_err(internal)?;

                // vault 에 main 채널의 webhook 저장 (기본 forward 채널).
                if let Some(main_ch) = result.get("channels").and_then(|v| v.as_array()).and_then(|a| a.first()) {
                    if let Some(url) = main_ch.get("webhook_url").and_then(|v| v.as_str()) {
                        VaultStore::new(&mut self.db)
                            .set("notify.discord.webhook_url", url.as_bytes(), &pw, &[])
                            .map_err(|e| internal(format!("vault set: {e}")))?;
                    }
                }

                Ok(result)
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
