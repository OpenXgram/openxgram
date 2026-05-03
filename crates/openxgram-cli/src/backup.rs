//! Cold backup — 데이터 디렉토리를 tar.gz 로 묶고 ChaCha20-Poly1305 로 암호화.
//!
//! SPEC-lifecycle §5.2 옵션 2 (마스터 결정 2026-04-30 — keystore §12.6 와
//! 일관). restore 는 Phase 2 후속 PR.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use openxgram_keystore::encrypt_blob;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
}

/// data_dir 통째로 tar.gz 후 ChaCha20-Poly1305 암호화하여 target_path 에 저장.
pub fn create_cold_backup(
    data_dir: &Path,
    target_path: &Path,
    password: &str,
) -> Result<BackupInfo> {
    if !data_dir.exists() {
        return Err(anyhow!(
            "데이터 디렉토리 미존재: {}",
            data_dir.display()
        ));
    }

    // 1. tar.gz 메모리에 작성
    let gz = GzEncoder::new(Vec::new(), Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.append_dir_all(".", data_dir)
        .with_context(|| format!("tar 생성 실패: {}", data_dir.display()))?;
    let gz = tar.into_inner().context("tar 마감 실패")?;
    let plaintext = gz.finish().context("gzip 마감 실패")?;

    // 2. ChaCha20-Poly1305 암호화
    let blob = encrypt_blob(password, &plaintext)
        .map_err(|e| anyhow!("backup 암호화 실패: {e}"))?;

    // 3. 파일 저장 (parent 디렉토리 생성 보장)
    if let Some(parent) = target_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("backup 부모 디렉토리 생성 실패: {}", parent.display())
            })?;
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
