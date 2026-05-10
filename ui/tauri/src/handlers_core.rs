//! 기존 9개 invoke 핸들러 + `is_initialized` — vault/peer/memory/payment 직접 호출.

use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::State;

use openxgram_memory::{default_embedder, MemoryKind, MemoryStore, MessageStore, TraitStore};
use openxgram_payment::DailyLimitStore;
use openxgram_peer::{PeerRole, PeerStore};
use openxgram_vault::VaultStore;

use crate::state::{is_data_initialized, with_db_optional, with_db_required, AppState};

// ── subprocess wrappers (legacy) ─────────────────────────────────────────────

#[derive(Serialize)]
pub struct StatusResult {
    success: bool,
    output: String,
}

#[tauri::command]
pub fn get_status() -> StatusResult {
    match Command::new("xgram").args(["doctor", "--json"]).output() {
        Ok(out) => StatusResult {
            success: out.status.success(),
            output: String::from_utf8_lossy(&out.stdout).into_owned(),
        },
        Err(e) => StatusResult {
            success: false,
            output: format!("xgram 실행 실패: {e}\n\n`xgram` 이 PATH 에 있는지 확인."),
        },
    }
}

#[tauri::command]
pub fn get_version() -> StatusResult {
    match Command::new("xgram").args(["version", "--json"]).output() {
        Ok(out) => StatusResult {
            success: out.status.success(),
            output: String::from_utf8_lossy(&out.stdout).into_owned(),
        },
        Err(e) => StatusResult {
            success: false,
            output: format!("xgram 실행 실패: {e}"),
        },
    }
}

#[tauri::command]
pub fn dump(kind: String) -> StatusResult {
    match Command::new("xgram").args(["dump", &kind]).output() {
        Ok(out) => StatusResult {
            success: out.status.success(),
            output: if out.status.success() {
                String::from_utf8_lossy(&out.stdout).into_owned()
            } else {
                String::from_utf8_lossy(&out.stderr).into_owned()
            },
        },
        Err(e) => StatusResult {
            success: false,
            output: format!("xgram 실행 실패: {e}"),
        },
    }
}

// ── onboarding ──────────────────────────────────────────────────────────────

/// `is_initialized` — DB 파일 존재 여부. Onboarding 분기에 사용.
///
/// 원격 daemon 모드 (`XGRAM_DAEMON_URL` 설정) 면 daemon `/v1/gui/initialized` 호출.
/// 로컬 모드면 기존대로 manifest 파일 존재 검사.
#[tauri::command]
pub async fn is_initialized(state: State<'_, AppState>) -> Result<bool, String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        return client.initialized().await;
    }
    Ok(is_data_initialized(&state))
}

// ── vault pending ───────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct PendingDto {
    pub id: String,
    pub key: String,
    pub agent: String,
    pub action: String,
    pub status: String,
    pub requested_at: String,
}

#[tauri::command]
pub async fn vault_pending_list(state: State<'_, AppState>) -> Result<Vec<PendingDto>, String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        let r = client.vault_pending_list().await?;
        return Ok(r
            .into_iter()
            .map(|p| PendingDto {
                id: p.id,
                key: p.key,
                agent: p.agent,
                action: p.action,
                status: p.status,
                requested_at: p.requested_at,
            })
            .collect());
    }
    let out: Option<Vec<PendingDto>> = with_db_optional(&state, |db| {
        let mut store = VaultStore::new(db);
        let rows = store
            .list_pending()
            .map_err(|e| format!("list_pending: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|p| PendingDto {
                id: p.id,
                key: p.key,
                agent: p.agent,
                action: p.action.as_str().to_string(),
                status: p.status.as_str().to_string(),
                requested_at: p.requested_at.to_rfc3339(),
            })
            .collect())
    })?;
    Ok(out.unwrap_or_default())
}

#[tauri::command]
pub async fn vault_pending_approve(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        return client.vault_pending_approve(&id).await;
    }
    with_db_required(&state, |db| {
        let mut store = VaultStore::new(db);
        store
            .approve_confirmation(&id)
            .map_err(|e| format!("approve_confirmation: {e}"))
    })
}

#[tauri::command]
pub async fn vault_pending_deny(
    state: State<'_, AppState>,
    id: String,
    reason: Option<String>,
) -> Result<(), String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        return client.vault_pending_deny(&id, reason).await;
    }
    let _ = reason; // Phase 2: 거부 사유 컬럼 추가 후 기록.
    with_db_required(&state, |db| {
        let mut store = VaultStore::new(db);
        store
            .deny_confirmation(&id)
            .map_err(|e| format!("deny_confirmation: {e}"))
    })
}

// ── memory_search ───────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct HitDto {
    pub id: String,
    pub layer: String,
    pub body: String,
    pub score: f64,
}

