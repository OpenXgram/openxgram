//! Chain invoke 핸들러 — `openxgram-orchestration::ChainStore` 직접 호출.
//!
//! 사용자는 YAML 텍스트를 붙여넣어 체인을 만든다. backend 가 `serde_yaml`
//! 으로 파싱하고 ChainStore::create 에 전달.
//!
//! `chain_run` 은 NoopSender 로 dry-run — 실제 송신은 다른 PR (channel embed) 후속.

use serde::Serialize;
use tauri::State;

use openxgram_orchestration::{ChainDefinition, ChainRunner, ChainStore, NoopSender};

use crate::state::{with_db_optional, with_db_required, AppState};

#[derive(Serialize, Clone)]
pub struct ChainDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at_kst: i64,
    pub enabled: bool,
    pub step_count: usize,
}

#[derive(Serialize, Clone)]
pub struct ChainStepDto {
    pub step_order: i64,
    pub target_kind: String,
    pub target: String,
    pub payload: String,
    pub delay_secs: i64,
    pub condition_kind: Option<String>,
    pub condition_value: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ChainDetailDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at_kst: i64,
    pub enabled: bool,
    pub steps: Vec<ChainStepDto>,
}

#[derive(Serialize, Clone)]
pub struct ChainStepRunDto {
    pub step_order: i64,
    pub executed: bool,
    pub response: String,
    pub error: Option<String>,
    pub skipped_reason: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct ChainRunDto {
    pub chain_name: String,
    pub failed: bool,
    pub steps: Vec<ChainStepRunDto>,
}

#[tauri::command]
pub async fn chain_list(state: State<'_, AppState>) -> Result<Vec<ChainDto>, String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        let r = client.chain_list().await?;
        return Ok(r
            .into_iter()
            .map(|c| ChainDto {
                id: c.id,
                name: c.name,
                description: c.description,
                created_at_kst: c.created_at_kst,
                enabled: c.enabled,
                step_count: c.step_count,
            })
            .collect());
    }
    let out: Option<Vec<ChainDto>> = with_db_optional(&state, |db| {
        let store = ChainStore::new(db.conn());
        let chains = store.list().map_err(|e| format!("chain list: {e}"))?;
        let mut out = Vec::with_capacity(chains.len());
        for c in chains {
            let steps = store
                .list_steps(&c.id)
                .map_err(|e| format!("chain list_steps: {e}"))?;
            out.push(ChainDto {
                id: c.id,
                name: c.name,
                description: c.description,
                created_at_kst: c.created_at_kst,
                enabled: c.enabled,
                step_count: steps.len(),
            });
        }
        Ok(out)
    })?;
    Ok(out.unwrap_or_default())
}

#[tauri::command]
pub async fn chain_show(
    state: State<'_, AppState>,
    name: String,
) -> Result<ChainDetailDto, String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        let r = client.chain_show(&name).await?;
        return Ok(ChainDetailDto {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at_kst: r.created_at_kst,
            enabled: r.enabled,
            steps: r
                .steps
                .into_iter()
                .map(|s| ChainStepDto {
                    step_order: s.step_order,
                    target_kind: s.target_kind,
                    target: s.target,
                    payload: s.payload,
                    delay_secs: s.delay_secs,
                    condition_kind: s.condition_kind,
                    condition_value: s.condition_value,
                })
                .collect(),
        });
    }
    with_db_required(&state, |db| {
        let store = ChainStore::new(db.conn());
        let (chain, steps) = store
            .get_by_name(&name)
            .map_err(|e| format!("chain get_by_name: {e}"))?;
        Ok(ChainDetailDto {
            id: chain.id,
            name: chain.name,
            description: chain.description,
            created_at_kst: chain.created_at_kst,
            enabled: chain.enabled,
            steps: steps
                .into_iter()
                .map(|s| ChainStepDto {
                    step_order: s.step_order,
                    target_kind: s.target_kind.as_str().to_string(),
                    target: s.target,
                    payload: s.payload,
                    delay_secs: s.delay_secs,
                    condition_kind: s.condition_kind.map(|c| c.as_str().to_string()),
                    condition_value: s.condition_value,
                })
                .collect(),
        })
    })
}

/// YAML 텍스트로 체인 생성.
#[tauri::command]
pub fn chain_create_yaml(state: State<'_, AppState>, yaml: String) -> Result<String, String> {
    if yaml.trim().is_empty() {
        return Err("yaml 비어있음".into());
    }
    let def: ChainDefinition = serde_yaml::from_str(&yaml)
        .map_err(|e| format!("YAML 파싱 실패: {e}"))?;
    if def.name.trim().is_empty() {
        return Err("chain.name 비어있음".into());
    }
    if def.steps.is_empty() {
        return Err("chain.steps 비어있음".into());
    }
    with_db_required(&state, |db| {
        let store = ChainStore::new(db.conn());
        store
            .create(&def)
            .map_err(|e| format!("chain create: {e}"))
    })
}

#[tauri::command]
pub async fn chain_delete(state: State<'_, AppState>, name: String) -> Result<(), String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        return client.chain_delete(&name).await;
    }
    with_db_required(&state, |db| {
        let store = ChainStore::new(db.conn());
        store
            .delete_by_name(&name)
            .map_err(|e| format!("chain delete: {e}"))
    })
}

/// dry-run 실행 — NoopSender 로 단계별 진행만 시뮬레이트.
/// 실제 송신은 channel embed PR 후속.
#[tauri::command]
pub async fn chain_run(state: State<'_, AppState>, name: String) -> Result<ChainRunDto, String> {
    // tokio runtime 안에서 실행 — Tauri command async 지원.
    let chain_steps = with_db_required(&state, |db| {
        let store = ChainStore::new(db.conn());
        let (_chain, steps) = store
            .get_by_name(&name)
            .map_err(|e| format!("chain get_by_name: {e}"))?;
        Ok(steps)
    })?;

    let sender = NoopSender;
    let result = ChainRunner::run(&chain_steps, &sender, &name).await;

    Ok(ChainRunDto {
        chain_name: result.chain_name,
        failed: result.failed,
        steps: result
            .steps
            .into_iter()
            .map(|s| ChainStepRunDto {
                step_order: s.step_order,
                executed: s.executed,
                response: s.response,
                error: s.error,
                skipped_reason: s.skipped_reason,
            })
            .collect(),
    })
}
