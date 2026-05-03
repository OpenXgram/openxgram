//! xgram init — 9단계 온보딩 워크플로우 (Phase 1: Step 1-6 + manifest 작성).
//!
//! 비대화 모드만 지원. TUI 마법사는 후속 PR.
//!
//! 단계:
//!   1. 사전 점검 (포트·OS·기존 설치)
//!   2. 머신 식별 (alias, role)
//!   3. 마스터 시드 (BIP39 24단어 자동 생성 또는 XGRAM_SEED import)
//!   4. 마스터 키페어 (BIP44 m/44'/60'/0'/0/0, ChaCha20-Poly1305 저장)
//!   5. 데이터 디렉토리 (~/.openxgram/{,keystore,backup})
//!   6. DB 초기화 (SQLite + 첫 마이그레이션)
//!   7. install-manifest.json 작성 + secp256k1 ECDSA 서명

use std::net::TcpListener;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, FixedOffset, Utc};
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::{
    DirectoryEntry, InstallManifest, Machine, MachineRole, OsKind, PortEntry, PortProtocol,
    RegisteredKey, SCHEMA_VERSION,
};

const DEFAULT_PORTS: &[u16] = &[7300, 7301];
const SEED_ENV: &str = "XGRAM_SEED";
const PASSWORD_ENV: &str = "XGRAM_KEYSTORE_PASSWORD";
const MIN_PASSWORD_LEN: usize = 12;
const MASTER_KEY_NAME: &str = "master";
const MASTER_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";

#[derive(Debug, Clone)]
pub struct InitOpts {
    pub alias: String,
    pub role: MachineRole,
    pub data_dir: PathBuf,
    pub force: bool,
    pub dry_run: bool,
    /// XGRAM_SEED 환경변수의 24단어 시드를 import (다른 머신에서 동일 시드 사용)
    pub import: bool,
}

pub fn run_init(opts: &InitOpts) -> Result<()> {
    let label = if opts.dry_run { " (dry-run)" } else { "" };

    println!("[1/6] 사전 점검{label}");
    precheck(opts)?;

    println!(
        "[2/6] 머신 식별 — alias={}, role={}",
        opts.alias,
        role_str(opts.role)
    );
    let machine = build_machine(opts)?;

    println!("[3/6] 마스터 시드");
    let phrase = obtain_seed_phrase(opts)?;

    println!("[4/6] 마스터 키페어 ({MASTER_DERIVATION_PATH})");
    let password = std::env::var(PASSWORD_ENV)
        .map_err(|_| anyhow!("환경변수 {PASSWORD_ENV} 누락"))?;
    if password.len() < MIN_PASSWORD_LEN {
        bail!(
            "패스워드는 최소 {MIN_PASSWORD_LEN}자 (현재: {})",
            password.len()
        );
    }
    let registered_master = if opts.dry_run {
        RegisteredKey {
            alias: MASTER_KEY_NAME.into(),
            address: "0x[dry-run-skipped]".into(),
            derivation_path: MASTER_DERIVATION_PATH.into(),
        }
    } else {
        setup_master_keypair(&opts.data_dir, phrase.as_deref(), &password)?
    };

    println!("[5/6] 데이터 디렉토리 {}", opts.data_dir.display());
    let directories = ensure_data_dirs(&opts.data_dir, opts.dry_run)?;

    println!("[6/6] DB 초기화 + 마이그레이션");
    if !opts.dry_run {
        init_database(&opts.data_dir).context("DB 초기화 실패")?;
    }

    let manifest_path = opts.data_dir.join("install-manifest.json");
    let unsigned = build_manifest(&machine, registered_master, directories);

    if opts.dry_run {
        println!();
        println!("[dry-run] 작성될 manifest:");
        println!("{}", serde_json::to_string_pretty(&unsigned)?);
        return Ok(());
    }

    let signed = sign_manifest(&opts.data_dir, &password, unsigned)?;
    signed
        .write(&manifest_path)
        .with_context(|| format!("install-manifest 저장 실패: {}", manifest_path.display()))?;

    println!();
    println!("✓ OpenXgram 설치 완료");
    println!("  alias    : {}", opts.alias);
    println!("  address  : {}", signed.registered_keys[0].address);
    println!("  data_dir : {}", opts.data_dir.display());
    println!("  manifest : {}", manifest_path.display());
    Ok(())
}

fn precheck(opts: &InitOpts) -> Result<()> {
    let manifest_path = opts.data_dir.join("install-manifest.json");
    if manifest_path.exists() && !opts.force {
        bail!(
            "이미 설치되어 있습니다 ({}). `xgram uninstall` 후 재시도하거나 `xgram init --force` 사용.",
            manifest_path.display()
        );
    }

    for &port in DEFAULT_PORTS {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => drop(l),
            Err(e) => bail!("필수 포트 {port} 점유: {e}"),
        }
    }

    Ok(())
}

