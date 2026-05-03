//! xgram uninstall — Phase 1 비대화 제거 + cold backup 옵션.
//!
//! SPEC-lifecycle §5:
//!   1. precheck — install-manifest 읽고 검증, 없으면 idempotent exit 0
//!   2. backup option:
//!      --cold-backup-to PATH  → 옵션 2 (ChaCha20-Poly1305 + tar.gz)
//!      --no-backup            → 옵션 4 (--confirm "DELETE OPENXGRAM" 필수)
//!   3. dry-run 또는 실제 제거 — data_dir 통째로
//!   4. post-verify — data_dir 미존재 확인
//!
//! cold backup 시는 백업 자체가 안전망이므로 --confirm 불필요.
//! full backup(옵션 1, Tailscale)·부분 보존(옵션 3)·흔적 스캔은 후속 PR.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use openxgram_manifest::InstallManifest;

use crate::backup::create_cold_backup;

const CONFIRM_STRING: &str = "DELETE OPENXGRAM";
const PASSWORD_ENV: &str = "XGRAM_KEYSTORE_PASSWORD";

#[derive(Debug, Clone)]
pub struct UninstallOpts {
    pub data_dir: PathBuf,
    pub no_backup: bool,
    /// cold backup 대상 경로 (`~/.openxgram-backup-YYYYMMDD-HHMMSS.tar.gz.enc`)
    pub cold_backup_to: Option<PathBuf>,
    pub confirm: Option<String>,
    pub dry_run: bool,
}

pub fn run_uninstall(opts: &UninstallOpts) -> Result<()> {
    let manifest_path = opts.data_dir.join("install-manifest.json");
    if !manifest_path.exists() {
        println!(
            "이미 제거되었거나 설치된 적이 없습니다 ({}).",
            manifest_path.display()
        );
        return Ok(());
    }

    println!("[1/4] 사전 검증");
    let manifest = InstallManifest::read(&manifest_path)
        .context("install-manifest 읽기·검증 실패")?;
    println!("  alias       : {}", manifest.machine.alias);
    println!("  registered  : {}개 키", manifest.registered_keys.len());
    println!("  directories : {}", manifest.directories.len());

    println!("[2/4] 백업 옵션");
    match (opts.cold_backup_to.as_ref(), opts.no_backup) {
        (Some(_), true) => bail!(
            "--cold-backup-to 와 --no-backup 동시 사용 금지. 하나만 선택."
        ),
        (None, false) => bail!(
            "백업 옵션 필요: --cold-backup-to PATH (옵션 2) 또는 --no-backup (옵션 4)"
        ),
        (Some(target), false) => {
            let password = std::env::var(PASSWORD_ENV)
                .map_err(|_| anyhow!("환경변수 {PASSWORD_ENV} 누락 — backup 암호화에 필요"))?;
            if opts.dry_run {
                println!(
                    "  [dry-run] cold backup 대상: {} (실제 생성 생략)",
                    target.display()
                );
            } else {
                let info = create_cold_backup(&opts.data_dir, target, &password)?;
                println!(
                    "  ✓ cold backup 생성: {} ({} bytes)",
                    info.path.display(),
                    info.size_bytes
                );
                println!("    sha256={}", info.sha256);
            }
        }
        (None, true) => {
            println!("  --no-backup 선택 (백업 없음)");
            // confirm 필수
            let confirm = opts.confirm.as_deref().ok_or_else(|| {
                anyhow!("--no-backup 사용 시 --confirm \"{CONFIRM_STRING}\" 정확 일치 필요")
            })?;
            if confirm != CONFIRM_STRING {
                bail!(
                    "확인 문자열 불일치. 정확히 \"{CONFIRM_STRING}\" 입력 필요 (대소문자 포함)"
                );
            }
        }
    }

    if opts.dry_run {
        println!();
        println!("[dry-run] 다음 작업이 수행됩니다:");
        println!("  rm -rf {}", opts.data_dir.display());
        return Ok(());
    }

    println!("[3/4] 제거");
    remove_data_dir(&opts.data_dir).context("데이터 디렉토리 제거 실패")?;

    println!("[4/4] 사후 검증");
    if opts.data_dir.exists() {
        bail!(
            "사후 검증 실패: 데이터 디렉토리가 여전히 존재 ({})",
            opts.data_dir.display()
        );
    }

    println!();
    println!("✓ OpenXgram 제거 완료");
    println!("  data_dir 삭제: {}", opts.data_dir.display());
    println!("  홈 디렉토리 흔적 스캔(find $HOME -name '*xgram*')은 후속 PR 에서 자동화됩니다.");
    Ok(())
}

fn remove_data_dir(data_dir: &Path) -> Result<()> {
    if !data_dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(data_dir)
        .with_context(|| format!("디렉토리 제거 실패: {}", data_dir.display()))
}
