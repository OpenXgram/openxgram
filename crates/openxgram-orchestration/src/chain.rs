//! 메시지 체인 — 순차 단계 + 조건 분기 (응답 파싱, delay).
//!
//! YAML/JSON 정의 → DB 저장 → `ChainRunner::run` 실행.

use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::scheduled::{kst_now_epoch, TargetKind};
use crate::{OrchestrationError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionKind {
    Always,
    ResponseContains,
    ResponseNotContains,
}

impl ConditionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::ResponseContains => "response_contains",
            Self::ResponseNotContains => "response_not_contains",
        }
    }
}

impl FromStr for ConditionKind {
    type Err = OrchestrationError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "always" => Ok(Self::Always),
            "response_contains" => Ok(Self::ResponseContains),
            "response_not_contains" => Ok(Self::ResponseNotContains),
            other => Err(OrchestrationError::Send(format!(
                "unknown condition_kind: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageChain {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at_kst: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ChainStep {
    pub id: String,
    pub chain_id: String,
    pub step_order: i64,
    pub target_kind: TargetKind,
    pub target: String,
    pub payload: String,
    pub delay_secs: i64,
    pub condition_kind: Option<ConditionKind>,
    pub condition_value: Option<String>,
}

/// YAML 정의 step (사용자 친화적 형식).
#[derive(Debug, Clone, Deserialize)]
pub struct ChainStepInput {
    /// `to_role: master` 또는 `to_platform: discord` 둘 중 하나.
    #[serde(default)]
    pub to_role: Option<String>,
    #[serde(default)]
    pub to_platform: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub delay_secs: i64,
    #[serde(default)]
    pub condition_kind: Option<String>,
    #[serde(default)]
    pub condition_value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChainDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub steps: Vec<ChainStepInput>,
}

impl ChainStepInput {
    /// (target_kind, target) 추출. role 우선.
    pub fn resolve_target(&self) -> Result<(TargetKind, String)> {
        if let Some(role) = &self.to_role {
            return Ok((TargetKind::Role, role.clone()));
        }
        if let Some(platform) = &self.to_platform {
            let channel = self.channel_id.as_deref().ok_or_else(|| {
                OrchestrationError::Send(format!(
                    "platform `{platform}` step requires channel_id"
                ))
            })?;
            return Ok((TargetKind::Platform, format!("{platform}:{channel}")));
        }
        Err(OrchestrationError::Send(
            "step requires either to_role or to_platform".into(),
        ))
    }
}

pub struct ChainStore<'a> {
    conn: &'a Connection,
}

impl<'a> ChainStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn create(&self, def: &ChainDefinition) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = kst_now_epoch();
        self.conn.execute(
            "INSERT INTO message_chains (id, name, description, created_at_kst, enabled) \
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![id, def.name, def.description, now],
        )?;
        for (idx, step) in def.steps.iter().enumerate() {
            let (tk, target) = step.resolve_target()?;
            let cond_kind = step
                .condition_kind
                .as_deref()
                .map(ConditionKind::from_str)
                .transpose()?;
            self.conn.execute(
                "INSERT INTO chain_steps \
                 (id, chain_id, step_order, target_kind, target, payload, delay_secs, \
                  condition_kind, condition_value) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    Uuid::new_v4().to_string(),
                    id,
                    idx as i64,
                    tk.as_str(),
                    target,
                    step.text,
                    step.delay_secs,
                    cond_kind.map(|c| c.as_str()),
                    step.condition_value
                ],
            )?;
        }
        Ok(id)
    }

    pub fn list(&self) -> Result<Vec<MessageChain>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, created_at_kst, enabled \
             FROM message_chains ORDER BY created_at_kst",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(MessageChain {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at_kst: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
            })
        })?;
        rows.map(|r| r.map_err(OrchestrationError::from))
            .collect::<Result<Vec<_>>>()
    }

    pub fn get_by_name(&self, name: &str) -> Result<(MessageChain, Vec<ChainStep>)> {
        let chain = self
            .conn
            .query_row(
                "SELECT id, name, description, created_at_kst, enabled \
                 FROM message_chains WHERE name=?1",
                params![name],
                |row| {
                    Ok(MessageChain {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        created_at_kst: row.get(3)?,
                        enabled: row.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    OrchestrationError::ChainNotFound(name.to_string())
                }
                other => OrchestrationError::Db(other),
            })?;
        let steps = self.list_steps(&chain.id)?;
        Ok((chain, steps))
    }

    pub fn list_steps(&self, chain_id: &str) -> Result<Vec<ChainStep>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chain_id, step_order, target_kind, target, payload, delay_secs, \
             condition_kind, condition_value \
             FROM chain_steps WHERE chain_id=?1 ORDER BY step_order",
        )?;
        let rows = stmt.query_map(params![chain_id], |row| {
            let tk_str: String = row.get(3)?;
            let cond_str: Option<String> = row.get(7)?;
            Ok(ChainStep {
                id: row.get(0)?,
                chain_id: row.get(1)?,
                step_order: row.get(2)?,
                target_kind: tk_str.parse().map_err(|e: OrchestrationError| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                target: row.get(4)?,
                payload: row.get(5)?,
                delay_secs: row.get(6)?,
                condition_kind: cond_str
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|e: OrchestrationError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                condition_value: row.get(8)?,
            })
        })?;
        rows.map(|r| r.map_err(OrchestrationError::from))
            .collect::<Result<Vec<_>>>()
    }

    pub fn delete_by_name(&self, name: &str) -> Result<()> {
        let affected = self.conn.execute(
            "DELETE FROM message_chains WHERE name=?1",
            params![name],
        )?;
        if affected == 0 {
            return Err(OrchestrationError::ChainNotFound(name.to_string()));
        }
        Ok(())
    }
}