fn build_machine(opts: &InitOpts) -> Result<Machine> {
    Ok(Machine {
        alias: opts.alias.clone(),
        role: opts.role,
        os: detect_os()?,
        arch: std::env::consts::ARCH.to_string(),
        hostname: std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into()),
        tailscale_ip: None,
    })
}

fn detect_os() -> Result<OsKind> {
    Ok(match std::env::consts::OS {
        "linux" => OsKind::Linux,
        "macos" => OsKind::Macos,
        "windows" => OsKind::Windows,
        other => bail!("지원하지 않는 OS: {other}"),
    })
}

fn obtain_seed_phrase(opts: &InitOpts) -> Result<Option<String>> {
    if opts.import {
        let phrase = std::env::var(SEED_ENV).map_err(|_| {
            anyhow!("--import 시 환경변수 {SEED_ENV} 가 24단어 시드를 담고 있어야 합니다")
        })?;
        Ok(Some(phrase))
    } else {
        Ok(None)
    }
}

fn setup_master_keypair(
    data_dir: &Path,
    seed_phrase: Option<&str>,
    password: &str,
) -> Result<RegisteredKey> {
    let ks = FsKeystore::new(data_dir.join("keystore"));
    let address = match seed_phrase {
        None => {
            let (addr, mnemonic) = ks
                .create(MASTER_KEY_NAME, password)
                .context("마스터 키 생성 실패")?;
            println!();
            println!("새 마스터 시드 (BIP39 24단어).");
            println!("⚠ 안전한 곳에 보관 — 다시 표시되지 않습니다.");
            println!("  {mnemonic}");
            println!();
            addr
        }
        Some(phrase) => ks
            .import(MASTER_KEY_NAME, phrase, password)
            .context("시드 import 실패")?,
    };
    Ok(RegisteredKey {
        alias: MASTER_KEY_NAME.into(),
        address: address.to_string(),
        derivation_path: MASTER_DERIVATION_PATH.into(),
    })
}

fn ensure_data_dirs(data_dir: &Path, dry_run: bool) -> Result<Vec<DirectoryEntry>> {
    let dirs = [
        data_dir.to_path_buf(),
        data_dir.join("keystore"),
        data_dir.join("backup"),
    ];
    let mut entries = Vec::with_capacity(dirs.len());
    for d in &dirs {
        if !dry_run {
            std::fs::create_dir_all(d)
                .with_context(|| format!("디렉토리 생성 실패: {}", d.display()))?;
        }
        entries.push(DirectoryEntry {
            path: d.clone(),
            created_by_installer: true,
        });
    }
    Ok(entries)
}

fn init_database(data_dir: &Path) -> Result<()> {
    let mut db = Db::open(DbConfig {
        path: data_dir.join("db.sqlite"),
        ..Default::default()
    })?;
    db.migrate()?;
    Ok(())
}

fn build_manifest(
    machine: &Machine,
    master: RegisteredKey,
    directories: Vec<DirectoryEntry>,
) -> InstallManifest {
    InstallManifest {
        version: SCHEMA_VERSION.into(),
        installed_at: kst_now(),
        machine: machine.clone(),
        uninstall_token: String::new(), // 서명 후 hex(sig) 채움
        files: vec![],
        directories,
        system_services: vec![],
        binaries: vec![],
        shell_integrations: vec![],
        external_resources: vec![],
        registered_keys: vec![master],
        ports: vec![
            PortEntry {
                number: 7300,
                protocol: PortProtocol::Tcp,
                service: "xgram-rpc".into(),
            },
            PortEntry {
                number: 7301,
                protocol: PortProtocol::Tcp,
                service: "xgram-http".into(),
            },
        ],
        os_keychain_entries: vec![],
        selected_extractors: serde_json::Value::Null,
        inbound_webhook_port: Some(14921),
        backup_schedule: None,
    }
}

fn sign_manifest(
    data_dir: &Path,
    password: &str,
    manifest: InstallManifest,
) -> Result<InstallManifest> {
    let ks = FsKeystore::new(data_dir.join("keystore"));
    let kp = ks
        .load(MASTER_KEY_NAME, password)
        .context("master 키 로드 실패 — keystore 패스워드 확인")?;
    let signature = kp.sign(&manifest.canonical_bytes()?);
    let mut signed = manifest;
    signed.uninstall_token = hex::encode(signature);
    Ok(signed)
}

fn kst_now() -> DateTime<FixedOffset> {
    let kst = FixedOffset::east_opt(9 * 3600).expect("KST offset valid");
    Utc::now().with_timezone(&kst)
}

fn role_str(r: MachineRole) -> &'static str {
    match r {
        MachineRole::Primary => "primary",
        MachineRole::Secondary => "secondary",
        MachineRole::Worker => "worker",
    }
}

pub fn default_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME 환경변수 누락"))?;
    Ok(PathBuf::from(home).join(".openxgram"))
}
