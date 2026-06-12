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
use openxgram_mistakes::{mcp::MistakeTools, NewMistake};
use openxgram_patterns::{mcp::PatternTools, pattern::ActionStep};
use openxgram_vault::VaultStore;
use openxgram_wiki::{mcp::WikiTools, WikiFs};
use serde_json::{json, Value};

/// mcp-serve 시작 시 자격(`XGRAM_KEYSTORE_PASSWORD` / `XGRAM_MCP_TOKEN`) 해석.
///
/// 반복 핵심 문제 fix: Claude Code 가 세션마다 띄우는 `xgram mcp-serve` 프로세스에
/// config env 주입이 누락되면 peer_send/vault 가 "비번 필요" 로 실패한다.
/// config env 배선에 의존하는 게 취약점이므로, **같은 머신·같은 유저**라는 전제 하에
/// `{data_dir}/daemon.env`(데몬이 쓰는 동일 파일)에서 비번을 자동 확보한다.
///
/// 해석 순서:
///   1) env `XGRAM_MCP_TOKEN` 존재 → 그대로 (데몬 서명 경로). 추가 작업 없음.
///   2) env `XGRAM_KEYSTORE_PASSWORD` 존재 → 그대로.
///   3) 둘 다 없음 → `daemon.env` 의 `XGRAM_KEYSTORE_PASSWORD=` 줄을 읽어
///      현재 프로세스 env 로 set (이후 require_password / peer_send 서명이 자동으로 사용).
///   파일/키 없으면 명시 경고 로그(silent 금지) 후 자격 없이 진행(읽기 도구만 동작).
///
/// 비밀번호 값은 로그/출력에 평문으로 내보내지 않는다 — 존재 여부만 기록.
pub fn ensure_mcp_credentials(data_dir: &Path) {
    // 1) 데몬 서명 토큰이 있으면 keystore 비번 없이도 peer_send 가능 — 그대로 둔다.
    if std::env::var("XGRAM_MCP_TOKEN").is_ok() {
        eprintln!("[mcp-serve][cred] XGRAM_MCP_TOKEN env 존재 — 데몬 서명 경로 사용");
        return;
    }
    // 2) keystore 비번이 이미 env 에 있으면 그대로 둔다 (기존 동작).
    if std::env::var(openxgram_core::env::PASSWORD_ENV).is_ok() {
        eprintln!("[mcp-serve][cred] XGRAM_KEYSTORE_PASSWORD env 존재 — 기존 자격 사용");
        return;
    }
    // 3) 둘 다 없음 → daemon.env fallback.
    let env_path = data_dir.join("daemon.env");
    match read_daemon_env_password(&env_path) {
        Some(pw) if !pw.is_empty() => {
            std::env::set_var(openxgram_core::env::PASSWORD_ENV, &pw);
            eprintln!(
                "[mcp-serve][cred] {} 에서 XGRAM_KEYSTORE_PASSWORD 자동 확보 — peer_send/vault 활성",
                env_path.display()
            );
        }
        _ => {
            eprintln!(
                "[mcp-serve][cred][WARN] env 자격 없음 + {} 에 XGRAM_KEYSTORE_PASSWORD 없음 \
                 — 자격 없이 진행 (읽기 도구만 동작, peer_send/vault 불가)",
                env_path.display()
            );
        }
    }
}

/// `daemon.env`(KEY=VALUE 라인 형식, 데몬도 같은 파일을 source)에서
/// `XGRAM_KEYSTORE_PASSWORD` 값을 추출. 파일/키 없으면 None.
/// 따옴표 한 겹과 `export ` prefix 는 벗겨낸다. 값은 trim (env.rs require_password 와 동일 정책).
fn read_daemon_env_password(env_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(env_path).ok()?;
    let key = openxgram_core::env::PASSWORD_ENV;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(v);
        let v = v.trim();
        if v.is_empty() {
            return None;
        }
        return Some(v.to_string());
    }
    None
}