/// 채널 송신 추상화. 채널 embed PR 의 `RouteEngine` 가 이 trait 를 구현하면
/// 자동으로 orchestration 이 그것을 사용한다.
#[async_trait]
pub trait ChannelSender: Send + Sync {
    async fn send_to_role(&self, role: &str, payload: &str) -> Result<String>;
    async fn send_to_platform(
        &self,
        platform: &str,
        channel_id: &str,
        text: &str,
    ) -> Result<String>;
}

/// 테스트/dry-run 용 — 모든 호출에 빈 응답을 돌려준다.
pub struct NoopSender;

#[async_trait]
impl ChannelSender for NoopSender {
    async fn send_to_role(&self, _role: &str, _payload: &str) -> Result<String> {
        Ok(String::new())
    }
    async fn send_to_platform(
        &self,
        _platform: &str,
        _channel_id: &str,
        _text: &str,
    ) -> Result<String> {
        Ok(String::new())
    }
}

#[derive(Debug, Clone)]
pub struct ChainStepResult {
    pub step_order: i64,
    pub executed: bool,
    pub response: String,
    pub error: Option<String>,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChainRunResult {
    pub chain_name: String,
    pub steps: Vec<ChainStepResult>,
    pub failed: bool,
}

pub struct ChainRunner;

impl ChainRunner {
    /// 체인을 순차 실행.
    /// - 각 step 의 `delay_secs` 만큼 대기 (직전 단계 후)
    /// - 직전 step 의 응답으로 condition 평가, 미충족 시 skip
    /// - silent fallback 금지 — send 실패 시 step 단위 error 기록 + 후속 step 도 skip 처리
    pub async fn run(
        steps: &[ChainStep],
        sender: &dyn ChannelSender,
        chain_name: &str,
    ) -> ChainRunResult {
        let mut results: Vec<ChainStepResult> = Vec::with_capacity(steps.len());
        let mut last_response = String::new();
        let mut failed = false;

        for step in steps {
            // 1) condition 평가 (직전 응답 기준)
            if let Some(cond) = step.condition_kind {
                let value = step.condition_value.as_deref().unwrap_or("");
                let ok = match cond {
                    ConditionKind::Always => true,
                    ConditionKind::ResponseContains => last_response.contains(value),
                    ConditionKind::ResponseNotContains => !last_response.contains(value),
                };
                if !ok {
                    results.push(ChainStepResult {
                        step_order: step.step_order,
                        executed: false,
                        response: String::new(),
                        error: None,
                        skipped_reason: Some(format!(
                            "condition `{}` not met (value=`{}`)",
                            cond.as_str(),
                            value
                        )),
                    });
                    continue;
                }
            }

            // 2) delay
            if step.delay_secs > 0 {
                tokio::time::sleep(Duration::from_secs(step.delay_secs as u64)).await;
            }

            // 3) send
            let send_result = match step.target_kind {
                TargetKind::Role => sender.send_to_role(&step.target, &step.payload).await,
                TargetKind::Platform => {
                    // target = "platform:channel_id"
                    let (platform, channel) = match step.target.split_once(':') {
                        Some((p, c)) => (p, c),
                        None => {
                            results.push(ChainStepResult {
                                step_order: step.step_order,
                                executed: false,
                                response: String::new(),
                                error: Some(format!(
                                    "platform target malformed: {} (expected platform:channel)",
                                    step.target
                                )),
                                skipped_reason: None,
                            });
                            failed = true;
                            break;
                        }
                    };
                    sender
                        .send_to_platform(platform, channel, &step.payload)
                        .await
                }
            };
            match send_result {
                Ok(resp) => {
                    last_response = resp.clone();
                    results.push(ChainStepResult {
                        step_order: step.step_order,
                        executed: true,
                        response: resp,
                        error: None,
                        skipped_reason: None,
                    });
                }
                Err(e) => {
                    results.push(ChainStepResult {
                        step_order: step.step_order,
                        executed: false,
                        response: String::new(),
                        error: Some(e.to_string()),
                        skipped_reason: None,
                    });
                    failed = true;
                    break;
                }
            }
        }

        ChainRunResult {
            chain_name: chain_name.to_string(),
            steps: results,
            failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openxgram_db::{Db, DbConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn open_db() -> Db {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let cfg = DbConfig {
            path: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let mut db = Db::open(cfg).unwrap();
        db.migrate().unwrap();
        std::mem::forget(tmp);
        db
    }

    struct CountingSender {
        count: Arc<AtomicUsize>,
        response: String,
    }

    #[async_trait]
    impl ChannelSender for CountingSender {
        async fn send_to_role(&self, _role: &str, _payload: &str) -> Result<String> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
        async fn send_to_platform(
            &self,
            _platform: &str,
            _channel_id: &str,
            _text: &str,
        ) -> Result<String> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    #[test]
    fn create_and_get_chain() {
        let mut db = open_db();
        let store = ChainStore::new(db.conn());
        let yaml = r#"
name: morning
description: morning routine
steps:
  - to_role: master
    text: "오늘 일정?"
  - to_role: res
    text: "뉴스 요약"
    delay_secs: 0
"#;
        let def: ChainDefinition = serde_yaml::from_str(yaml).unwrap();
        let _id = store.create(&def).unwrap();
        let (chain, steps) = store.get_by_name("morning").unwrap();
        assert_eq!(chain.name, "morning");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].target, "master");
        assert_eq!(steps[1].target, "res");
    }

    #[tokio::test]
    async fn run_chain_executes_all_steps() {
        let mut db = open_db();
        let store = ChainStore::new(db.conn());
        let def: ChainDefinition = serde_yaml::from_str(
            r#"
name: r1
steps:
  - to_role: a
    text: hi
  - to_role: b
    text: hi
  - to_role: c
    text: hi
"#,
        )
        .unwrap();
        store.create(&def).unwrap();
        let (chain, steps) = store.get_by_name("r1").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let sender = CountingSender {
            count: count.clone(),
            response: "ok".to_string(),
        };
        let result = ChainRunner::run(&steps, &sender, &chain.name).await;
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert!(!result.failed);
        assert_eq!(result.steps.len(), 3);
    }

    #[tokio::test]
    async fn condition_response_contains_skips() {
        let mut db = open_db();
        let store = ChainStore::new(db.conn());
        let def: ChainDefinition = serde_yaml::from_str(
            r#"
name: c1
steps:
  - to_role: a
    text: hi
  - to_role: b
    text: hi
    condition_kind: response_contains
    condition_value: "no-match"
"#,
        )
        .unwrap();
        store.create(&def).unwrap();
        let (chain, steps) = store.get_by_name("c1").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let sender = CountingSender {
            count: count.clone(),
            response: "ok".to_string(),
        };
        let result = ChainRunner::run(&steps, &sender, &chain.name).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(!result.steps[1].executed);
    }
}
