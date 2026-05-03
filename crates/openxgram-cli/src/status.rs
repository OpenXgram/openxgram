//! xgram status — install-manifest 기반 현재 상태 출력.
//!
//! Phase 1: manifest read + 머신·키·포트 요약. 데몬·DB 통계·연결 상태는
//! 해당 모듈 구현 후 단계적 추가.

use std::path::PathBuf;

use anyhow::Result;
use openxgram_core::paths::manifest_path;
use openxgram_manifest::InstallManifest;

#[derive(Debug, Clone)]
pub struct StatusOpts {
    pub data_dir: PathBuf,
}

pub fn run_status(opts: &StatusOpts) -> Result<()> {
    let mp = manifest_path(&opts.data_dir);
    if !mp.exists() {
        println!("OpenXgram 미설치 ({}).", mp.display());
        println!("  `xgram init --alias <NAME>` 으로 설치하세요.");
        return Ok(());
    }

    let m = InstallManifest::read(&mp)?;
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
    println!("  manifest     : {}", mp.display());
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