#[tauri::command]
pub fn memory_search(
    state: State<'_, AppState>,
    query: String,
    layers: Vec<String>,
) -> Result<Vec<HitDto>, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let want_l0 = layers.iter().any(|l| l == "L0");
    let want_l2 = layers.iter().any(|l| l == "L2");
    let want_l4 = layers.iter().any(|l| l == "L4");

    let out: Option<Vec<HitDto>> = with_db_optional(&state, |db| {
        let mut hits: Vec<HitDto> = Vec::new();

        if want_l0 {
            let embedder = default_embedder().map_err(|e| format!("embedder init: {e}"))?;
            let mut store = MessageStore::new(db, embedder.as_ref());
            match store.recall_top_k(&q, 10) {
                Ok(rows) => {
                    for r in rows {
                        let score = 1.0_f64 / (1.0_f64 + r.distance as f64);
                        hits.push(HitDto {
                            id: r.message.id.clone(),
                            layer: "L0".into(),
                            body: r.message.body.clone(),
                            score,
                        });
                    }
                }
                Err(e) => {
                    eprintln!("[openxgram-desktop] memory_search L0 skip: {e}");
                }
            }
        }
        if want_l2 {
            let mut store = MemoryStore::new(db);
            for kind in [
                MemoryKind::Fact,
                MemoryKind::Decision,
                MemoryKind::Reference,
                MemoryKind::Rule,
            ] {
                let rows = store
                    .list_by_kind(kind)
                    .map_err(|e| format!("memory list_by_kind: {e}"))?;
                for m in rows {
                    if m.content.contains(&q) {
                        hits.push(HitDto {
                            id: m.id.clone(),
                            layer: "L2".into(),
                            body: m.content.clone(),
                            score: 0.5,
                        });
                    }
                }
            }
        }
        if want_l4 {
            let mut store = TraitStore::new(db);
            let rows = store.list().map_err(|e| format!("trait list: {e}"))?;
            for t in rows {
                let body = format!("{}: {}", t.name, t.value);
                if body.contains(&q) {
                    hits.push(HitDto {
                        id: t.id.clone(),
                        layer: "L4".into(),
                        body,
                        score: 0.5,
                    });
                }
            }
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(20);
        Ok(hits)
    })?;
    Ok(out.unwrap_or_default())
}

// ── peers ───────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct PeerDto {
    pub id: String,
    pub alias: String,
    pub address: String,
    pub public_key_hex: String,
    pub role: String,
    pub created_at: String,
    pub last_seen: Option<String>,
}

#[tauri::command]
pub async fn peers_list(state: State<'_, AppState>) -> Result<Vec<PeerDto>, String> {
    // 원격 daemon 모드: HTTP 클라이언트로 위임 (XGRAM_DAEMON_URL 설정 시).
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        let remote = client.peers().await?;
        return Ok(remote
            .into_iter()
            .map(|p| PeerDto {
                id: p.id,
                alias: p.alias,
                address: p.address,
                public_key_hex: p.public_key_hex,
                role: p.role,
                created_at: p.created_at,
                last_seen: p.last_seen,
            })
            .collect());
    }
    // 로컬 모드 — 기존대로 lib 직접 호출.
    let out: Option<Vec<PeerDto>> = with_db_optional(&state, |db| {
        let mut store = PeerStore::new(db);
        let rows = store.list().map_err(|e| format!("peer list: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|p| PeerDto {
                id: p.id,
                alias: p.alias,
                address: p.address,
                public_key_hex: p.public_key_hex,
                role: p.role.as_str().to_string(),
                created_at: p.created_at.to_rfc3339(),
                last_seen: p.last_seen.map(|t| t.to_rfc3339()),
            })
            .collect())
    })?;
    Ok(out.unwrap_or_default())
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct PeerAddForm {
    pub alias: String,
    pub address: String,
    pub pubkey: String,
    pub machine: Option<String>,
}

#[tauri::command]
pub async fn peer_add(
    state: State<'_, AppState>,
    alias: String,
    address: String,
    pubkey: String,
    machine: Option<String>,
) -> Result<PeerDto, String> {
    if alias.trim().is_empty() || address.trim().is_empty() || pubkey.trim().is_empty() {
        return Err("alias/address/pubkey 필수".into());
    }
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        let r = client
            .peer_add(&crate::daemon_client::PeerAddBody {
                alias: alias.clone(),
                address: address.clone(),
                public_key_hex: pubkey.clone(),
                notes: machine.clone(),
            })
            .await?;
        return Ok(PeerDto {
            id: r.id,
            alias: r.alias,
            address: r.address,
            public_key_hex: r.public_key_hex,
            role: r.role,
            created_at: r.created_at,
            last_seen: r.last_seen,
        });
    }
    with_db_required(&state, |db| {
        let mut store = PeerStore::new(db);
        let p = store
            .add(
                &alias,
                &pubkey,
                &address,
                PeerRole::Worker,
                machine.as_deref(),
            )
            .map_err(|e| format!("peer add: {e}"))?;
        Ok(PeerDto {
            id: p.id,
            alias: p.alias,
            address: p.address,
            public_key_hex: p.public_key_hex,
            role: p.role.as_str().to_string(),
            created_at: p.created_at.to_rfc3339(),
            last_seen: p.last_seen.map(|t| t.to_rfc3339()),
        })
    })
}

