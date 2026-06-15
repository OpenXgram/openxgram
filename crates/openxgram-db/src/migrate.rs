use crate::error::DbError;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone)]
pub struct MigrationRecord {
    pub version: u32,
    pub name: String,
    pub applied_at: String,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "message_embeddings",
        sql: include_str!("../migrations/0002_message_embeddings.sql"),
    },
    Migration {
        version: 3,
        name: "episodes",
        sql: include_str!("../migrations/0003_episodes.sql"),
    },
    Migration {
        version: 4,
        name: "patterns",
        sql: include_str!("../migrations/0004_patterns.sql"),
    },
    Migration {
        version: 5,
        name: "traits",
        sql: include_str!("../migrations/0005_traits.sql"),
    },
    Migration {
        version: 6,
        name: "vault",
        sql: include_str!("../migrations/0006_vault.sql"),
    },
    Migration {
        version: 7,
        name: "vault_acl",
        sql: include_str!("../migrations/0007_vault_acl.sql"),
    },
    Migration {
        version: 8,
        name: "vault_pending",
        sql: include_str!("../migrations/0008_vault_pending.sql"),
    },
    Migration {
        version: 9,
        name: "mcp_tokens",
        sql: include_str!("../migrations/0009_mcp_tokens.sql"),
    },
    Migration {
        version: 10,
        name: "peers",
        sql: include_str!("../migrations/0010_peers.sql"),
    },
    Migration {
        version: 11,
        name: "payment_intents",
        sql: include_str!("../migrations/0011_payment_intents.sql"),
    },
    Migration {
        version: 12,
        name: "peer_eth_address",
        sql: include_str!("../migrations/0012_peer_eth_address.sql"),
    },
    Migration {
        version: 13,
        name: "audit_chain",
        sql: include_str!("../migrations/0013_audit_chain.sql"),
    },
    Migration {
        version: 14,
        name: "kek_rotations",
        sql: include_str!("../migrations/0014_kek_rotations.sql"),
    },
    Migration {
        version: 15,
        name: "payment_daily_limits",
        sql: include_str!("../migrations/0015_payment_daily_limits.sql"),
    },
    Migration {
        version: 16,
        name: "orchestration",
        sql: include_str!("../migrations/0016_orchestration.sql"),
    },
    Migration {
        version: 17,
        name: "conversation_id",
        sql: include_str!("../migrations/0017_conversation_id.sql"),
    },
    Migration {
        version: 18,
        name: "wiki_pages",
        sql: include_str!("../migrations/0018_wiki_pages.sql"),
    },
    Migration {
        version: 19,
        name: "mistakes",
        sql: include_str!("../migrations/0019_mistakes.sql"),
    },
    Migration {
        version: 20,
        name: "action_patterns",
        sql: include_str!("../migrations/0020_action_patterns.sql"),
    },
    // v22 users — 폐기 (PRD §1: 1 사람 = 1 daemon. multi-user X).
    // 이미 적용된 DB는 그대로 두되 신규 설치에서는 생성하지 않음.
    Migration {
        version: 21,
        name: "sub_wallets",
        sql: include_str!("../migrations/0021_sub_wallets.sql"),
    },
    Migration {
        version: 23,
        name: "messenger_full",
        sql: include_str!("../migrations/0022_messenger_full.sql"),
    },
    Migration {
        version: 24,
        name: "messenger_attachments",
        sql: include_str!("../migrations/0023_messenger_attachments.sql"),
    },
    Migration {
        version: 25,
        name: "memory_full",
        sql: include_str!("../migrations/0024_memory_full.sql"),
    },
    Migration {
        version: 26,
        name: "session_channel_bindings",
        sql: include_str!("../migrations/0025_session_channel_bindings.sql"),
    },
    Migration {
        version: 27,
        name: "full_specs",
        sql: include_str!("../migrations/0026_full_specs.sql"),
    },
    Migration {
        version: 28,
        name: "external_agent",
        sql: include_str!("../migrations/0028_external_agent.sql"),
    },
    Migration {
        version: 29,
        name: "workflows",
        sql: include_str!("../migrations/0029_workflows.sql"),
    },
    Migration {
        version: 30,
        name: "identity_settings",
        sql: include_str!("../migrations/0030_identity_settings.sql"),
    },
    Migration {
        version: 31,
        name: "role_policies",
        sql: include_str!("../migrations/0031_role_policies.sql"),
    },
    Migration {
        version: 32,
        name: "session_aliases",
        sql: include_str!("../migrations/0032_session_aliases.sql"),
    },
    Migration {
        version: 33,
        name: "claude_ingest",
        sql: include_str!("../migrations/0033_claude_ingest.sql"),
    },
    Migration {
        version: 34,
        name: "discord_bots",
        sql: include_str!("../migrations/0034_discord_bots.sql"),
    },
    Migration {
        version: 35,
        name: "agent_capabilities",
        sql: include_str!("../migrations/0035_agent_capabilities.sql"),
    },
    // rc.196 — 36/37/38 잘못된 SKIP 가정 fix. 새 install (zalman 등) 에서 안 적용되어
    // 'no column named group_name' error 로 messenger 등록 fail 한 본질 결함.
    // runner 가 'duplicate column' 은 graceful skip (이미 수동 ALTER 된 server-seoul case).
    Migration {
        version: 36,
        name: "messenger_group",
        sql: include_str!("../migrations/0036_messenger_group.sql"),
    },
    Migration {
        version: 37,
        name: "agent_orchestration",
        sql: include_str!("../migrations/0037_agent_orchestration.sql"),
    },
    Migration {
        version: 38,
        name: "agent_templates",
        sql: include_str!("../migrations/0038_agent_templates.sql"),
    },
    Migration {
        version: 39,
        name: "memory_embeddings",
        sql: include_str!("../migrations/0039_memory_embeddings.sql"),
    },
    Migration {
        version: 40,
        name: "message_ack",
        sql: include_str!("../migrations/0040_message_ack.sql"),
    },
    Migration {
        version: 41,
        name: "peer_gui_address",
        sql: include_str!("../migrations/0041_peer_gui_address.sql"),
    },
    Migration {
        version: 42,
        name: "session_binding_echo_state",
        sql: include_str!("../migrations/0042_session_binding_echo_state.sql"),
    },
    Migration {
        version: 43,
        name: "binding_proj_name",
        sql: include_str!("../migrations/0043_binding_proj_name.sql"),
    },
    Migration {
        version: 44,
        name: "outbound_queue_ack",
        sql: include_str!("../migrations/0044_outbound_queue_ack.sql"),
    },
    Migration {
        version: 45,
        name: "app_ack",
        sql: include_str!("../migrations/0045_app_ack.sql"),
    },
    Migration {
        version: 46,
        name: "peer_session_identifier",
        sql: include_str!("../migrations/0046_peer_session_identifier.sql"),
    },
    // rc.276 — Paperclip orchestration absorption, Phase 1 (core entities / schema).
    // companies, agent_capabilities org overlay, goals/projects/project_goals,
    // issues (+checkout lock), issue_relations, activity_log.
    Migration {
        version: 47,
        name: "paperclip_orchestration",
        sql: include_str!("../migrations/0047_paperclip_orchestration.sql"),
    },
    // Phase 2-D — 카카오톡 셸 GUI: 에이전트 프로필 (classification/execution_mode/ai_type/worktree/public).
    Migration {
        version: 48,
        name: "agent_profiles",
        sql: include_str!("../migrations/0048_agent_profiles.sql"),
    },
    // ACP 대화 영속화 — 새로고침/재시작 후 대화 기록·복원.
    Migration {
        version: 49,
        name: "acp_messages",
        sql: include_str!("../migrations/0049_acp_messages.sql"),
    },
    // 에이전트 대화명(표시 이름) — 로스터/헤더에 alias 대신.
    Migration {
        version: 50,
        name: "agent_display_name",
        sql: include_str!("../migrations/0050_agent_display_name.sql"),
    },
    // ACP 대화 읽음 상태 — 안읽음 배지/정렬.
    Migration {
        version: 51,
        name: "acp_read",
        sql: include_str!("../migrations/0051_acp_read.sql"),
    },
    // 기본 동봉(built-in) 특수에이전트 source/activated — xgram-ops 설치·활성화.
    Migration {
        version: 52,
        name: "agent_source_activated",
        sql: include_str!("../migrations/0052_agent_source_activated.sql"),
    },
    // 에이전트별 컴포저 설정 영속 — perm_mode/model/thinking.
    Migration {
        version: 53,
        name: "agent_composer_settings",
        sql: include_str!("../migrations/0053_agent_composer_settings.sql"),
    },
    // LLM 위키 Phase 1 — 페이지 간 [[wikilink]] 연결/backlink.
    Migration {
        version: 54,
        name: "wiki_links",
        sql: include_str!("../migrations/0054_wiki_links.sql"),
    },
    // 마켓 (c)갈래 — 지갑 거래 원장 (topup/purchase/earn 감사 추적).
    Migration {
        version: 55,
        name: "wallet_ledger",
        sql: include_str!("../migrations/0055_wallet_ledger.sql"),
    },
    // 마켓 (d)갈래 — free-tier 요금제 게이팅 (무료 할당량 config + per-agent per-day 사용량).
    Migration {
        version: 56,
        name: "free_tier",
        sql: include_str!("../migrations/0056_free_tier.sql"),
    },
    // 런타임 하네스 — 큐레이션된 주입 항목(규칙·원칙) 리스트 + 기본 시드 2개.
    Migration {
        version: 57,
        name: "injection_rules",
        sql: include_str!("../migrations/0057_injection_rules.sql"),
    },
    // rc.321 — 친구 단위 POLICY (권한/격리/비용). agent_profiles 정책 컬럼 + friend_cost_ledger.
    Migration {
        version: 58,
        name: "friend_policy",
        sql: include_str!("../migrations/0058_friend_policy.sql"),
    },
    // rc.330 — 방(대화) 단위 설정 저장 (GUI P3). 하네스·역할·오케스트레이션·
    // 시스템 프롬프트·이벤트 규칙을 JSON 컬럼으로 방별 보관. 저장만, 강제는 P4.
    Migration {
        version: 59,
        name: "room_config",
        sql: include_str!("../migrations/0059_room_config.sql"),
    },
    // rc.332 — 오케스트레이션 RUN 상태 (GUI P4c). 방의 orchestration_json 단계를
    // 데몬이 순서대로 실행하는 runner 의 진행 상태(current_step/status/steps_json) 영속.
    Migration {
        version: 60,
        name: "orchestration_run",
        sql: include_str!("../migrations/0060_orchestration_run.sql"),
    },
    // rc.333 — 방(대화) 동적 멤버십 (GUI P5). 방의 활성 참가자 목록(초대/내보내기).
    // 1:1 방은 row 없음 → 멤버십 gate 통과(무회귀). 그룹 방은 active 참가자만 전달/턴 대상.
    Migration {
        version: 61,
        name: "room_participants",
        sql: include_str!("../migrations/0061_room_participants.sql"),
    },
    // rc.334 — 방(대화) 단위 공유 보안 스코프 (GUI P6: 보안 공유방).
    // 멤버만 복호화·열람, 비멤버 차단. 실제 비밀 본문은 vault crate(vault_entries)가 암호화 보관 —
    // 이 테이블은 METADATA(이름표·kind·sensitive·vault_key·created_by)만. 평문 비밀 없음.
    // 퇴장 시 키회전은 자동 X — room_vault_rotation_flag 로 marker 만(사람 결정).
    Migration {
        version: 62,
        name: "room_vault",
        sql: include_str!("../migrations/0062_room_vault.sql"),
    },
];

