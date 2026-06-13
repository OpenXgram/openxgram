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
    //   RESILIENT (rc.322): 역사적으로 다른 비번으로 만들어져 섞인 keyfile 은
    //   old 비번으로 복호화 안 되면 skip(warn) 하고 진행. vault(아래)가 로그인
    //   게이트이므로 깨진 서명 keyfile 몇 개가 비번 변경을 막지 않는다.
    let ks_dir = keystore_dir(data_dir);
    let ks = FsKeystore::new(&ks_dir);
    let (ks_count, ks_skipped) = ks
        .reencrypt_all(old, new)
        .map_err(|e| with_backup_hint(&backup_dir, format!("keystore 재암호화 실패: {e}")))?;
    println!("[rekey] keystore 재암호화: {ks_count} 개 keyfile");
    if !ks_skipped.is_empty() {
        // 매 skip 은 keystore 레이어에서 이미 warn 로그됨 — 여기선 집계 보고.
        tracing::warn!(
            skipped = ks_skipped.len(),
            keyfiles = ?ks_skipped,
            "rc.322 keystore rekey: old 비번 불일치 keyfile skip (재암호화 안 됨 — 원본 유지)",
        );
        println!(
            "[rekey] keystore skip: {} 개 (old 비번 불일치 — {})",
            ks_skipped.len(),
            ks_skipped.join(", ")
        );
    }

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

    // (e) VERIFY — vault.get 은 hard check(로그인 게이트). keystore 는
    //   '성공적으로 재암호화된' 키만 검증(skip 된 깨진 keyfile 은 검증 대상 아님).
    //   재암호화된 keyfile 이 0 개면 keystore 검증을 skip(warn) 한다 — vault 가 게이트.
    verify(data_dir, new, &ks_skipped, vault_count)
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
///
/// keystore 검증은 **성공적으로 재암호화된 키 1개**만 대상으로 한다(`ks_skipped`
/// 에 든 깨진 keyfile 은 old 비번 그대로 남아 있으므로 new 로 load 하면 당연히
/// 실패 → 검증 대상에서 제외). 재암호화된 키가 하나도 없으면(전부 skip) keystore
/// 검증을 건너뛰고 warn 만 남긴다 — vault 가 로그인 게이트라 비번 변경은 유효.
fn verify(data_dir: &Path, new: &str, ks_skipped: &[String], vault_count: usize) -> Result<()> {
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let entries = ks.list().map_err(|e| anyhow!("keystore list: {e}"))?;
    // skip 되지 않은(=재암호화 성공한) 첫 keyfile 을 검증 대상으로 선택.
    let verify_target = entries
        .iter()
        .find(|e| !ks_skipped.iter().any(|s| s == &e.name));
    match verify_target {
        Some(entry) => {
            ks.load(&entry.name, new)
                .map_err(|e| anyhow!("새 비번으로 keystore.load 실패: {e}"))?;
        }
        None => {
            // 재암호화된 keyfile 0 개 — 검증 skip(hard fail 아님). vault 가 게이트.
            tracing::warn!(
                "rc.322 rekey verify: 재암호화된 keystore keyfile 이 없어 keystore 검증 skip (vault 검증으로 게이트)",
            );
        }
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