// ── vault_get ───────────────────────────────────────────────────────────────

#[tauri::command]
pub fn vault_get(state: State<'_, AppState>, key: String) -> Result<String, String> {
    let password = std::env::var("XGRAM_KEYSTORE_PASSWORD")
        .map_err(|_| "XGRAM_KEYSTORE_PASSWORD 미설정 — vault reveal 불가".to_string())?;
    with_db_required(&state, |db| {
        let mut store = VaultStore::new(db);
        let bytes = store
            .get(&key, &password)
            .map_err(|e| format!("vault get: {e}"))?;
        String::from_utf8(bytes).map_err(|e| format!("vault value utf8 변환 실패: {e}"))
    })
}

// ── payment limits ──────────────────────────────────────────────────────────

const PAYMENT_LIMIT_AGENT: &str = "default";
const PAYMENT_LIMIT_CHAIN: &str = "base";

#[tauri::command]
pub async fn payment_get_daily_limit(state: State<'_, AppState>) -> Result<i64, String> {
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        return client.payment_get_daily_limit().await;
    }
    let out: Option<i64> = with_db_optional(&state, |db| {
        let mut store = DailyLimitStore::new(db);
        let row = store
            .get(PAYMENT_LIMIT_AGENT, PAYMENT_LIMIT_CHAIN)
            .map_err(|e| format!("daily_limit get: {e}"))?;
        Ok(row.map(|r| r.daily_micro).unwrap_or(0))
    })?;
    Ok(out.unwrap_or(0))
}

#[tauri::command]
pub async fn payment_set_daily_limit(
    state: State<'_, AppState>,
    micro_usdc: i64,
) -> Result<(), String> {
    if micro_usdc < 0 {
        return Err("micro_usdc 음수 불가".into());
    }
    if let Some(client) = crate::daemon_client::DaemonClient::from_env() {
        return client.payment_set_daily_limit(micro_usdc).await;
    }
    with_db_required(&state, |db| {
        let mut store = DailyLimitStore::new(db);
        store
            .set(PAYMENT_LIMIT_AGENT, PAYMENT_LIMIT_CHAIN, micro_usdc)
            .map_err(|e| format!("daily_limit set: {e}"))?;
        Ok(())
    })
}

// ── peer_send — Messenger 송신 (v0.2-β) ─────────────────────────────────────

#[tauri::command]
pub async fn peer_send(
    state: State<'_, AppState>,
    alias: String,
    body: String,
) -> Result<(), String> {
    let password = std::env::var("XGRAM_KEYSTORE_PASSWORD").map_err(|_| {
        "XGRAM_KEYSTORE_PASSWORD 미설정 — keystore 패스워드를 환경변수로 export 후 \
         xgram gui 재실행. 예: export XGRAM_KEYSTORE_PASSWORD='...'"
            .to_string()
    })?;
    openxgram_cli::peer_send::run_peer_send(&state.data_dir, &alias, None, &body, &password)
        .await
        .map_err(|e| format!("peer_send: {e}"))
}

// ── messages_recent — Messenger 활동 흐름 모니터링 ──────────────────────────

#[derive(Serialize, Clone)]
pub struct MessageDto {
    pub id: String,
    pub session_id: String,
    pub sender: String,
    pub body: String,
    pub timestamp: String,
    pub conversation_id: String,
}

#[tauri::command]
pub fn messages_recent(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<MessageDto>, String> {
    let lim = limit.unwrap_or(50).min(500);
    let out: Option<Vec<MessageDto>> = with_db_optional(&state, |db| {
        let embedder = default_embedder().map_err(|e| format!("embedder init: {e}"))?;
        let mut store = MessageStore::new(db, embedder.as_ref());
        let msgs = store
            .list_recent(lim)
            .map_err(|e| format!("list_recent: {e}"))?;
        Ok(msgs
            .into_iter()
            .map(|m| MessageDto {
                id: m.id,
                session_id: m.session_id,
                sender: m.sender,
                body: m.body,
                timestamp: m.timestamp.to_rfc3339(),
                conversation_id: m.conversation_id,
            })
            .collect())
    })?;
    Ok(out.unwrap_or_default())
}
