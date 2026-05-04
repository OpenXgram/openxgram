//! OpenXgram Desktop — Tauri 2.x library entry.
//!
//! 본 crate 는 두 가지를 제공한다:
//!   1. 기존 subprocess wrapper (`xgram doctor/version/dump …`) — get_status / get_version / dump
//!   2. **store API 직접 호출** invoke 핸들러 — frontend (.tsx) 가 호출하는 9개 명령
//!      (vault_pending_*, memory_search, peers_list, peer_add, vault_get,
//!      payment_get_daily_limit, payment_set_daily_limit)
//!
//! 핸들러 정책:
//!   - DB 는 lazy-open, AppState 의 `Mutex<Option<Db>>` 로 공유.
//!   - DB 파일 미존재 시 raise 하지 않고 **빈 결과** 반환 (UI smoke 가능).
//!     쓰기 명령(peer_add, payment_set_daily_limit) 은 명시 raise.
//!   - 비밀번호 필요 명령(vault_get) 은 `XGRAM_KEYSTORE_PASSWORD` 환경변수 사용 — fallback 없이 raise.
//!
//! Payment daily limit 매핑:
//!   payment 전용 store 는 daily_limit 개념이 없다 (PRD §16 baseline).
//!   따라서 vault ACL 의 `key_pattern="payment.usdc.transfer"`, `agent="default"`
//!   row 의 `daily_limit` 컬럼을 microUSDC 단위로 사용. (의미: 마스터가 default agent 에게
//!   하루에 허용한 USDC payment intent 건수의 micro 환산. ACL 정합 — 별도 store 미생성.)

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::{
    default_embedder, MemoryKind, MemoryStore, MessageStore, TraitStore,
};
use openxgram_peer::{PeerRole, PeerStore};
use openxgram_vault::{AclAction, AclPolicy, VaultStore};

// ───────────────────────────── subprocess wrappers (legacy) ──────────────────

#[derive(Serialize)]
pub struct StatusResult {
    success: bool,
    output: String,
}

#[tauri::command]
fn get_status() -> StatusResult {
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
fn get_version() -> StatusResult {
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
fn dump(kind: String) -> StatusResult {
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

// ───────────────────────────── AppState (DB 핸들 공유) ───────────────────────

/// AppState — DB 는 lazy-open. 첫 호출 시 ~/.openxgram/openxgram.db 를 연다.
/// 파일 미존재 시 호출자에게 raise (단, 빈 결과 반환 핸들러는 None 으로 흡수).
pub struct AppState {
    pub data_dir: PathBuf,
    pub db: Mutex<Option<Db>>,
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            db: Mutex::new(None),
        }
    }

    /// 기본 데이터 디렉토리: $XGRAM_DATA_DIR > ~/.openxgram. fallback 금지 — env/home 둘 다 실패 시 raise.
    pub fn default_data_dir() -> Result<PathBuf, String> {
        if let Ok(d) = std::env::var("XGRAM_DATA_DIR") {
            return Ok(PathBuf::from(d));
        }
        let home = dirs_home()?;
        Ok(home.join(".openxgram"))
    }
}

fn dirs_home() -> Result<PathBuf, String> {
    // env $HOME 만 사용 — `dirs` crate 회피 (deps 최소).
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|e| format!("$HOME 미설정: {e}"))
}

