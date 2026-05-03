//! xgram uninstall — Phase 1 비대화 핵심 제거.
//!
//! SPEC-lifecycle §5 의 일부:
//!   1. precheck — install-manifest.json 읽고 검증, 없으면 idempotent exit 0
//!   2. backup option — 첫 PR 은 `--no-backup` (옵션 4) 만 지원
//!   3. confirm — `--confirm "DELETE OPENXGRAM"` 정확 일치 필수
//!   4. dry-run 또는 실제 제거 — data_dir 통째로
//!   5. post-verify — data_dir / manifest_path 미존재 확인
//!
//! 후속 PR 범위: cold backup(옵션 2 — ChaCha20-Poly1305 + tar), full backup
//! (옵션 1 — Tailscale transport), 부분 보존 (옵션 3),
//! `find $HOME -name "*xgram*"` 흔적 스캔.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use openxgram_manifest::InstallManifest;

const CONFIRM_STRING: &str = "DELETE OPENXGRAM";

#[derive(Debug, Clone)]
pub struct UninstallOpts {
    pub data_dir: PathBuf,
    pub no_backup: bool,
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
    if !opts.no_backup {
        bail!(
            "Phase 1 은 --no-backup 만 지원합니다. cold/full backup 은 후속 PR."
        );
    }
    println!("  --no-backup 선택 (백업 없음)");

    println!("[3/4] 명시적 확인");
    let confirm = opts.confirm.as_deref().ok_or_else(|| {
        anyhow!("--no-backup 사용 시 --confirm \"{CONFIRM_STRING}\" 정확 일치 필요")
    })?;
    if confirm != CONFIRM_STRING {
        bail!(
            "확인 문자열 불일치. 정확히 \"{CONFIRM_STRING}\" 입력 필요 (대소문자 포함)"
        );
    }
    println!("  확인 문자열 일치");

    if opts.dry_run {
        println!();
        println!("[dry-run] 다음 작업이 수행됩니다:");
        println!("  rm -rf {}", opts.data_dir.display());
        return Ok(());
    }

    println!("[4/4] 제거");
    remove_data_dir(&opts.data_dir).context("데이터 디렉토리 제거 실패")?;

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