pub struct MigrationRunner<'a> {
    conn: &'a mut rusqlite::Connection,
}

impl<'a> MigrationRunner<'a> {
    pub fn new(conn: &'a mut rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub fn run_all(&mut self) -> Result<(), DbError> {
        // schema_migrations 테이블 보장
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            )",
            [],
        )?;

        for m in MIGRATIONS {
            let already: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                [m.version],
                |r| r.get(0),
            )?;
            if already {
                tracing::debug!(
                    version = m.version,
                    name = m.name,
                    "migration already applied, skipping"
                );
                continue;
            }

            tracing::info!(version = m.version, name = m.name, "applying migration");

            let tx = self.conn.transaction()?;
            // rc.196: ALTER ADD COLUMN 가 이미 수동 적용된 경우 (server-seoul 36-38 case)
            // 'duplicate column name' 으로 fail. graceful skip — schema_migrations 만 등록.
            match tx.execute_batch(m.sql) {
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("duplicate column name")
                        || msg.contains("already exists")
                    {
                        tracing::info!(
                            version = m.version,
                            name = m.name,
                            error = %msg,
                            "migration partial (column/object already exists) — schema_migrations 만 mark"
                        );
                    } else {
                        return Err(DbError::Migration {
                            version: m.version,
                            reason: msg,
                        });
                    }
                }
            }

            let now = chrono::Local::now().to_rfc3339();
            let affected = tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![m.version, m.name, now],
            )?;

            // silent error 방지 — affected_rows 검증
            if affected != 1 {
                return Err(DbError::UnexpectedRowCount {
                    expected: 1,
                    actual: affected as u64,
                });
            }
            tx.commit()?;
        }
        Ok(())
    }
}
