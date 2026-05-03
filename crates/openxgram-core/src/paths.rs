//! 데이터 디렉토리 layout — 모든 crate 가 이 한 곳에서 경로를 받는다.
//!
//! 마스터 결정: 데이터 디렉토리는 `~/.openxgram/` 고정 (CLAUDE.md 절대 규칙).
//! 새 경로·파일명을 다른 crate 가 직접 만들지 않는다 — 여기에 add 후 import.

use std::path::{Path, PathBuf};

use crate::{CoreError, Result};

pub const APP_DIR_NAME: &str = ".openxgram";
pub const KEYSTORE_SUBDIR: &str = "keystore";
pub const BACKUP_SUBDIR: &str = "backup";
pub const FAILED_SUBDIR: &str = "failed";

pub const DB_FILENAME: &str = "db.sqlite";
pub const MANIFEST_FILENAME: &str = "install-manifest.json";
pub const MASTER_KEY_NAME: &str = "master";
pub const MASTER_KEYFILE: &str = "master.json";

/// 기본 데이터 디렉토리 (`$HOME/.openxgram`). HOME 미설정 시 raise.
pub fn default_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| CoreError::NoHome)?;
    Ok(PathBuf::from(home).join(APP_DIR_NAME))
}

pub fn manifest_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MANIFEST_FILENAME)
}

pub fn keystore_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(KEYSTORE_SUBDIR)
}

pub fn master_keyfile(data_dir: &Path) -> PathBuf {
    keystore_dir(data_dir).join(MASTER_KEYFILE)
}

pub fn backup_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(BACKUP_SUBDIR)
}

pub fn failed_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(FAILED_SUBDIR)
}

pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DB_FILENAME)
}

/// init 이 생성하는 디렉토리 목록 (data_dir + keystore + backup).
pub fn install_dirs(data_dir: &Path) -> [PathBuf; 3] {
    [
        data_dir.to_path_buf(),
        keystore_dir(data_dir),
        backup_dir(data_dir),
    ]
}