pub struct OpenxgramDispatcher {
    db: Db,
    /// peer_send 등 keystore 접근 도구가 master 키 로드할 때 사용.
    data_dir: std::path::PathBuf,
    /// XGRAM_KEYSTORE_PASSWORD 환경변수가 있으면 저장. vault tools 활성 여부의 키.
    vault_password: Option<String>,
    /// HTTP transport 측에서 Bearer 토큰 검증 후 주입. None 이면 master 호출 가정.
    current_agent: Option<String>,
    /// ACP (Agent Client Protocol) tool 표면 — spawn 된 agent process registry 를
    /// 내부에 보유 (HashMap<handleId, AcpClient> behind async Mutex). dispatch 간
    /// 영속 (§3.1 — agent 가 단일 request frame 보다 오래 살아야 함). Phase B-2.
    acp_tools: openxgram_acp::AcpTools,
    /// A2A (Google Agent2Agent) tool 표면 — agent↔agent: OpenXgram 이 외부/타
    /// 에이전트의 A2A endpoint 를 호출 (AgentCard discover + tasks/send|get|cancel).
    /// stateless (매 호출 새 client). callee 측 AgentCard 호스팅은 후속. Phase 3.
    a2a_tools: openxgram_a2a::A2aTools,
    /// Marketplace 상거래 tool 표면 — OpenAgentX 디렉토리 검색·에이전트 조회·서비스 구매(job).
    /// 원격 HTTP(reqwest, base_url=OpenAgentX). MarketplaceTools 는 Clone 미구현이라 Arc 로
    /// 감싸 sync dispatch 간 공유. 마켓 (a)갈래 배선. 구매(purchase)는 실결제((c)) 후 노출.
    marketplace_tools: std::sync::Arc<openxgram_marketplace::MarketplaceTools>,
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
        // Marketplace client — base_url 은 XGRAM_MARKETPLACE_URL env, 미설정 시 기본(openagentx.org).
        // 디렉토리 미배포면 search/get 호출이 실제 HTTP 에러 반환(가짜 성공 없음).
        let mp_builder = openxgram_marketplace::MarketplaceClient::builder();
        let mp_builder = match std::env::var("XGRAM_MARKETPLACE_URL") {
            Ok(u) if !u.trim().is_empty() => mp_builder.base_url(u),
            _ => mp_builder,
        };
        // 마켓 (c)갈래 — 결제 게이트웨이를 **설정 기반**으로 선택 (가짜 성공 금지).
        //   - 온체인: `XGRAM_CHAIN_RPC`(체인 RPC URL) env **그리고** vault 마스터 키 비밀번호
        //     (`XGRAM_KEYSTORE_PASSWORD`, 위 `vault_password`) 가 둘 다 있을 때만.
        //     `OnchainPaymentGateway` 가 submit_intent 로 실제 USDC transfer 제출 — 자금/RPC
        //     부재 시 RPC reject → 실제 에러 반환(silent fallback 없음).
        //   - 그 외(기본): 기존 `LedgerPaymentGateway`(내부 원장). env 미설정이 기본이므로
        //     기존 동작 무변경, 온체인은 opt-in.
        // funded on-chain wallet/RPC 부재 → 내부 ledger 는 sub_wallets 실잔액 검증·차감 기반.
        // 같은 db.sqlite 에 별도 연결(WAL + busy_timeout 으로 동시 접근 안전).
        let chain_rpc = std::env::var("XGRAM_CHAIN_RPC")
            .ok()
            .filter(|u| !u.trim().is_empty());
        let payment_gateway: std::sync::Arc<dyn openxgram_marketplace::PaymentGateway> =
            match (chain_rpc, vault_password.as_ref()) {
                (Some(rpc), Some(pw)) => std::sync::Arc::new(
                    crate::onchain_gateway::OnchainPaymentGateway::open(
                        data_dir.to_path_buf(),
                        db_path(data_dir),
                        rpc,
                        pw.clone(),
                    )
                    .context("온체인 payment gateway 생성 실패")?,
                ),
                _ => std::sync::Arc::new(
                    crate::ledger_gateway::LedgerPaymentGateway::open(db_path(data_dir))
                        .context("ledger payment gateway 생성 실패")?,
                ),
            };
        // 마켓 (d)갈래 — free-tier 게이트 (무료 할당량). 결제 전에 무료 잔여를 먼저 소비 시도.
        // 무료 잔여 있으면 과금 없이 통과, 소진이면 (c)갈래 ledger 결제로. 별도 DB 연결.
        let free_quota_gate = std::sync::Arc::new(
            crate::free_tier::LedgerFreeQuotaGate::open(db_path(data_dir))
                .context("free-tier quota gate 생성 실패")?,
        );
        let marketplace_tools = std::sync::Arc::new(
            openxgram_marketplace::MarketplaceTools::new(
                mp_builder
                    .build()
                    .context("marketplace client 생성 실패")?,
                openxgram_marketplace::SpendPolicy::conservative(),
                payment_gateway,
            )
            .with_free_quota(free_quota_gate)
            // 결제 체인 — env `XGRAM_CHAIN`(기본 "base"). 테스트넷은 "ethereum-sepolia"/"base-sepolia".
            // chain.rs 레지스트리에서 chain_id·USDC 컨트랙트 매핑.
            .with_chain(std::env::var("XGRAM_CHAIN").unwrap_or_else(|_| "base".to_string())),
        );
        Ok(Self {
            db,
            data_dir: data_dir.to_path_buf(),
            vault_password,
            current_agent: None,
            acp_tools: openxgram_acp::AcpTools::new(),
            a2a_tools: openxgram_a2a::A2aTools::new(),
            marketplace_tools,
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
                description: "이 세션을 OpenXgram peer 로 등록 + 능력 명시. rc.92 D1: capabilities + description 도 함께 저장 → 다른 에이전트가 list_peers / request_help 로 너를 발견·호출.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "role": {"type": "string", "description": "역할 / 봇 alias (예: claude-code, codex, my-agent)"},
                        "machine": {"type": "string", "description": "머신 prefix (선택, 자동 hostname)"},
                        "description": {"type": "string", "description": "rc.92: 1-3 문장으로 '이 에이전트는 X 를 잘함' 설명. 다른 에이전트가 너에게 요청할지 판단 근거."},
                        "capabilities": {"type": "array", "items": {"type": "string"}, "description": "rc.92: 가능한 능력 keywords (예: ['web_search', 'code_review', 'translation']). list_peers 가 이 list 반환 → request_help 매칭 키."},
                        "ai_type": {"type": "string", "enum": ["claude", "codex", "gemini"], "description": "Phase 2: AI 종류 (동적 설정 탐지 분기). 기본 claude."},
                        "classification": {"type": "string", "enum": ["primary", "project", "special"], "description": "Phase 2: 명부 분류. primary=👑상시 / project=📁프로젝트 / special=⚙️특수. 기본 project."},
                        "execution_mode": {"type": "string", "enum": ["always", "on_demand", "heartbeat"], "description": "Phase 2: 실행모드. always=상시 / on_demand=선택 / heartbeat=깨움. 기본 on_demand."},
                        "worktree": {"type": "string", "description": "Phase 2: git worktree 경로 (선택)."},
                        "project_path": {"type": "string", "description": "에이전트 프로젝트 폴더 절대경로 (예: /home/llm/projects/starian-set). ACP 대화 cwd 로 사용 + 로스터 표시."},
                        "group_name": {"type": "string", "description": "에이전트 그룹명 (선택, 예: 배포팀)."}
                    },
                    "required": ["role"]
                }),
            },
            ToolSpec {
                name: "request_help".into(),
                description: "rc.92 D3: 특정 능력 가진 에이전트에게 자동 위임. master 가 모든 등록된 capabilities 매칭 → 가장 적합한 peer 에게 peer_send. Claude 가 'X 에 능숙한 누군가에게 부탁' 같은 상황에 호출.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "task": {"type": "string", "description": "요청할 작업 내용 (자연어). 예: '이 코드 리뷰 부탁'"},
                        "required_capability": {"type": "string", "description": "필수 능력 키워드 (예: 'code_review'). 매칭되는 peer 중 선택."},
                        "hint": {"type": "string", "description": "특정 role 지정 (선택)"}
                    },
                    "required": ["task"]
                }),
            },
            ToolSpec {
                name: "send_to_discord".into(),
                description: "Discord 채널로 메시지 push (agent-push outbound). 두 모드: \
                              (1) bot mode — channel(channel_id) 명시. discord_bots 테이블의 봇 token 사용. \
                              (2) webhook mode — webhook_url 명시 또는 vault notify.discord.webhook_url 사용. \
                              [Discord:user] inbound 받은 후 답변할 때 이 도구로 동일 채널에 자동 echo 권장.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "보낼 내용"},
                        "channel": {"type": "string", "description": "Discord channel_id (bot mode). 예: 1505791143307247678"},
                        "bot_id": {"type": "string", "description": "discord_bots.id (선택). 생략 시 첫 active 봇 자동"},
                        "webhook_url": {"type": "string", "description": "webhook mode (legacy). 생략 시 vault 기본값"}
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
            // ─── L2 위키 (5) — PRD-OpenXgram §4.1 ───
            ToolSpec {
                name: "read_wiki_page".into(),
                description: "L2 위키 페이지 읽기 — frontmatter + body. topic 은 `entity/alice` 같은 `<type>/<slug>` 형식.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "topic": {"type": "string", "description": "<type>/<slug> (예: entity/alice)"}
                    },
                    "required": ["topic"]
                }),
            },
            ToolSpec {
                name: "write_wiki_page".into(),
                description: "L2 위키 페이지 생성/업데이트. content 는 markdown 본문. page_type 미지정 시 topic 의 prefix 사용.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "topic": {"type": "string"},
                        "content": {"type": "string"},
                        "page_type": {"type": "string", "enum": ["entity", "concept", "comparison", "other"]},
                        "expected_hash": {"type": "string", "description": "낙관 잠금용 기존 content_hash (선택)"}
                    },
                    "required": ["topic", "content"]
                }),
            },
            ToolSpec {
                name: "link_concepts".into(),
                description: "두 위키 페이지 간 cross-link 추가. from→to.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "from": {"type": "string"},
                        "to": {"type": "string"},
                        "reason": {"type": "string"}
                    },
                    "required": ["from", "to"]
                }),
            },
            ToolSpec {
                name: "search_wiki".into(),
                description: "L2 위키 LIKE 검색 (k 기본 5). 벡터 검색은 후속.".into(),
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
                name: "list_wiki".into(),
                description: "L2 위키 페이지 목록. page_type 으로 필터링 가능.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "page_type": {"type": "string", "enum": ["entity", "concept", "comparison", "other"]}
                    }
                }),
            },
            // ─── 실수 레지스트리 (4) — PRD-OpenXgram §4.2 ───
            ToolSpec {
                name: "check_for_mistakes".into(),
                description: "planned_action 으로 유사 과거 실수 top-K 조회 + 경고문 생성. 행동 시작 전 호출 권장 (W 규칙 1).".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "planned_action": {"type": "string"},
                        "k": {"type": "integer", "minimum": 1, "default": 5}
                    },
                    "required": ["planned_action"]
                }),
            },
            ToolSpec {
                name: "log_mistake".into(),
                description: "실수 등록 — intended/outcome/reason/lesson + severity(1~10) + related_wiki.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "session_id": {"type": "string"},
                        "intended_action": {"type": "string"},
                        "actual_outcome": {"type": "string"},
                        "failure_reason": {"type": "string"},
                        "lesson": {"type": "string"},
                        "severity": {"type": "integer", "minimum": 1, "maximum": 10},
                        "related_wiki": {"type": "string"}
                    },
                    "required": ["session_id", "intended_action", "actual_outcome", "failure_reason", "lesson"]
                }),
            },
            ToolSpec {
                name: "find_similar_failures".into(),
                description: "situation 으로 유사 과거 실수 검색 (k 기본 5).".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "situation": {"type": "string"},
                        "k": {"type": "integer", "minimum": 1, "default": 5}
                    },
                    "required": ["situation"]
                }),
            },
            ToolSpec {
                name: "resolve_mistake".into(),
                description: "실수 해결 완료 표시 — mistake_id + 적용한 해결책.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "mistake_id": {"type": "string"},
                        "resolution": {"type": "string"}
                    },
                    "required": ["mistake_id", "resolution"]
                }),
            },
            // ─── 패턴 매칭 엔진 (4) — PRD-OpenXgram §4.3 ───
            ToolSpec {
                name: "match_action_pattern".into(),
                description: "유사 행동 패턴 top-K (현재 LIKE; 임베딩 KNN 은 후속). W 규칙 2 — 새 행동 시작 전 호출.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "new_action": {"type": "string"},
                        "k": {"type": "integer", "minimum": 1, "default": 5},
                        "min_similarity": {"type": "number", "default": 0.0}
                    },
                    "required": ["new_action"]
                }),
            },
            ToolSpec {
                name: "suggest_next_steps".into(),
                description: "current_state 와 매칭되는 패턴의 다음 단계 추천.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "current_state": {"type": "string"}
                    },
                    "required": ["current_state"]
                }),
            },
            ToolSpec {
                name: "confirm_pattern_execution".into(),
                description: "패턴 실행 확정 — modifications 가 있으면 plan 치환. 실행 자체는 외부 도구.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern_id": {"type": "string"},
                        "modifications": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "step": {"type": "string"},
                                    "tool": {"type": "string"},
                                    "args": {}
                                },
                                "required": ["step"]
                            }
                        }
                    },
                    "required": ["pattern_id"]
                }),
            },
            ToolSpec {
                name: "record_pattern_outcome".into(),
                description: "패턴 실행 결과 기록 — success + duration_ms (선택). 성공률 누적 평균 갱신.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern_id": {"type": "string"},
                        "success": {"type": "boolean"},
                        "duration_ms": {"type": "integer", "minimum": 0}
                    },
                    "required": ["pattern_id", "success"]
                }),
            },
        ];

        // peer_send — master keystore (XGRAM_KEYSTORE_PASSWORD env) 로 서명.
        // rc.205 본질 fix: vault_password 조건부 제거. peer_send 가 vault 와 무관하게 master 키만
        // 사용. 이전 조건부였던 게 sub-agent LLM 의 자율 통신 불가의 root cause —
        // MCP client list_tools 응답에서 peer_send 누락 → 도구 사용 불가.
        // 항상 노출 + handler 가 keystore unlock fail 시 runtime error.
        // rc.223 본질 fix: MCP subprocess 에 XGRAM_KEYSTORE_PASSWORD 없어도 작동.
        // primary path = daemon HTTP /v1/gui/peers/{alias}/send-unsigned (daemon 자체 unlock 상태).
        // XGRAM_MCP_TOKEN env 있으면 daemon HTTP 사용, 없으면 CLI path fallback.
        tools.push(ToolSpec {
            name: "peer_send".into(),
            description: "Fire-and-forget send. Returns immediately. Reply (if any) auto-arrives in your tmux via push notification. DO NOT poll, DO NOT loop wait — replies inject automatically. 즉시 send + return. 답장 자동 tmux push, polling 금지. (rc.223 — daemon HTTP path 사용 시 password env 불필요)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "alias": {"type": "string", "description": "받는 peer 의 alias (peers table)"},
                    "body": {"type": "string", "description": "메시지 본문"},
                    "conversation_id": {"type": "string", "description": "(선택) 대화 thread id — 미지정 시 daemon 이 자동 UUID 부여하여 reply auto-correlate"}
                },
                "required": ["alias", "body"]
            }),
        });

        // rc.151 — ack tracking. receiver 가 메시지 처리 상태 보고, sender 가 조회.
        tools.push(ToolSpec {
            name: "peer_ack".into(),
            description: "받은 메시지의 처리 상태 보고 (delivered/read/processing/done/failed).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message_id": {"type": "string"},
                    "status": {
                        "type": "string",
                        "enum": ["delivered", "read", "processing", "done", "failed"]
                    },
                    "note": {"type": "string", "description": "결과/실패 사유 (선택)"}
                },
                "required": ["message_id", "status"]
            }),
        });
        tools.push(ToolSpec {
            name: "get_message_status".into(),
            description: "보낸 메시지의 ack 상태 조회 (sender 측).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message_id": {"type": "string"}
                },
                "required": ["message_id"]
            }),
        });

        // ACP (Agent Client Protocol) tools — spawn/drive ACP agent subprocess.
        // Phase B-2. 항상 노출 (vault 무관). spawn 은 agent binary 가 설치돼야 성공.
        tools.extend([
            ToolSpec {
                name: "acp_list_agents".into(),
                description: "알려진 ACP agent adapter 목록 (claude-agent-acp, codex-acp, gemini, opencode).".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            ToolSpec {
                name: "acp_spawn".into(),
                description: "ACP agent 를 subprocess 로 spawn + initialize. handleId 반환 (이후 acp_prompt/cancel/close 에 사용).".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "agent": {"type": "string"} },
                    "required": ["agent"]
                }),
            },
            ToolSpec {
                name: "acp_prompt".into(),
                description: "spawn 된 ACP agent 에 한 prompt turn 실행. {stopReason, updates} 반환.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "handleId": {"type": "integer"},
                        "cwd": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["handleId", "cwd", "text"]
                }),
            },
            ToolSpec {
                name: "acp_cancel".into(),
                description: "ACP session 의 현재 turn 을 session/cancel notification 으로 중단.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "handleId": {"type": "integer"},
                        "sessionId": {"type": "string"}
                    },
                    "required": ["handleId", "sessionId"]
                }),
            },
            ToolSpec {
                name: "acp_close".into(),
                description: "spawn 된 ACP agent process 를 kill + reap, registry 에서 제거.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "handleId": {"type": "integer"} },
                    "required": ["handleId"]
                }),
            },
        ]);

        // A2A (Google Agent2Agent) tools — agent↔agent delegation. 항상 노출.
        // CLIENT-only: OpenXgram 이 외부/타 에이전트의 A2A endpoint 를 호출.
        // Phase 3 (ACP-A2A-CORE). callee 측 AgentCard 호스팅은 후속 작업.
        tools.extend([
            ToolSpec {
                name: "a2a_discover".into(),
                description: "외부 A2A 에이전트의 AgentCard 조회 (/.well-known/agent-card.json). url=에이전트 base URL.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "url": {"type": "string"} },
                    "required": ["url"]
                }),
            },
            ToolSpec {
                name: "a2a_send".into(),
                description: "다른 에이전트에 작업 위임 — A2A tasks/send. agentUrl + skill + params 로 Task 반환.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "agentUrl": {"type": "string"},
                        "skill": {"type": "string"},
                        "params": {"type": "object"},
                        "sessionId": {"type": "string"}
                    },
                    "required": ["agentUrl", "skill"]
                }),
            },
            ToolSpec {
                name: "a2a_get".into(),
                description: "A2A 작업 상태/결과 조회 — tasks/get. agentUrl + taskId.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "agentUrl": {"type": "string"},
                        "taskId": {"type": "string"}
                    },
                    "required": ["agentUrl", "taskId"]
                }),
            },
            ToolSpec {
                name: "a2a_cancel".into(),
                description: "A2A 작업 취소 — tasks/cancel. agentUrl + taskId.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "agentUrl": {"type": "string"},
                        "taskId": {"type": "string"}
                    },
                    "required": ["agentUrl", "taskId"]
                }),
            },
        ]);

        // Marketplace (a2a 상거래) tools — OpenAgentX 디렉토리 검색·조회. 항상 노출.
        // base_url=XGRAM_MARKETPLACE_URL|openagentx.org. 디렉토리 미배포면 실제 HTTP 에러(가짜 성공 없음).
        // 구매(purchase_service)는 실결제 게이트웨이((c)갈래) 배선 후 노출 — NoopGateway 가짜 영수증 금지.
        tools.extend([
            ToolSpec {
                name: "marketplace_search".into(),
                description: "OpenAgentX 마켓에서 타 사용자 공개 에이전트 검색. query=검색어, limit=최대개수(옵션).".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "query": {"type": "string"}, "limit": {"type": "integer"} },
                    "required": ["query"]
                }),
            },
            ToolSpec {
                name: "marketplace_get_agent".into(),
                description: "마켓 에이전트 상세 조회 — 서비스·가격·평판. agentId.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "agentId": {"type": "string"} },
                    "required": ["agentId"]
                }),
            },
            ToolSpec {
                name: "get_job_status".into(),
                description: "마켓 구매(job) 상태/결과 조회. jobId.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "jobId": {"type": "string"} },
                    "required": ["jobId"]
                }),
            },
            // 마켓 (c)갈래 — 실결제 게이트웨이(내부 ledger) 배선 완료 후 노출.
            // 가짜 영수증 아님: sub_wallets 실잔액 검증·차감 후 job 생성. 잔액 부족/한도 초과 시
            // 실제 에러 또는 NeedsConfirmation 반환.
            ToolSpec {
                name: "purchase_service".into(),
                description: "마켓 에이전트 서비스 구매(job 발주) — 내부 지갑 원장에서 실제 잔액 차감. agentId, serviceId, input(객체), maxPriceUsdcMicro(옵션). 잔액 부족 시 결제 실패(에러).".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "agentId": {"type": "string"},
                        "serviceId": {"type": "string"},
                        "input": {"type": "object", "description": "서비스 schema 에 맞는 입력"},
                        "maxPriceUsdcMicro": {"type": "integer", "description": "최대 지불 의향(microUSDC). 생략 시 서비스 정가."}
                    },
                    "required": ["agentId", "serviceId", "input"]
                }),
            },
            // 검색→연결 직결 — 마켓 에이전트를 로컬 peer 디렉토리에 등록(친구추가, idempotent).
            // 마켓 디렉토리 응답엔 서명용 pubkey/eth_address 가 없으므로(Agent 구조체에 없음)
            // 실시간 서명 메시징은 안 됨 — 그 사실을 정직히 반환(messaging_ready=false) +
            // 상호작용은 purchase_service 안내. 가짜 성공 금지.
            ToolSpec {
                name: "marketplace_connect".into(),
                description: "마켓 검색 결과의 에이전트를 로컬 peer 디렉토리에 연결(등록). idempotent — 이미 있으면 재사용. 디렉토리에 서명키가 없으면 실시간 메시징 대신 purchase_service 로 사용 안내. agentId.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "agentId": {"type": "string"} },
                    "required": ["agentId"]
                }),
            },
        ]);

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
                            "source": h.source,
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
                // rc.92 D2 — agent_capabilities LEFT JOIN 으로 description / capabilities 도 반환.
                // rc.185 — agent_capabilities 의 row 자체 도 응답 (peers 외 agent 도 보임).
                let mut caps_map: std::collections::HashMap<String, (Option<String>, Option<String>, Option<String>)> = Default::default();
                if let Ok(mut stmt) = self.db.conn().prepare(
                    "SELECT alias, role, description, capabilities FROM agent_capabilities WHERE messenger_enabled = 1 OR alias IN (SELECT alias FROM peers)"
                ) {
                    if let Ok(rows) = stmt.query_map([], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?))
                    }) {
                        for row in rows.flatten() {
                            caps_map.insert(row.0, (Some(row.1), row.2, row.3));
                        }
                    }
                }
                let peer_aliases: std::collections::HashSet<String> = peers.iter().map(|p| p.alias.clone()).collect();
                let mut items: Vec<Value> = peers
                    .iter()
                    .map(|p| {
                        let (_role, description, capabilities_json) = caps_map.get(&p.alias).cloned().unwrap_or((None, None, None));
                        let capabilities: Vec<String> = capabilities_json.as_ref()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or_default();
                        json!({
                            "alias": p.alias,
                            "public_key_hex": p.public_key_hex,
                            "address": p.address,
                            "role": p.role.as_str(),
                            "eth_address": p.eth_address,
                            "description": description,
                            "capabilities": capabilities,
                            "kind": "peer",
                        })
                    })
                    .collect();
                // rc.185: agent_capabilities 의 row 중 peers 에 없는 alias 도 추가 (kind="agent").
                for (alias, (role, description, caps_json)) in &caps_map {
                    if peer_aliases.contains(alias) { continue; }
                    let capabilities: Vec<String> = caps_json.as_ref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_default();
                    items.push(json!({
                        "alias": alias,
                        "public_key_hex": null,
                        "address": null,
                        "role": role.clone().unwrap_or_else(|| "agent".to_string()),
                        "eth_address": null,
                        "description": description,
                        "capabilities": capabilities,
                        "kind": "agent",
                    }));
                }
                let count = items.len();
                Ok(json!({"peers": items, "count": count}))
            }
            "request_help" => {
                // rc.92 D3 — capabilities 매칭 → 가장 적합한 peer 에게 peer_send 자동 위임.
                let task = args.get("task").and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("task required"))?;
                let req_cap = args.get("required_capability").and_then(|v| v.as_str());
                let hint_role = args.get("hint").and_then(|v| v.as_str());
                // 모든 capabilities 조회
                let mut candidates: Vec<(String, String, Vec<String>, Option<String>)> = Vec::new();
                if let Ok(mut stmt) = self.db.conn().prepare(
                    "SELECT alias, role, capabilities, description FROM agent_capabilities"
                ) {
                    if let Ok(rows) = stmt.query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                        ))
                    }) {
                        for row in rows.flatten() {
                            let caps: Vec<String> = row.2.as_ref()
                                .and_then(|s| serde_json::from_str(s).ok())
                                .unwrap_or_default();
                            candidates.push((row.0, row.1, caps, row.3));
                        }
                    }
                }
                // 매칭 — required_capability 우선 → hint role → 첫번째
                let mut chosen: Option<(String, String)> = None;
                if let Some(rc) = req_cap {
                    for (alias, role, caps, _) in &candidates {
                        if caps.iter().any(|c| c.eq_ignore_ascii_case(rc)) {
                            chosen = Some((alias.clone(), role.clone()));
                            break;
                        }
                    }
                }
                if chosen.is_none() {
                    if let Some(h) = hint_role {
                        for (alias, role, _, _) in &candidates {
                            if role.eq_ignore_ascii_case(h) {
                                chosen = Some((alias.clone(), role.clone()));
                                break;
                            }
                        }
                    }
                }
                if chosen.is_none() && !candidates.is_empty() {
                    let c = &candidates[0];
                    chosen = Some((c.0.clone(), c.1.clone()));
                }
                let (alias, role) = chosen.ok_or_else(|| invalid("매칭되는 peer 없음 — register_subagent 호출 안 됨"))?;

                // rc.194 본질 fix — 매칭만 하지 말고 자동 peer_send 까지 (오케스트레이션 본질).
                // rc.195 — tokio runtime 안에서 block_on 호출 시 panic. block_in_place 로 우회.
                let pw = openxgram_core::env::require_password()
                    .map_err(|e| internal(&format!("XGRAM_KEYSTORE_PASSWORD 필요 (daemon env): {e}")))?;
                let data_dir = self.data_dir.clone();
                let task_clone = task.to_string();
                let alias_clone = alias.clone();
                let send_result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async move {
                        crate::peer_send::run_peer_send(&data_dir, &alias_clone, None, &task_clone, &pw).await
                    })
                });
                let (sent, error) = match send_result {
                    Ok(()) => (true, None),
                    Err(e) => (false, Some(e.to_string())),
                };

                Ok(json!({
                    "matched_alias": alias,
                    "matched_role": role,
                    "task": task,
                    "sent": sent,
                    "error": error,
                    "candidates_count": candidates.len(),
                }))
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
                // rc.122 — group fan-out 먼저 검사. alias 가 agent_capabilities.group_name 과
                // 일치하면 그 group 의 모든 messenger_enabled 멤버에게 fan-out.
                let group_members: Vec<String> = {
                    let conn = self.db.conn();
                    conn.prepare(
                        "SELECT alias FROM agent_capabilities WHERE group_name = ?1 AND messenger_enabled = 1"
                    ).ok().and_then(|mut stmt| {
                        stmt.query_map(rusqlite::params![&alias], |r| r.get::<_, String>(0)).ok()
                            .map(|rows| rows.filter_map(|r| r.ok()).collect())
                    }).unwrap_or_default()
                };
                if !group_members.is_empty() {
                    use openxgram_manifest::InstallManifest;
                    let manifest_path = openxgram_core::paths::manifest_path(&self.data_dir);
                    let from_alias = InstallManifest::read(&manifest_path)
                        .map(|m| m.machine.alias.clone())
                        .unwrap_or_else(|_| "anon".to_string());
                    let injected = format!("[Group:{}] ⮕ {}", alias, body);
                    let handle = tokio::runtime::Handle::current();
                    let members_clone = group_members.clone();
                    let injected_clone = injected.clone();
                    // rc.286 — bare block_on → block_in_place (runtime-in-runtime panic 우회).
                    let delivered: Vec<String> = tokio::task::block_in_place(|| handle.block_on(async move {
                        let mut ok = vec![];
                        for m in members_clone {
                            if let Some((session, idx)) = crate::notify::resolve_alias_to_tmux(&m).await {
                                let target = format!("{}:{}", session, idx);
                                let wrapped = format!("\x1b[200~{}\x1b[201~", injected_clone);
                                if tokio::process::Command::new("tmux")
                                    .args(["send-keys", "-t", &target, "-l", &wrapped])
                                    .output().await.is_ok() {
                                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                                    let _ = tokio::process::Command::new("tmux")
                                        .args(["send-keys", "-t", &target, "Enter"])
                                        .output().await;
                                    ok.push(m);
                                }
                            }
                        }
                        ok
                    }));
                    return Ok(json!({
                        "sent": true,
                        "via": "group_fanout",
                        "group": alias,
                        "from": from_alias,
                        "delivered": delivered,
                        "members": group_members,
                    }));
                }
                // rc.120 — local binding session 우선 처리.
                // alias 가 본 머신 session_channel_bindings 의 agent_id 와 일치하면
                // peer transport(vault/HTTP/XMTP) 우회 + 직접 그 tmux 세션의
                // chat input 에 bracket-paste dispatch. Discord 무관 자율 대화.
                let is_local_binding = self.db.conn().query_row::<i64, _, _>(
                    "SELECT COUNT(*) FROM session_channel_bindings WHERE agent_id = ?1 AND active = 1",
                    rusqlite::params![&alias],
                    |r| r.get(0),
                ).unwrap_or(0) > 0;
                if is_local_binding {
                    // self alias (from)
                    use openxgram_manifest::InstallManifest;
                    let manifest_path = openxgram_core::paths::manifest_path(&self.data_dir);
                    let from_alias = InstallManifest::read(&manifest_path)
                        .map(|m| m.machine.alias.clone())
                        .unwrap_or_else(|_| "anon".to_string());
                    let injected = format!("[Peer:{}] ⮕ {}", from_alias, body);
                    let alias_clone = alias.clone();
                    // rc.197 — block_on → block_in_place (tokio runtime panic 우회)
                    let result = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async move {
                            let (session, idx) = crate::notify::resolve_alias_to_tmux(&alias_clone).await
                                .ok_or_else(|| format!("alias '{}' → tmux 매핑 실패", alias_clone))?;
                            let target = format!("{}:{}", session, idx);
                            let wrapped = format!("\x1b[200~{}\x1b[201~", injected);
                            let out = tokio::process::Command::new("tmux")
                                .args(["send-keys", "-t", &target, "-l", &wrapped])
                                .output().await.map_err(|e| format!("tmux paste: {}", e))?;
                            if !out.status.success() {
                                return Err(format!("tmux paste 실패: {}", String::from_utf8_lossy(&out.stderr)));
                            }
                            let out2 = tokio::process::Command::new("tmux")
                                .args(["send-keys", "-t", &target, "Enter"])
                                .output().await.map_err(|e| format!("tmux Enter: {}", e))?;
                            if !out2.status.success() {
                                return Err(format!("tmux Enter 실패: {}", String::from_utf8_lossy(&out2.stderr)));
                            }
                            Ok::<_, String>(session)
                        })
                    }).map_err(internal)?;
                    return Ok(json!({
                        "sent": true,
                        "alias": alias,
                        "via": "local_binding",
                        "tmux_session": result,
                        "from": from_alias,
                        "delivery_mode": "fire_and_forget",
                        "reply_behavior": "auto_push_to_inbox_on_arrival",
                        "next_step": "Continue your work. Reply will auto-inject when peer responds. DO NOT poll.",
                    }));
                }
                // 외부 peer (다른 머신) — primary: daemon HTTP path (rc.223).
                // rc.207 본질 fix — conversation_id 미지정 시 daemon 이 자동 UUID 부여.
                // 이로써 reply 가 auto-correlate 되고, LLM polling 시도 자체가 무의미해짐.
                let conv = args
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let data_dir = self.data_dir.clone();
                let handle = tokio::runtime::Handle::current();

                // rc.223 본질 fix — daemon HTTP path 우선.
                // MCP subprocess 가 XGRAM_KEYSTORE_PASSWORD 없어도 daemon (자기 unlock) 이 sign.
                // XGRAM_MCP_TOKEN env 있을 때만 시도. 없으면 CLI path 폴백.
                let daemon_token = std::env::var("XGRAM_MCP_TOKEN").ok();
                if let Some(token) = daemon_token.as_ref() {
                    let daemon_url = std::env::var("XGRAM_DAEMON_GUI_URL")
                        .unwrap_or_else(|_| "http://127.0.0.1:47302".to_string());
                    let url = format!(
                        "{}/v1/gui/peers/{}/send-unsigned",
                        daemon_url.trim_end_matches('/'),
                        urlencoding::encode(&alias),
                    );
                    let payload = json!({"body": body, "conversation_id": conv});
                    let token_clone = token.clone();
                    let url_clone = url.clone();
                    let payload_clone = payload.clone();
                    // rc.286 본질 fix — bare handle.block_on 은 tokio runtime thread 에서
                    // "Cannot start a runtime from within a runtime" panic → MCP 프로세스 종료
                    // ("Connection closed"). rc.195/rc.197 과 동일하게 block_in_place 로 우회.
                    let http_res: std::result::Result<(reqwest::StatusCode, String), reqwest::Error> =
                        tokio::task::block_in_place(|| {
                            handle.block_on(async move {
                                let client = reqwest::Client::new();
                                let resp = client
                                    .post(&url_clone)
                                    .header("Authorization", format!("Bearer {}", token_clone))
                                    .header("Content-Type", "application/json")
                                    .json(&payload_clone)
                                    .send()
                                    .await?;
                                let status = resp.status();
                                let text = resp.text().await.unwrap_or_default();
                                Ok((status, text))
                            })
                        });
                    match http_res {
                        Ok((status, text)) if status.is_success() => {
                            let parsed: serde_json::Value =
                                serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}));
                            return Ok(json!({
                                "sent": true,
                                "alias": alias,
                                "via": "daemon_http",
                                "msg_ulid": parsed.get("queued").cloned().unwrap_or(json!(null)),
                                "to_alias": parsed.get("to_alias").cloned().unwrap_or(json!(alias.clone())),
                                "conversation_id": conv,
                                "delivery_mode": "fire_and_forget",
                                "reply_behavior": "auto_push_to_inbox_on_arrival",
                                "next_step": "Continue your work. Reply will auto-inject when peer responds. DO NOT poll.",
                            }));
                        }
                        Ok((status, text)) => {
                            // rc.286 — silent fallback 금지(룰1). 단, vault 자격 있으면
                            // keystore CLI path 로 graceful fallback 후 그래도 실패 시 명시 error.
                            // 자격 없으면 즉시 명시 error 반환(아래 fall-through 없이).
                            if self.vault_password.is_none() {
                                return Err(internal(format!(
                                    "daemon HTTP {} from {}: {}",
                                    status, url, text
                                )));
                            }
                            eprintln!(
                                "[mcp-serve][peer_send] daemon HTTP {} — keystore CLI path 로 fallback",
                                status
                            );
                        }
                        Err(e) => {
                            if self.vault_password.is_none() {
                                return Err(internal(format!(
                                    "daemon HTTP request 실패 ({}): {}",
                                    url, e
                                )));
                            }
                            eprintln!(
                                "[mcp-serve][peer_send] daemon HTTP request 실패 ({}) — keystore CLI path 로 fallback",
                                e
                            );
                        }
                    }
                }

                // XGRAM_MCP_TOKEN 미설정(또는 token path fallback) — CLI path (XGRAM_KEYSTORE_PASSWORD 필요).
                // xgram peer send CLI 명령 backward compat 보장. 마스터 직접 호출 시 작동.
                let pw = self.require_vault()?.to_string();
                // rc.286 — bare block_on → block_in_place (runtime-in-runtime panic 우회).
                let p2p_result = tokio::task::block_in_place(|| {
                    handle.block_on(crate::peer_send::run_peer_send_with_conv(
                        &data_dir, &alias, None, &body, &pw, Some(conv.clone()),
                    ))
                });
                match p2p_result {
                    Ok(_) => Ok(json!({
                        "sent": true,
                        "alias": alias,
                        "via": "remote_peer",
                        "conversation_id": conv,
                        "delivery_mode": "fire_and_forget",
                        "reply_behavior": "auto_push_to_inbox_on_arrival",
                        "next_step": "Continue your work. Reply will auto-inject when peer responds. DO NOT poll.",
                    })),
                    Err(p2p_err) => {
                        // rc.152 — multi-transport fallback: P2P fail → Discord 봇으로 backup
                        let p2p_err_str = format!("{}", p2p_err);
                        let bot_row: rusqlite::Result<(String, String, String)> = self.db.conn().query_row(
                            "SELECT bot_token, default_channel_id, alias FROM discord_bots WHERE active=1 LIMIT 1",
                            [],
                            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
                        );
                        if let Ok((token, channel_id, bot_alias)) = bot_row {
                            let backup_body = format!("📩 [backup peer:{}] {}", alias, body);
                            let payload = json!({"content": backup_body});
                            let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);
                            // rc.286 — bare block_on → block_in_place (runtime-in-runtime panic 우회).
                            let post_res = tokio::task::block_in_place(|| handle.block_on(async {
                                let client = reqwest::Client::new();
                                client.post(&url)
                                    .header("Authorization", format!("Bot {}", token))
                                    .header("Content-Type", "application/json")
                                    .json(&payload)
                                    .send().await
                            }));
                            if let Ok(resp) = post_res {
                                if resp.status().is_success() {
                                    return Ok(json!({
                                        "sent": true, "alias": alias,
                                        "via": "discord_backup",
                                        "discord_bot": bot_alias,
                                        "p2p_error": p2p_err_str,
                                        "conversation_id": conv,
                                        "delivery_mode": "fire_and_forget",
                                        "reply_behavior": "auto_push_to_inbox_on_arrival",
                                        "next_step": "Continue your work. Reply will auto-inject when peer responds. DO NOT poll.",
                                    }));
                                }
                            }
                        }
                        // Discord backup 도 fail (또는 봇 없음) — error 반환
                        Err(internal(format!("p2p fail + discord backup unavailable: {}", p2p_err_str)))
                    }
                }
            }
            "peer_ack" => {
                let message_id = args.get("message_id").and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'message_id'"))?;
                let status = args.get("status").and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'status'"))?;
                let note = args.get("note").and_then(|v| v.as_str()).unwrap_or("");
                let allowed = ["delivered", "read", "processing", "done", "failed"];
                if !allowed.contains(&status) {
                    return Err(invalid(&format!("status must be one of {:?}", allowed)));
                }
                let now = chrono::Local::now().to_rfc3339();
                let updated = self.db.conn().execute(
                    "UPDATE messages SET ack_status=?1, acked_at=?2, ack_note=?3 WHERE id=?4",
                    rusqlite::params![status, &now, note, message_id],
                ).map_err(internal)?;
                Ok(json!({"acked": updated > 0, "message_id": message_id, "status": status, "acked_at": now}))
            }
            "get_message_status" => {
                let message_id = args.get("message_id").and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'message_id'"))?;
                let row: Result<(String, Option<String>, Option<String>, Option<String>), _> = self.db.conn().query_row(
                    "SELECT ack_status, acked_at, ack_via, ack_note FROM messages WHERE id=?1",
                    rusqlite::params![message_id],
                    |r| Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    )),
                );
                match row {
                    Ok((status, acked_at, via, note)) => Ok(json!({
                        "message_id": message_id,
                        "ack_status": status,
                        "acked_at": acked_at,
                        "ack_via": via,
                        "ack_note": note,
                    })),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Err(invalid(&format!("message_id not found: {}", message_id))),
                    Err(e) => Err(internal(e)),
                }
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
                    .list_recent(limit * 4) // filter 적용 후도 충분히 남도록 4배 fetch
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
                let token_arg = args
                    .get("bot_token")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let guild_arg = args
                    .get("guild_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let webhook_arg = args
                    .get("webhook_url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                // 1. bot_token 결정 — 인자 우선, 없으면 vault.
                let bot_token = if let Some(t) = token_arg.as_ref() {
                    t.clone()
                } else {
                    let bytes = VaultStore::new(&mut self.db)
                        .get("notify.discord.bot_token", &pw)
                        .map_err(|_| {
                            invalid(
                                "Discord bot token 미설정 — bot_token 인자로 전달. \
                             webhook 만 쓸 거면 vault 에 notify.discord.webhook_url 직접 set",
                            )
                        })?;
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
                            resp.status(),
                            resp.text().unwrap_or_default()
                        ));
                    }
                    resp.json().map_err(|e| format!("json: {e}"))
                })
                .join()
                .map_err(|_| internal("HTTP thread panic"))?
                .map_err(internal)?;

                let bot_id = bot_info
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let bot_username = bot_info
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();

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
                let token_arg = args
                    .get("bot_token")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let chat_arg = args
                    .get("chat_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let test_msg = args
                    .get("test_message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("✓ OpenXgram → Telegram 연결 테스트")
                    .to_string();

                let read_vault_or_arg = |arg: &Option<String>,
                                         key: &str,
                                         db: &mut Db|
                 -> Result<String, JsonRpcError> {
                    if let Some(v) = arg {
                        return Ok(v.clone());
                    }
                    let bytes = VaultStore::new(db).get(key, &pw).map_err(|_| {
                        invalid(&format!(
                            "{} 미설정 — 인자로 전달하거나 vault 에 미리 저장",
                            key
                        ))
                    })?;
                    String::from_utf8(bytes).map_err(|e| internal(format!("vault utf8: {e}")))
                };

                let token =
                    read_vault_or_arg(&token_arg, "notify.telegram.bot_token", &mut self.db)?;
                let chat_id =
                    read_vault_or_arg(&chat_arg, "notify.telegram.chat_id", &mut self.db)?;

                let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
                let body_str =
                    serde_json::to_string(&json!({"chat_id": chat_id, "text": test_msg})).unwrap();
                let (status_code, err_body): (u16, String) =
                    std::thread::spawn(move || -> Result<(u16, String), String> {
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
                    })
                    .join()
                    .map_err(|_| internal("HTTP thread panic"))?
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
                // rc.112 — 두 가지 모드 지원:
                //   1) bot token mode: args.channel (Discord channel_id) → discord_bots 테이블 lookup
                //      → POST /channels/{id}/messages (multibot 우선, args.bot_id 또는 첫 active 봇)
                //   2) webhook mode: args.webhook_url 또는 vault notify.discord.webhook_url (legacy)
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'content'"))?
                    .to_string();
                // bot mode 우선 — channel 인자가 있을 때
                if let Some(channel_id) = args.get("channel").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                    let bot_id_param = args.get("bot_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    // discord_bots lookup
                    let tok: Option<String> = {
                        let conn = self.db.conn();
                        let from_param = bot_id_param.as_deref().and_then(|bid| {
                            conn.query_row::<String, _, _>(
                                "SELECT bot_token FROM discord_bots WHERE id = ?1 AND active = 1",
                                rusqlite::params![bid], |r| r.get(0)
                            ).ok().filter(|t: &String| !t.is_empty())
                        });
                        from_param.or_else(|| conn.query_row::<String, _, _>(
                            "SELECT bot_token FROM discord_bots WHERE active=1 ORDER BY created_at ASC LIMIT 1",
                            [], |r| r.get(0)
                        ).ok().filter(|t: &String| !t.is_empty()))
                    };
                    let tok = tok.ok_or_else(|| invalid("Discord bot 미등록 — GUI 의 에이전트 패널 → 채널 바인딩 → '+ 봇' 으로 등록"))?;
                    let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);
                    let body_str = serde_json::to_string(&json!({"content": content})).unwrap();
                    let auth = format!("Bot {}", tok);
                    let (status, err_body): (u16, String) =
                        std::thread::spawn(move || -> Result<(u16, String), String> {
                            let resp = reqwest::blocking::Client::new()
                                .post(&url)
                                .header("authorization", auth)
                                .header("content-type", "application/json")
                                .body(body_str)
                                .send()
                                .map_err(|e| format!("Discord POST: {e}"))?;
                            let s = resp.status().as_u16();
                            let t = if (200..300).contains(&s) { String::new() } else { resp.text().unwrap_or_default() };
                            Ok((s, t))
                        }).join().map_err(|_| internal("HTTP thread panic"))?.map_err(internal)?;
                    if !(200..300).contains(&status) {
                        return Err(invalid(&format!("Discord channel HTTP {status}: {err_body}")));
                    }
                    return Ok(json!({"sent": true, "mode": "bot", "status": status, "channel": channel_id, "content_len": content.len()}));
                }
                // webhook mode (legacy)
                let pw = self.require_vault()?.to_string();
                let webhook = if let Some(w) = args.get("webhook_url").and_then(|v| v.as_str()) {
                    w.to_string()
                } else {
                    let bytes = VaultStore::new(&mut self.db)
                        .get("notify.discord.webhook_url", &pw)
                        .map_err(|_| invalid("Discord 발신 path 없음. 옵션: (1) args.channel 지정(bot mode) (2) args.webhook_url 지정 (3) vault notify.discord.webhook_url 등록"))?;
                    String::from_utf8(bytes).map_err(|e| internal(format!("vault utf8: {e}")))?
                };
                let body_str = serde_json::to_string(&json!({"content": content})).unwrap();
                let webhook_clone = webhook.clone();
                let (status, err_body): (u16, String) =
                    std::thread::spawn(move || -> Result<(u16, String), String> {
                        let resp = reqwest::blocking::Client::new()
                            .post(&webhook_clone)
                            .header("content-type", "application/json")
                            .body(body_str)
                            .send()
                            .map_err(|e| format!("Discord POST: {e}"))?;
                        let s = resp.status().as_u16();
                        let t = if (200..300).contains(&s) { String::new() } else { resp.text().unwrap_or_default() };
                        Ok((s, t))
                    }).join().map_err(|_| internal("HTTP thread panic"))?.map_err(internal)?;
                if !(200..300).contains(&status) {
                    return Err(invalid(&format!("Discord webhook HTTP {status}: {err_body}")));
                }
                Ok(json!({"sent": true, "mode": "webhook", "status": status, "content_len": content.len()}))
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
                    .map_err(|_| {
                        invalid("notify.telegram.bot_token vault 에 없음 — 먼저 connect_telegram")
                    })?;
                let token = String::from_utf8(token_bytes)
                    .map_err(|e| internal(format!("vault utf8: {e}")))?;
                let chat_id = if let Some(c) = args.get("chat_id").and_then(|v| v.as_str()) {
                    c.to_string()
                } else {
                    let bytes = VaultStore::new(&mut self.db)
                        .get("notify.telegram.chat_id", &pw)
                        .map_err(|_| invalid("notify.telegram.chat_id vault 에 없음"))?;
                    String::from_utf8(bytes).map_err(|e| internal(format!("vault utf8: {e}")))?
                };

                let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
                let body_str =
                    serde_json::to_string(&json!({"chat_id": chat_id, "text": content})).unwrap();
                let (status, err_body): (u16, String) =
                    std::thread::spawn(move || -> Result<(u16, String), String> {
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
                    })
                    .join()
                    .map_err(|_| internal("HTTP thread panic"))?
                    .map_err(internal)?;
                if !(200..300).contains(&status) {
                    return Err(invalid(&format!(
                        "Telegram API 응답 HTTP {status}: {err_body}"
                    )));
                }
                Ok(json!({"sent": true, "status": status, "content_len": content.len()}))
            }
            "register_subagent" => {
                let pw = self.require_vault()?.to_string();
                let role = args
                    .get("role")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'role'"))?;
                if !role
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
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
                // rc.92 D1 — capabilities + description 저장 (agent_capabilities 테이블)
                let description = args.get("description").and_then(|v| v.as_str());
                let capabilities = args.get("capabilities")
                    .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".into()));
                let now = chrono::Utc::now().to_rfc3339();
                // 프로젝트 폴더 + 그룹 — 에이전트가 생성 요청 시 지정 가능(GUI 추가 모달과 동일).
                let project_path = args.get("project_path").and_then(|v| v.as_str());
                let group_name = args.get("group_name").and_then(|v| v.as_str());
                let _ = self.db.conn().execute(
                    "INSERT INTO agent_capabilities (alias, role, description, capabilities, project_path, group_name, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(alias) DO UPDATE SET role=excluded.role, description=excluded.description, \
                       capabilities=excluded.capabilities, \
                       project_path=COALESCE(excluded.project_path, project_path), \
                       group_name=COALESCE(excluded.group_name, group_name), \
                       updated_at=excluded.updated_at",
                    rusqlite::params![entry.alias, role, description, capabilities, project_path, group_name, now],
                );
                // Phase 2 — 프로필 차원 upsert (agent_profiles). 미제공 시 기본값, 잘못된 enum 은 거부(rule #1).
                let ai_type = args.get("ai_type").and_then(|v| v.as_str()).unwrap_or("claude");
                let classification = args.get("classification").and_then(|v| v.as_str()).unwrap_or("project");
                let execution_mode = args.get("execution_mode").and_then(|v| v.as_str()).unwrap_or("on_demand");
                let worktree = args.get("worktree").and_then(|v| v.as_str());
                let machine_p = args.get("machine").and_then(|v| v.as_str());
                if !matches!(ai_type, "claude" | "codex" | "gemini") { return Err(invalid("ai_type 은 claude|codex|gemini")); }
                if !matches!(classification, "primary" | "project" | "special") { return Err(invalid("classification 은 primary|project|special")); }
                if !matches!(execution_mode, "always" | "on_demand" | "heartbeat") { return Err(invalid("execution_mode 은 always|on_demand|heartbeat")); }
                let _ = self.db.conn().execute(
                    "INSERT INTO agent_profiles (alias, ai_type, classification, execution_mode, machine, worktree, is_public, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7) \
                     ON CONFLICT(alias) DO UPDATE SET ai_type=excluded.ai_type, classification=excluded.classification, \
                       execution_mode=excluded.execution_mode, machine=excluded.machine, worktree=excluded.worktree, updated_at=excluded.updated_at",
                    rusqlite::params![entry.alias, ai_type, classification, execution_mode, machine_p, worktree, now],
                );
                Ok(json!({
                    "registered": true,
                    "name": entry.name,
                    "alias": entry.alias,
                    "data_dir": entry.data_dir.display().to_string(),
                    "transport_port": entry.transport_port,
                    "mcp_port": entry.transport_port + 2,
                    "description_saved": description.is_some(),
                    "capabilities_saved": capabilities.is_some(),
                    "ai_type": ai_type,
                    "classification": classification,
                    "execution_mode": execution_mode,
                }))
            }
            "install_hooks" => {
                let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("user");
                let settings_path = match scope {
                    "user" => {
                        let home = std::env::var("HOME")
                            .or_else(|_| std::env::var("USERPROFILE"))
                            .map_err(|_| internal("HOME/USERPROFILE 미설정"))?;
                        std::path::PathBuf::from(home).join(".claude/settings.json")
                    }
                    "project" => std::env::current_dir()
                        .map_err(|e| internal(e))?
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
                    h.get("hooks")
                        .and_then(|v| v.as_array())
                        .map(|hs| {
                            hs.iter().any(|sub| {
                                sub.get("command")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.contains("xgram") && s.contains("identity"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
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

                let new_content =
                    serde_json::to_string_pretty(&settings).map_err(|e| internal(e))?;
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
                let name_arg = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let subagents: Vec<String> = args
                    .get("subagents")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();

                // bot_token + guild_id 는 vault 에서.
                let bot_token = VaultStore::new(&mut self.db)
                    .get("notify.discord.bot_token", &pw)
                    .map_err(|_| invalid("notify.discord.bot_token vault 에 없음 — 먼저 connect_discord 또는 xgram setup discord"))?;
                let bot_token = String::from_utf8(bot_token)
                    .map_err(|e| internal(format!("token utf8: {e}")))?;
                let guild_id = VaultStore::new(&mut self.db)
                    .get("notify.discord.guild_id", &pw)
                    .map_err(|_| invalid("notify.discord.guild_id vault 에 없음 — vault_set 으로 추가 (Discord 서버 ID)"))?;
                let guild_id = String::from_utf8(guild_id)
                    .map_err(|e| internal(format!("guild_id utf8: {e}")))?;

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
                        .post(format!(
                            "https://discord.com/api/v10/guilds/{}/channels",
                            guild_clone
                        ))
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
                    let cat_obj: Value =
                        cat_resp.json().map_err(|e| format!("category json: {e}"))?;
                    let cat_id = cat_obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or("카테고리 id 누락")?
                        .to_string();

                    // 2. 채널들 생성 — main + sub-agents.
                    let mut channels: Vec<Value> = Vec::new();
                    let names: Vec<String> = std::iter::once("main".to_string())
                        .chain(subs.into_iter())
                        .collect();
                    for ch_name in &names {
                        let ch_resp = client
                            .post(format!(
                                "https://discord.com/api/v10/guilds/{}/channels",
                                guild_clone
                            ))
                            .header("Authorization", &auth)
                            .header("content-type", "application/json")
                            .body(
                                serde_json::to_string(&json!({
                                    "name": ch_name,
                                    "type": 0,
                                    "parent_id": cat_id,
                                }))
                                .unwrap(),
                            )
                            .send()
                            .map_err(|e| format!("channel {ch_name} POST: {e}"))?;
                        if !ch_resp.status().is_success() {
                            return Err(format!(
                                "채널 {ch_name} 생성 실패: HTTP {}",
                                ch_resp.status()
                            ));
                        }
                        let ch_obj: Value =
                            ch_resp.json().map_err(|e| format!("channel json: {e}"))?;
                        let ch_id = ch_obj
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or("채널 id 누락")?
                            .to_string();

                        // 3. webhook 발급.
                        let wh_resp = client
                            .post(format!(
                                "https://discord.com/api/v10/channels/{}/webhooks",
                                ch_id
                            ))
                            .header("Authorization", &auth)
                            .header("content-type", "application/json")
                            .body(
                                serde_json::to_string(
                                    &json!({"name": format!("openxgram-{ch_name}")}),
                                )
                                .unwrap(),
                            )
                            .send()
                            .map_err(|e| format!("webhook {ch_name} POST: {e}"))?;
                        if !wh_resp.status().is_success() {
                            return Err(format!(
                                "채널 {ch_name} webhook 발급 실패: HTTP {}",
                                wh_resp.status()
                            ));
                        }
                        let wh_obj: Value =
                            wh_resp.json().map_err(|e| format!("webhook json: {e}"))?;
                        let wh_url = wh_obj
                            .get("url")
                            .and_then(|v| v.as_str())
                            .ok_or("webhook url 누락")?
                            .to_string();

                        channels.push(
                            json!({"name": ch_name, "channel_id": ch_id, "webhook_url": wh_url}),
                        );
                    }

                    Ok(json!({"category_id": cat_id, "channels": channels}))
                })
                .join()
                .map_err(|_| internal("HTTP thread panic"))?
                .map_err(internal)?;

                // vault 에 main 채널의 webhook 저장 (기본 forward 채널).
                if let Some(main_ch) = result
                    .get("channels")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                {
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
            // ─── L2 위키 (5) ───
            "read_wiki_page" => {
                let topic = args
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'topic'"))?
                    .to_string();
                let wiki_root = self.data_dir.join("wiki");
                let fs = WikiFs::new(&wiki_root);
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = WikiTools::new(&fs, conn);
                let handle = tokio::runtime::Handle::current();
                let r = handle.block_on(tools.read(&topic)).map_err(internal)?;
                Ok(serde_json::to_value(r).map_err(internal)?)
            }
            "write_wiki_page" => {
                let topic = args
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'topic'"))?
                    .to_string();
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'content'"))?
                    .to_string();
                let page_type = args
                    .get("page_type")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let expected_hash = args
                    .get("expected_hash")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let wiki_root = self.data_dir.join("wiki");
                let fs = WikiFs::new(&wiki_root);
                let handle = tokio::runtime::Handle::current();
                handle.block_on(fs.ensure_dirs()).map_err(internal)?;
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = WikiTools::new(&fs, conn);
                let r = handle
                    .block_on(tools.write(
                        &topic,
                        &content,
                        page_type.as_deref(),
                        expected_hash.as_deref(),
                    ))
                    .map_err(internal)?;
                Ok(serde_json::to_value(r).map_err(internal)?)
            }
            "link_concepts" => {
                let from = args
                    .get("from")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'from'"))?
                    .to_string();
                let to = args
                    .get("to")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'to'"))?
                    .to_string();
                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let wiki_root = self.data_dir.join("wiki");
                let fs = WikiFs::new(&wiki_root);
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = WikiTools::new(&fs, conn);
                let handle = tokio::runtime::Handle::current();
                let r = handle
                    .block_on(tools.link(&from, &to, reason.as_deref()))
                    .map_err(internal)?;
                Ok(serde_json::to_value(r).map_err(internal)?)
            }
            "search_wiki" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'query'"))?
                    .to_string();
                let k = args.get("k").and_then(|v| v.as_u64()).map(|n| n as usize);
                let wiki_root = self.data_dir.join("wiki");
                let fs = WikiFs::new(&wiki_root);
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = WikiTools::new(&fs, conn);
                let hits = tools.search(&query, k, None).map_err(internal)?;
                let items: Vec<Value> = hits
                    .iter()
                    .map(|h| {
                        json!({
                            "id": h.id.to_string(),
                            "title": h.title,
                            "score": h.score,
                        })
                    })
                    .collect();
                Ok(json!({"hits": items, "count": items.len()}))
            }
            "list_wiki" => {
                let page_type = args
                    .get("page_type")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let wiki_root = self.data_dir.join("wiki");
                let fs = WikiFs::new(&wiki_root);
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = WikiTools::new(&fs, conn);
                let entries = tools.list(page_type.as_deref()).map_err(internal)?;
                Ok(
                    json!({"entries": serde_json::to_value(&entries).map_err(internal)?, "count": entries.len()}),
                )
            }
            // ─── 실수 레지스트리 (4) ───
            "check_for_mistakes" => {
                let planned_action = args
                    .get("planned_action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'planned_action'"))?
                    .to_string();
                let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = MistakeTools::new(conn);
                let r = tools.check(&planned_action, k).map_err(internal)?;
                Ok(serde_json::to_value(r).map_err(internal)?)
            }
            "log_mistake" => {
                let input: NewMistake = serde_json::from_value(args.clone())
                    .map_err(|e| invalid(&format!("invalid log_mistake args: {e}")))?;
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = MistakeTools::new(conn);
                let r = tools.log(input).map_err(internal)?;
                Ok(serde_json::to_value(r).map_err(internal)?)
            }
            "find_similar_failures" => {
                let situation = args
                    .get("situation")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'situation'"))?
                    .to_string();
                let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = MistakeTools::new(conn);
                let hits = tools.find_similar(&situation, k).map_err(internal)?;
                Ok(
                    json!({"hits": serde_json::to_value(&hits).map_err(internal)?, "count": hits.len()}),
                )
            }
            "resolve_mistake" => {
                let mistake_id = args
                    .get("mistake_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'mistake_id'"))?
                    .to_string();
                let resolution = args
                    .get("resolution")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'resolution'"))?
                    .to_string();
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = MistakeTools::new(conn);
                tools.resolve(&mistake_id, &resolution).map_err(internal)?;
                Ok(json!({"resolved": true, "mistake_id": mistake_id}))
            }
            // ─── 패턴 매칭 엔진 (4) ───
            "match_action_pattern" => {
                let new_action = args
                    .get("new_action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'new_action'"))?
                    .to_string();
                let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let min_similarity = args
                    .get("min_similarity")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = PatternTools::new(conn);
                let r = tools
                    .match_pattern(&new_action, k, min_similarity)
                    .map_err(internal)?;
                Ok(serde_json::to_value(r).map_err(internal)?)
            }
            "suggest_next_steps" => {
                let current_state = args
                    .get("current_state")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'current_state'"))?
                    .to_string();
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = PatternTools::new(conn);
                let suggestions = tools.suggest_next(&current_state).map_err(internal)?;
                Ok(
                    json!({"suggestions": serde_json::to_value(&suggestions).map_err(internal)?, "count": suggestions.len()}),
                )
            }
            "confirm_pattern_execution" => {
                let pattern_id = args
                    .get("pattern_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'pattern_id'"))?
                    .to_string();
                let modifications: Option<Vec<ActionStep>> = match args.get("modifications") {
                    Some(v) if !v.is_null() => Some(
                        serde_json::from_value(v.clone())
                            .map_err(|e| invalid(&format!("invalid 'modifications': {e}")))?,
                    ),
                    _ => None,
                };
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = PatternTools::new(conn);
                let r = tools
                    .confirm(&pattern_id, modifications)
                    .map_err(internal)?;
                Ok(serde_json::to_value(r).map_err(internal)?)
            }
            "record_pattern_outcome" => {
                let pattern_id = args
                    .get("pattern_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'pattern_id'"))?
                    .to_string();
                let success = args
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .ok_or_else(|| invalid("missing 'success'"))?;
                let duration_ms = args.get("duration_ms").and_then(|v| v.as_i64());
                let conn: &rusqlite::Connection = self.db.conn();
                let tools = PatternTools::new(conn);
                tools
                    .record(&pattern_id, success, duration_ms)
                    .map_err(internal)?;
                Ok(json!({"recorded": true, "pattern_id": pattern_id, "success": success}))
            }
            // ── ACP (Agent Client Protocol) tools — Phase B-2 ──────────────
            // §2.4: 모든 public ACP API 는 async; 여기 sync dispatch 에서
            // block_in_place(|| handle.block_on(...)) 로 bridge (runtime-in-runtime
            // panic 우회, rc.195/197/286 과 동일 패턴). acp_tools 는 Clone (내부
            // Arc registry) — clone 해 가져온 뒤 bridge 안에서 await.
            "acp_list_agents" => Ok(self.acp_tools.acp_list_agents()),
            "acp_spawn" => {
                let agent = args
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'agent'"))?
                    .to_string();
                let tools = self.acp_tools.clone();
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| handle.block_on(async move { tools.acp_spawn(&agent).await }))
                    .map_err(internal)
            }
            "acp_prompt" => {
                let handle_id = args
                    .get("handleId")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| invalid("missing 'handleId'"))?;
                let cwd = args
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'cwd'"))?
                    .to_string();
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'text'"))?
                    .to_string();
                let tools = self.acp_tools.clone();
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.acp_prompt(handle_id, &cwd, &text).await })
                })
                .map_err(internal)
            }
            "acp_cancel" => {
                let handle_id = args
                    .get("handleId")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| invalid("missing 'handleId'"))?;
                let session_id = args
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'sessionId'"))?
                    .to_string();
                let tools = self.acp_tools.clone();
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.acp_cancel(handle_id, &session_id).await })
                })
                .map_err(internal)
            }
            "acp_close" => {
                let handle_id = args
                    .get("handleId")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| invalid("missing 'handleId'"))?;
                let tools = self.acp_tools.clone();
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| handle.block_on(async move { tools.acp_close(handle_id).await }))
                    .map_err(internal)
            }

            // ── A2A (Google Agent2Agent) tools — Phase 3 ───────────────────
            // agent↔agent: OpenXgram 이 외부 A2A 에이전트를 호출 (client-only).
            // ACP 와 동일하게 block_in_place(|| handle.block_on(...)) bridge 로
            // sync dispatch 에서 async crate API await. a2a_tools 는 Clone.
            // 반환 struct(AgentCard/Task) → serde_json::to_value 로 Value 화.
            "a2a_discover" => {
                let url = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'url'"))?
                    .to_string();
                let tools = self.a2a_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let card = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.discover(&url).await })
                })
                .map_err(internal)?;
                serde_json::to_value(card).map_err(internal)
            }
            "a2a_send" => {
                let agent_url = args
                    .get("agentUrl")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'agentUrl'"))?
                    .to_string();
                let skill = args
                    .get("skill")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'skill'"))?
                    .to_string();
                let params = args
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let session_id = args
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let send_args = openxgram_a2a::mcp::SendTaskArgs {
                    agent_url,
                    skill,
                    params,
                    session_id,
                };
                let tools = self.a2a_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let task = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.send_task(send_args).await })
                })
                .map_err(internal)?;
                serde_json::to_value(task).map_err(internal)
            }
            "a2a_get" => {
                let agent_url = args
                    .get("agentUrl")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'agentUrl'"))?
                    .to_string();
                let task_id = args
                    .get("taskId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'taskId'"))?
                    .to_string();
                let tools = self.a2a_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let task = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.get_task(&agent_url, &task_id).await })
                })
                .map_err(internal)?;
                serde_json::to_value(task).map_err(internal)
            }
            "a2a_cancel" => {
                let agent_url = args
                    .get("agentUrl")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'agentUrl'"))?
                    .to_string();
                let task_id = args
                    .get("taskId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'taskId'"))?
                    .to_string();
                let tools = self.a2a_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let task = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.cancel_task(&agent_url, &task_id).await })
                })
                .map_err(internal)?;
                serde_json::to_value(task).map_err(internal)
            }

            // Marketplace 상거래 — block_in_place bridge (a2a 동일). marketplace_tools 는 Arc 공유.
            // 핸들러 반환(SearchResult/Agent/Job) → serde_json::to_value. 디렉토리 미배포면 HTTP 에러 그대로.
            "marketplace_search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'query'"))?
                    .to_string();
                let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as u32);
                let tools = self.marketplace_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let r = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.search(&query, limit).await })
                })
                .map_err(internal)?;
                serde_json::to_value(r).map_err(internal)
            }
            "marketplace_get_agent" => {
                let agent_id = args
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'agentId'"))?
                    .to_string();
                let tools = self.marketplace_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let r = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.get_agent(&agent_id).await })
                })
                .map_err(internal)?;
                serde_json::to_value(r).map_err(internal)
            }
            "get_job_status" => {
                let job_id = args
                    .get("jobId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'jobId'"))?
                    .to_string();
                let tools = self.marketplace_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let r = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.get_job_status(&job_id).await })
                })
                .map_err(internal)?;
                serde_json::to_value(r).map_err(internal)
            }
            // 마켓 (c)갈래 — 실결제(내부 ledger) + job 발주. block_in_place bridge (search 동일).
            // gateway 가 sub_wallets 잔액 검증·차감 → 부족하면 실제 에러(가짜 성공 없음).
            "purchase_service" => {
                let agent_id = args
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'agentId'"))?
                    .to_string();
                let service_id = args
                    .get("serviceId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'serviceId'"))?
                    .to_string();
                let input = args
                    .get("input")
                    .cloned()
                    .ok_or_else(|| invalid("missing 'input'"))?;
                let max_price = args.get("maxPriceUsdcMicro").and_then(|v| v.as_i64());
                let req = openxgram_marketplace::NewJobRequest {
                    agent_id: openxgram_marketplace::AgentId(agent_id),
                    service_id: openxgram_marketplace::ServiceId(service_id),
                    input,
                    max_price_usdc_micro: max_price,
                };
                let tools = self.marketplace_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let r = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.purchase(req).await })
                })
                .map_err(internal)?;
                serde_json::to_value(r).map_err(internal)
            }
            // 검색→연결 직결: 마켓 에이전트를 로컬 peer 디렉토리에 idempotent 등록.
            //   1) get_agent 로 디렉토리에서 실제 에이전트 정보 조회(미배포/미존재면 실제 HTTP 에러).
            //   2) PeerStore 에 등록 — alias 충돌 시 재사용(idempotent). 디렉토리엔 서명용
            //      pubkey/eth_address 가 없으므로(Agent 구조체에 필드 없음), 실시간 서명 메시징은
            //      불가. 따라서 public_key_hex 는 `mkt:<agent_id>` 결정적 placeholder(UNIQUE 충족),
            //      note 에 agent_id + 사용법(purchase_service) 기록.
            //   3) 실제로 무엇이 됐는지 정직히 반환(messaging_ready=false). 가짜 성공 금지.
            "marketplace_connect" => {
                use openxgram_peer::{PeerRole, PeerStore};
                let agent_id = args
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing 'agentId'"))?
                    .to_string();

                // 1) 디렉토리 조회 — 미배포/미존재면 실제 에러(가짜 성공 없음).
                let tools = self.marketplace_tools.clone();
                let handle = tokio::runtime::Handle::current();
                let agent_id_q = agent_id.clone();
                let agent = tokio::task::block_in_place(|| {
                    handle.block_on(async move { tools.get_agent(&agent_id_q).await })
                })
                .map_err(internal)?;

                // 2) peer 등록 (idempotent). alias 우선순위: 표시명 → 충돌/빈값이면 agent_id.
                //    placeholder pubkey 로 UNIQUE 제약 충족. address 에 agent_id 기록.
                let placeholder_pubkey = format!("mkt:{}", agent.id.as_str());
                let note = format!(
                    "marketplace agent_id={} ({}). 디렉토리에 서명키 없음 — 실시간 메시징 대신 purchase_service 로 사용.",
                    agent.id.as_str(),
                    agent.name
                );
                let mut store = PeerStore::new(&mut self.db);

                // idempotent: 동일 placeholder pubkey(=같은 마켓 에이전트)면 기존 peer 재사용.
                let existing = store
                    .get_by_public_key(&placeholder_pubkey)
                    .map_err(internal)?;
                let (peer_alias, reused) = if let Some(p) = existing {
                    (p.alias, true)
                } else {
                    // alias 충돌 회피: 표시명이 비었거나 이미 점유면 agent_id 사용.
                    let preferred = if agent.name.trim().is_empty() {
                        agent.id.as_str().to_string()
                    } else {
                        agent.name.clone()
                    };
                    let alias = match store.get_by_alias(&preferred).map_err(internal)? {
                        Some(_) => agent.id.as_str().to_string(),
                        None => preferred,
                    };
                    store
                        .add(
                            &alias,
                            &placeholder_pubkey,
                            agent.id.as_str(),
                            PeerRole::Secondary,
                            Some(&note),
                        )
                        .map_err(internal)?;
                    (alias, false)
                };

                serde_json::to_value(json!({
                    "connected": true,
                    "reused": reused,
                    "peer_alias": peer_alias,
                    "agent_id": agent.id.as_str(),
                    "agent_name": agent.name,
                    // 디렉토리에 서명키가 없어 실시간 서명 메시징은 불가 — 정직히 알림.
                    "messaging_ready": false,
                    "how_to_use": "이 마켓 에이전트와 상호작용은 purchase_service(agentId, serviceId, input) 로 서비스를 구매(job 발주)하세요. peer_send 직접 메시징은 디렉토리에 서명키가 없어 지원되지 않습니다.",
                    "note": note
                }))
                .map_err(internal)
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