/// data_dir 의 DB 가 존재하면 open 후 callback 실행. 미존재 시 None.
fn with_db_optional<F, T>(state: &AppState, f: F) -> Result<Option<T>, String>
where
    F: FnOnce(&mut Db) -> Result<T, String>,
{
    let path = db_path(&state.data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let mut guard = state.db.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
    if guard.is_none() {
        let mut db = Db::open(DbConfig {
            path: path.clone(),
            ..Default::default()
        })
        .map_err(|e| format!("DB open 실패 ({}): {e}", path.display()))?;
        // 기존 DB 라도 누락 마이그레이션 적용 — schema 정합성 확보.
        db.migrate()
            .map_err(|e| format!("DB migrate 실패: {e}"))?;
        *guard = Some(db);
    }
    let db = guard.as_mut().expect("just-inserted");
    Ok(Some(f(db)?))
}

/// DB 가 반드시 있어야 하는 쓰기 명령용 — 미존재 시 명시 raise.
fn with_db_required<F, T>(state: &AppState, f: F) -> Result<T, String>
where
    F: FnOnce(&mut Db) -> Result<T, String>,
{
    match with_db_optional(state, f)? {
        Some(t) => Ok(t),
        None => Err(format!(
            "DB 파일 미존재 ({}). `xgram init --alias <NAME>` 먼저 실행.",
            db_path(&state.data_dir).display()
        )),
    }
}

// ───────────────────────────── invoke handlers — vault pending ───────────────

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
fn vault_pending_list(state: State<'_, AppState>) -> Result<Vec<PendingDto>, String> {
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
fn vault_pending_approve(state: State<'_, AppState>, id: String) -> Result<(), String> {
    with_db_required(&state, |db| {
        let mut store = VaultStore::new(db);
        store
            .approve_confirmation(&id)
            .map_err(|e| format!("approve_confirmation: {e}"))
    })
}

#[tauri::command]
fn vault_pending_deny(
    state: State<'_, AppState>,
    id: String,
    reason: Option<String>,
) -> Result<(), String> {
    let _ = reason; // Phase 2: 거부 사유는 store 에 컬럼 추가 후 기록. 현재는 무시.
    with_db_required(&state, |db| {
        let mut store = VaultStore::new(db);
        store
            .deny_confirmation(&id)
            .map_err(|e| format!("deny_confirmation: {e}"))
    })
}

// ───────────────────────────── invoke handler — memory_search ────────────────

#[derive(Serialize, Clone)]
pub struct HitDto {
    pub id: String,
    pub layer: String,
    pub body: String,
    pub score: f64,
}

#[tauri::command]
fn memory_search(
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
            // embedder 는 기본 dummy (XGRAM_EMBEDDER 또는 fastembed feature 시 BGE-small).
            let embedder = default_embedder().map_err(|e| format!("embedder init: {e}"))?;
            let mut store = MessageStore::new(db, embedder.as_ref());
            // recall_top_k 는 임베딩 기반 — 빈 결과는 정상.
            match store.recall_top_k(&q, 10) {
                Ok(rows) => {
                    for r in rows {
                        // sqlite-vec 의 distance(f32) 는 작을수록 유사 — score = 1 / (1 + distance) 로 변환 후 f64 캐스트.
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
                    // 임베더 미가용(fastembed feature off)은 silent skip — fallback 금지 정책 예외:
                    // L0 만 비활성화하고 L2/L4 계속.
                    tracing_log_optional(&format!("memory_search L0 skip: {e}"));
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

        // 점수 내림차순.
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

fn tracing_log_optional(msg: &str) {
    // Phase 2 baseline: stderr 로 단순 출력. tracing crate 통합은 후속.
    eprintln!("[openxgram-desktop] {msg}");
}

// ───────────────────────────── invoke handlers — peers ───────────────────────

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
fn peers_list(state: State<'_, AppState>) -> Result<Vec<PeerDto>, String> {
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

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct PeerAddForm {
    alias: String,
    address: String,
    pubkey: String,
    machine: Option<String>,
}

#[tauri::command]
fn peer_add(
    state: State<'_, AppState>,
    alias: String,
    address: String,
    pubkey: String,
    machine: Option<String>,
) -> Result<PeerDto, String> {
    if alias.trim().is_empty() || address.trim().is_empty() || pubkey.trim().is_empty() {
        return Err("alias/address/pubkey 필수".into());
    }
    with_db_required(&state, |db| {
        let mut store = PeerStore::new(db);
        // machine 은 메모란에 저장 — peer schema 에 별도 컬럼 없음. role 은 Worker 기본 (UI peer add 는 보조 머신/에이전트 가정).
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

// ───────────────────────────── invoke handler — vault_get ────────────────────

/// vault_get — plaintext 반환. password 는 `XGRAM_KEYSTORE_PASSWORD` env 사용.
/// 실 운용에서는 ephemeral token 기반 단발 reveal 로 대체 예정 (PRD-VAULT-MFA).
#[tauri::command]
fn vault_get(state: State<'_, AppState>, key: String) -> Result<String, String> {
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

// ───────────────────────────── invoke handlers — payment limits ──────────────

const PAYMENT_LIMIT_KEY_PATTERN: &str = "payment.usdc.transfer";
const PAYMENT_LIMIT_AGENT: &str = "default";

/// payment_get_daily_limit — vault_acl 에서 (payment.usdc.transfer, default) row 의 daily_limit 반환.
/// row 미존재 → 0 (의도: "한도 미설정 = 결제 차단").
#[tauri::command]
fn payment_get_daily_limit(state: State<'_, AppState>) -> Result<i64, String> {
    let out: Option<i64> = with_db_optional(&state, |db| {
        let mut store = VaultStore::new(db);
        let rows = store.list_acl().map_err(|e| format!("list_acl: {e}"))?;
        let limit = rows
            .iter()
            .find(|a| a.key_pattern == PAYMENT_LIMIT_KEY_PATTERN && a.agent == PAYMENT_LIMIT_AGENT)
            .map(|a| a.daily_limit)
            .unwrap_or(0);
        Ok(limit)
    })?;
    Ok(out.unwrap_or(0))
}

/// payment_set_daily_limit — vault_acl 에 microUSDC 한도 upsert.
/// frontend 는 `microUsdc` (number) 로 전달. i64 변환 후 저장.
#[tauri::command]
fn payment_set_daily_limit(state: State<'_, AppState>, micro_usdc: i64) -> Result<(), String> {
    if micro_usdc < 0 {
        return Err("micro_usdc 음수 불가".into());
    }
    with_db_required(&state, |db| {
        let mut store = VaultStore::new(db);
        store
            .upsert_acl(
                PAYMENT_LIMIT_KEY_PATTERN,
                PAYMENT_LIMIT_AGENT,
                &[AclAction::Get, AclAction::Set],
                micro_usdc,
                AclPolicy::Confirm,
            )
            .map_err(|e| format!("upsert_acl: {e}"))?;
        Ok(())
    })
}

// ───────────────────────────── Tauri builder ─────────────────────────────────

pub fn run() {
    let data_dir = AppState::default_data_dir().unwrap_or_else(|e| {
        eprintln!("[openxgram-desktop] data_dir 결정 실패: {e} — /tmp/openxgram-fallback 사용 (DB 미존재 → 빈 결과)");
        Path::new("/tmp/openxgram-fallback").to_path_buf()
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::new(data_dir))
        .invoke_handler(tauri::generate_handler![
            // legacy subprocess wrappers
            get_status,
            get_version,
            dump,
            // store-direct handlers
            vault_pending_list,
            vault_pending_approve,
            vault_pending_deny,
            memory_search,
            peers_list,
            peer_add,
            vault_get,
            payment_get_daily_limit,
            payment_set_daily_limit,
        ])
        .run(tauri::generate_context!())
        .expect("OpenXgram desktop 실행 실패");
}

// ───────────────────────────── unit tests (DB-free) ──────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appstate_with_db_optional_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::new(tmp.path().to_path_buf());
        // 핸들러 함수는 #[tauri::command] 라 직접 호출 어렵다.
        // with_db_optional 은 pub(crate) — 빈 데이터 디렉토리에서 None 반환 검증.
        let result: Result<Option<bool>, String> = with_db_optional(&state, |_db| Ok(true));
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn appstate_with_db_required_raises_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = AppState::new(tmp.path().to_path_buf());
        let result: Result<bool, String> = with_db_required(&state, |_db| Ok(true));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("DB 파일 미존재"));
    }
}
