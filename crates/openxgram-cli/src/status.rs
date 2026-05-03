//! xgram status — install-manifest 기반 현재 상태 출력.
//!
//! Phase 1: manifest read + 머신·키·포트 요약. 데몬·DB 통계·연결 상태는
//! 해당 모듈 구현 후 단계적 추가.

use std::path::PathBuf;

use anyhow::Result;
use openxgram_manifest::InstallManifest;

#[derive(Debug, Clone)]
pub struct StatusOpts {
    pub data_dir: PathBuf,
}

pub fn run_status(opts: &StatusOpts) -> Result<()> {
    let manifest_path = opts.data_dir.join("install-manifest.json");
    if !manifest_path.exists() {
        println!("OpenXgram 미설치 ({}).", manifest_path.display());
        println!("  `xgram init --alias <NAME>` 으로 설치하세요.");
        return Ok(());
    }

    let m = InstallManifest::read(&manifest_path)?;
    println!("xgram status");
    println!();
    println!("  alias        : {}", m.machine.alias);
    println!("  role         : {}", m.machine.role);
    println!("  os / arch    : {} / {}", m.machine.os, m.machine.arch);
    println!("  hostname     : {}", m.machine.hostname);
    println!(
        "  tailscale_ip : {}",
        m.machine.tailscale_ip.as_deref().unwrap_or("(미설정)")
    );
    println!("  installed_at : {}", m.installed_at);
    println!("  data_dir     : {}", opts.data_dir.display());
    println!("  manifest     : {}", manifest_path.display());
    println!();
    println!("  registered keys ({}):", m.registered_keys.len());
    for k in &m.registered_keys {
        println!("    {} — {} ({})", k.alias, k.address, k.derivation_path);
    }
    println!();
    println!("  ports ({}):", m.ports.len());
    for p in &m.ports {
        println!("    {}/{} — {}", p.number, p.protocol, p.service);
    }
    Ok(())
}
