//! Cold backup — 데이터 디렉토리를 tar.gz 로 묶고 ChaCha20-Poly1305 로 암호화.
//!
//! SPEC-lifecycle §5.2 옵션 2 (마스터 결정 2026-04-30 — keystore §12.6 와
//! 일관). restore 는 Phase 2 후속 PR.

use std::path::{Path, PathBuf};

/// `to` 가 기존 디렉토리면 KST timestamp 파일명을 생성, 아니면 그대로 사용.
/// systemd timer 가 동일 디렉토리로 반복 호출할 때 파일 충돌을 회피.
pub fn resolve_backup_target(to: &Path) -> anyhow::Result<PathBuf> {
    if to.is_dir() {
        let ts = openxgram_core::time::kst_now().format("%Y%m%d-%H%M%S");
        Ok(to.join(format!("openxgram-{ts}.cbk")))
    } else {
        Ok(to.to_path_buf())
    }
}

use anyhow::{anyhow, bail, Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use openxgram_keystore::{decrypt_blob, encrypt_blob};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct RestoreInfo {
    pub target_dir: PathBuf,
    pub bytes_restored: u64,
}

/// data_dir 통째로 tar.gz 후 ChaCha20-Poly1305 암호화하여 target_path 에 저장.
pub fn create_cold_backup(
    data_dir: &Path,
    target_path: &Path,
    password: &str,
) -> Result<BackupInfo> {
    if !data_dir.exists() {
        return Err(anyhow!("데이터 디렉토리 미존재: {}", data_dir.display()));
    }

    // 1. tar.gz 메모리에 작성
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.append_dir_all(".", data_dir)
        .with_context(|| format!("tar 생성 실패: {}", data_dir.display()))?;
    let gz = tar.into_inner().context("tar 마감 실패")?;
    let plaintext = gz.finish().context("gzip 마감 실패")?;

    // 2. ChaCha20-Poly1305 암호화
    let blob =
        encrypt_blob(password, &plaintext).map_err(|e| anyhow!("backup 암호화 실패: {e}"))?;

    // 3. 파일 저장 (parent 디렉토리 생성 보장)
    if let Some(parent) = target_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("backup 부모 디렉토리 생성 실패: {}", parent.display()))?;
        }
    }
    std::fs::write(target_path, &blob)
        .with_context(|| format!("backup 파일 저장 실패: {}", target_path.display()))?;

    // 4. SHA256 (암호화된 blob 기준)
    let mut hasher = Sha256::new();
    hasher.update(&blob);
    let sha = hex::encode(hasher.finalize());

    Ok(BackupInfo {
        path: target_path.to_path_buf(),
        size_bytes: blob.len() as u64,
        sha256: sha,
    })
}

/// cold backup 파일을 복호화·압축 해제하여 target_dir 로 복원.
/// target_dir 가 비어있지 않으면 raise — `--merge` 모드는 `restore_cold_backup_merge` 사용.
pub fn restore_cold_backup(
    backup_path: &Path,
    target_dir: &Path,
    password: &str,
) -> Result<RestoreInfo> {
    restore_internal(backup_path, target_dir, password, /*merge=*/ false)
}

/// merge 모드 — 비어있지 않은 target_dir 로 백업 파일 덮어쓰기.
/// 백업에 없는 파일은 보존, 백업에 있는 파일은 덮어씀. 위험: 양방향
/// 충돌 무관 단순 덮어쓰기 (분쟁 해소·rename 후속).
pub fn restore_cold_backup_merge(
    backup_path: &Path,
    target_dir: &Path,
    password: &str,
) -> Result<RestoreInfo> {
    restore_internal(backup_path, target_dir, password, /*merge=*/ true)
}

fn restore_internal(
    backup_path: &Path,
    target_dir: &Path,
    password: &str,
    merge: bool,
) -> Result<RestoreInfo> {
    let blob = std::fs::read(backup_path)
        .with_context(|| format!("backup 파일 읽기 실패: {}", backup_path.display()))?;
    let bytes_restored = blob.len() as u64;
    let plaintext =
        decrypt_blob(password, &blob).map_err(|e| anyhow!("backup 복호화 실패: {e}"))?;

    if target_dir.exists() {
        let mut iter = std::fs::read_dir(target_dir)
            .with_context(|| format!("target_dir read_dir 실패: {}", target_dir.display()))?;
        if iter.next().is_some() && !merge {
            bail!(
                "target_dir 비어있지 않음: {} — `xgram uninstall` 또는 빈 경로 사용 (또는 --merge 옵션)",
                target_dir.display()
            );
        }
    } else {
        std::fs::create_dir_all(target_dir)
            .with_context(|| format!("target_dir 생성 실패: {}", target_dir.display()))?;
    }

    let gz = GzDecoder::new(std::io::Cursor::new(plaintext));
    let mut archive = tar::Archive::new(gz);
    // tar 의 unpack 은 같은 경로 파일이 있으면 기본적으로 덮어씀.
    archive
        .unpack(target_dir)
        .with_context(|| format!("tar.gz 해제 실패: {}", target_dir.display()))?;

    Ok(RestoreInfo {
        target_dir: target_dir.to_path_buf(),
        bytes_restored,
    })
}
