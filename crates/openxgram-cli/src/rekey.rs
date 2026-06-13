//! xgram rekey — keystore + vault 비밀번호 변경 (rekey).
//!
//! 보안 핵심: keystore 의 식별 서명키 + vault 의 모든 자격증명을 old → new 비번으로
//! 재암호화한다. 봉투(envelope/DEK) 구조가 없어 각 항목이 비번에서 직접 파생되므로
//! 모든 항목을 개별 재암호화해야 한다.
//!
//! 순서:
//!   (a) BACKUP — keystore 디렉토리 + db.sqlite* 를 rekey-backup-<ts>/ 로 복사
//!   (b) keystore reencrypt_all(old, new)
//!   (c) vault reencrypt_all(old, new)
//!   (d) daemon.env 의 XGRAM_KEYSTORE_PASSWORD 줄 갱신 (없으면 0600 으로 생성)
//!   (e) VERIFY — keystore.load(first, new) + vault.get(first, new) 성공 확인
//!
//! 어느 단계든 실패 시 backup 경로를 포함한 에러를 raise (수동 복구 안내).
//! 모든 타임스탬프는 KST 기준이지만 backup 디렉토리명은 unix epoch 사용.
//! 비밀번호 값은 로그/출력에 평문 노출 금지.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use openxgram_core::paths::{db_path, keystore_dir};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_vault::VaultStore;

/// keystore + vault 를 old → new 비번으로 rekey 한다.
pub fn run_rekey(data_dir: &Path, old: &str, new: &str) -> Result<()> {
    if old == new {
        bail!("새 비밀번호가 현재 비밀번호와 동일합니다");
    }

    // (a) BACKUP — 되돌릴 수 있어야 자동 진행 (개발원칙 #2 롤백 가능).
    let backup_dir = backup(data_dir).context("rekey backup 실패 — 재암호화 미실행")?;
    println!("[rekey] backup 생성: {}", backup_dir.display());

    // (b) keystore 재암호화.
    let ks_dir = keystore_dir(data_dir);
    let ks = FsKeystore::new(&ks_dir);
    let ks_count = ks
        .reencrypt_all(old, new)
        .map_err(|e| with_backup_hint(&backup_dir, format!("keystore 재암호화 실패: {e}")))?;
    println!("[rekey] keystore 재암호화: {ks_count} 개 keyfile");

    // (c) vault 재암호화.
    let vault_count = {
        let mut db = open_db(data_dir)
            .map_err(|e| with_backup_hint(&backup_dir, format!("DB open 실패: {e}")))?;
        let mut store = VaultStore::new(&mut db);
        store
            .reencrypt_all(old, new)
            .map_err(|e| with_backup_hint(&backup_dir, format!("vault 재암호화 실패: {e}")))?
    };
    println!("[rekey] vault 재암호화: {vault_count} 개 항목");

    // (d) daemon.env 갱신 (없으면 0600 생성).
    update_daemon_env(data_dir, new)
        .map_err(|e| with_backup_hint(&backup_dir, format!("daemon.env 갱신 실패: {e}")))?;
    println!("[rekey] daemon.env 갱신 완료");

    // (e) VERIFY — 새 비번으로 keystore.load + vault.get 성공해야 한다.
    verify(data_dir, new, ks_count, vault_count)
        .map_err(|e| with_backup_hint(&backup_dir, format!("검증 실패: {e}")))?;
    println!("[rekey] 검증 통과 — 새 비밀번호로 keystore/vault 복호화 확인");

    Ok(())
}

/// keystore 디렉토리 + db.sqlite* 를 rekey-backup-<unix_ts>/ 로 복사.
fn backup(data_dir: &Path) -> Result<PathBuf> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_dir = data_dir.join(format!("rekey-backup-{ts}"));
    std::fs::create_dir_all(&backup_dir)
        .with_context(|| format!("backup 디렉토리 생성: {}", backup_dir.display()))?;

    // keystore 디렉토리 복사 (있을 때만).
    let ks_dir = keystore_dir(data_dir);
    if ks_dir.is_dir() {
        let dest = backup_dir.join("keystore");
        copy_dir_recursive(&ks_dir, &dest)
            .with_context(|| format!("keystore 백업: {}", ks_dir.display()))?;
    }

    // db.sqlite + WAL/SHM 사이드카 복사.
    let db = db_path(data_dir);
    if let Some(db_name) = db.file_name().and_then(|n| n.to_str()) {
        if let Some(parent) = db.parent() {
            for suffix in ["", "-wal", "-shm"] {
                let src = parent.join(format!("{db_name}{suffix}"));
                if src.is_file() {
                    let dest = backup_dir.join(format!("{db_name}{suffix}"));
                    std::fs::copy(&src, &dest)
                        .with_context(|| format!("db 백업: {}", src.display()))?;
                }
            }
        }
    }

    Ok(backup_dir)
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn open_db(data_dir: &Path) -> Result<Db> {
    let mut db = Db::open(DbConfig {
        path: db_path(data_dir),
        ..Default::default()
    })
    .map_err(|e| anyhow!("db open: {e}"))?;
    db.migrate().map_err(|e| anyhow!("db migrate: {e}"))?;
    Ok(db)
}

/// daemon.env 의 XGRAM_KEYSTORE_PASSWORD 줄을 교체/삽입. 다른 줄은 보존.
/// 파일이 없으면 0600 권한으로 생성.
fn update_daemon_env(data_dir: &Path, new: &str) -> Result<()> {
    let env_path = data_dir.join("daemon.env");
    let key = openxgram_core::env::PASSWORD_ENV;
    let new_line = format!("{key}={new}");

    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    if env_path.is_file() {
        let contents = std::fs::read_to_string(&env_path)
            .with_context(|| format!("daemon.env 읽기: {}", env_path.display()))?;
        for raw in contents.lines() {
            let trimmed = raw.trim_start();
            let stripped = trimmed.strip_prefix("export ").unwrap_or(trimmed);
            if stripped.starts_with(&format!("{key}=")) {
                lines.push(new_line.clone());
                replaced = true;
            } else {
                lines.push(raw.to_string());
            }
        }
    }
    if !replaced {
        lines.push(new_line);
    }

    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&env_path, out)
        .with_context(|| format!("daemon.env 쓰기: {}", env_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// 새 비번으로 keystore + vault 가 복호화되는지 확인.
fn verify(data_dir: &Path, new: &str, ks_count: usize, vault_count: usize) -> Result<()> {
    if ks_count > 0 {
        let ks = FsKeystore::new(keystore_dir(data_dir));
        let entries = ks.list().map_err(|e| anyhow!("keystore list: {e}"))?;
        let first = entries
            .first()
            .ok_or_else(|| anyhow!("keystore 항목이 사라짐"))?;
        ks.load(&first.name, new)
            .map_err(|e| anyhow!("새 비번으로 keystore.load 실패: {e}"))?;
    }

    if vault_count > 0 {
        let mut db = open_db(data_dir)?;
        let mut store = VaultStore::new(&mut db);
        let entries = store.list().map_err(|e| anyhow!("vault list: {e}"))?;
        let first = entries.first().ok_or_else(|| anyhow!("vault 항목이 사라짐"))?;
        store
            .get(&first.key, new)
            .map_err(|e| anyhow!("새 비번으로 vault.get 실패: {e}"))?;
    }

    Ok(())
}

fn with_backup_hint(backup_dir: &Path, msg: String) -> anyhow::Error {
    anyhow!(
        "{msg}\n수동 복구: 위 백업을 원위치로 되돌리세요 — {}",
        backup_dir.display()
    )
}
