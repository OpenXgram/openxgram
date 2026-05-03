//! xgram reset — Phase 1 `--hard` (데이터 + 키 모두 삭제 후 재초기화 가능 상태).
//!
//! SPEC-lifecycle §7. Phase 1 은 `--hard` 만 지원. `--keep-keys` /
//! `--keep-config` 는 후속 PR.
//!
//! `--hard` 동작은 사실상 uninstall 과 동일 (데이터 디렉토리 통째 삭제 후
//! `xgram init` 으로 재설치). 코드 중복 회피를 위해 `uninstall::run_uninstall`
//! 을 그대로 호출. confirm 문자열만 다르다 (RESET vs DELETE).

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use openxgram_core::confirm::{DELETE_CONFIRM, RESET_CONFIRM};

use crate::uninstall::{run_uninstall, UninstallOpts};

#[derive(Debug, Clone)]
pub struct ResetOpts {
    pub data_dir: PathBuf,
    pub hard: bool,
    pub confirm: Option<String>,
    pub dry_run: bool,
}

pub fn run_reset(opts: &ResetOpts) -> Result<()> {
    if !opts.hard {
        bail!("Phase 1 은 --hard 만 지원합니다. --keep-keys/--keep-config 는 후속 PR.");
    }

    let confirm = opts
        .confirm
        .as_deref()
        .ok_or_else(|| anyhow!("--hard 사용 시 --confirm \"{RESET_CONFIRM}\" 정확 일치 필요"))?;
    if confirm != RESET_CONFIRM {
        bail!("확인 문자열 불일치. 정확히 \"{RESET_CONFIRM}\" 입력 필요 (대소문자 포함)");
    }

    println!("xgram reset --hard");
    println!("  데이터 디렉토리 통째 삭제 후 재초기화 가능 상태로 둡니다.");
    println!("  (uninstall 과 동일 동작 — `xgram init` 으로 재설치 가능)");
    println!();

    run_uninstall(&UninstallOpts {
        data_dir: opts.data_dir.clone(),
        no_backup: true,
        cold_backup_to: None,
        confirm: Some(DELETE_CONFIRM.into()),
        dry_run: opts.dry_run,
    })?;

    if !opts.dry_run {
        println!();
        println!("✓ xgram reset --hard 완료. `xgram init --alias <NAME>` 으로 재설치하세요.");
    }
    Ok(())
}
