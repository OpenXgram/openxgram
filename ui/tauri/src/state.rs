//! AppState + DB 헬퍼 — 모든 invoke 핸들러가 공유하는 lazy-open DB 핸들.
//!
//! - `with_db_optional`: DB 파일 미존재 시 `None` (UI smoke 가능).
//! - `with_db_required`: DB 파일 미존재 시 명시 raise.
//! - fallback 금지 — env / home 미설정 시 `default_data_dir` 가 raise.

use std::path::PathBuf;
use std::sync::Mutex;

use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};

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

    pub fn default_data_dir() -> Result<PathBuf, String> {
        if let Ok(d) = std::env::var("XGRAM_DATA_DIR") {
            return Ok(PathBuf::from(d));
        }
        let home = dirs_home()?;
        Ok(home.join(".openxgram"))
    }
}

fn dirs_home() -> Result<PathBuf, String> {
    // Windows 는 HOME 없고 USERPROFILE 사용. CLI 의 paths::default_data_dir 와 동일 fallback.
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|e| format!("$HOME / $USERPROFILE 둘 다 미설정: {e}"))
}

pub fn with_db_optional<F, T>(state: &AppState, f: F) -> Result<Option<T>, String>
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
        db.migrate()
            .map_err(|e| format!("DB migrate 실패: {e}"))?;
        *guard = Some(db);
    }
    let db = guard.as_mut().expect("just-inserted");
    Ok(Some(f(db)?))
}

pub fn with_db_required<F, T>(state: &AppState, f: F) -> Result<T, String>
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

/// 첫 실행 여부: DB 파일 존재 ⇒ 초기화 완료.
pub fn is_data_initialized(state: &AppState) -> bool {
    db_path(&state.data_dir).exists()
}
