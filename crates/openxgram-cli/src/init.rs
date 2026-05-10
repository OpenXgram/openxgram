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

use anyhow::{bail, Context, Result};
use openxgram_core::env::{require_seed_phrase, MIN_PASSWORD_LEN};

use crate::pw::obtain_password_for_init;
use openxgram_core::paths::{db_path, install_dirs, keystore_dir, manifest_path, MASTER_KEY_NAME};
use openxgram_core::ports::{
    HTTP_PORT, HTTP_SERVICE, INBOUND_WEBHOOK_PORT, REQUIRED_PORTS, RPC_PORT, RPC_SERVICE,
};
use openxgram_core::time::kst_now;
use openxgram_db::{Db, DbConfig};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::{
    DirectoryEntry, InstallManifest, Machine, MachineRole, OsKind, PortEntry, PortProtocol,
    RegisteredKey, SCHEMA_VERSION,
};

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

    println!("[2/6] 머신 식별 — alias={}, role={}", opts.alias, opts.role);
    let machine = build_machine(opts)?;

    println!("[3/6] 마스터 시드");
    let phrase = obtain_seed_phrase(opts)?;

    println!("[4/6] 마스터 키페어 ({MASTER_DERIVATION_PATH})");
    let password = obtain_password_for_init()?;
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

    let target = manifest_path(&opts.data_dir);
    let unsigned = build_manifest(&machine, registered_master, directories);

    if opts.dry_run {
        println!();
        println!("[dry-run] 작성될 manifest:");
        println!("{}", serde_json::to_string_pretty(&unsigned)?);
        return Ok(());
    }

    let signed = sign_manifest(&opts.data_dir, &password, unsigned)?;
    signed
        .write(&target)
        .with_context(|| format!("install-manifest 저장 실패: {}", target.display()))?;

    println!();
    println!("✓ OpenXgram 설치 완료");
    println!("  alias    : {}", opts.alias);
    println!("  address  : {}", signed.registered_keys[0].address);
    println!("  data_dir : {}", opts.data_dir.display());
    println!("  manifest : {}", target.display());
    Ok(())
}

fn precheck(opts: &InitOpts) -> Result<()> {
    let mp = manifest_path(&opts.data_dir);
    if mp.exists() && !opts.force {
        bail!(
            "이미 설치되어 있습니다 ({}). `xgram uninstall` 후 재시도하거나 `xgram init --force` 사용.",
            mp.display()
        );
    }
    // 통합 테스트 병렬 실행 시 동일 포트 contention 회피용 escape hatch.
    if std::env::var("XGRAM_SKIP_PORT_PRECHECK").is_ok() {
        return Ok(());
    }
    // init 자체는 포트를 안 쓴다 (keystore·DB·manifest 만 만든다).
    // 포트는 daemon 가동 시점에야 필요하므로, 선점 사실은 경고로만 알리고 init 은 진행.
    // bail 로 막던 이전 동작은 사용자 마찰의 큰 원인이었다 (예: 데모 daemon 위에서 새 init).
    for &port in REQUIRED_PORTS {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => drop(l),
            Err(e) => eprintln!(
                "  ⚠ 포트 {port} 현재 점유 ({e}) — init 은 진행 가능. \
                 이후 `xgram daemon` 띄울 땐 다른 프로세스 정리 또는 --bind 로 다른 포트 지정 필요."
            ),
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
        Ok(Some(require_seed_phrase()?))
    } else {
        Ok(None)
    }
}

fn setup_master_keypair(
    data_dir: &Path,
    seed_phrase: Option<&str>,
    password: &str,
) -> Result<RegisteredKey> {
    let ks = FsKeystore::new(keystore_dir(data_dir));
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
            // 마스터 결정 2026-04-30 (CLAUDE.md 12.1.1): 시드 백업 확인 Y/N.
            // N 또는 비-y 입력 시 설치 중단. 비-TTY (CI/test) 는 경고 후 통과.
            confirm_seed_backed_up(data_dir)?;
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

/// 시드 백업 확인 prompt — TTY 면 Y/N 묻고, 비-TTY 면 자동화로 간주해 통과.
/// N (또는 비-y) 입력 시 즉시 종료. 마스터 결정 2026-04-30 정책 적용.
fn confirm_seed_backed_up(data_dir: &Path) -> Result<()> {
    use std::io::{self, BufRead, IsTerminal, Write};

    if !io::stdin().is_terminal() {
        eprintln!("⚠ TTY 아님 — 시드 백업 확인 프롬프트 건너뜀 (자동화 모드).");
        return Ok(());
    }

    print!("위 시드를 안전한 곳에 기록했습니까? [y/N]: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin()
        .lock()
        .read_line(&mut input)
        .context("표준 입력 읽기 실패")?;

    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => {
            println!();
            Ok(())
        }
        _ => {
            println!();
            eprintln!("✗ 시드 백업이 확인되지 않아 설치를 중단합니다.");
            eprintln!("  data_dir 을 삭제하고 다시 실행해 새 시드를 받으세요:");
            eprintln!("  - macOS / Linux: rm -rf {}", data_dir.display());
            eprintln!(
                "  - Windows (PowerShell): Remove-Item -Recurse -Force {}",
                data_dir.display()
            );
            bail!("시드 백업 미확인 — 백업 없이는 진행 불가 (절대 규칙)");
        }
    }
}

fn ensure_data_dirs(data_dir: &Path, dry_run: bool) -> Result<Vec<DirectoryEntry>> {
    let dirs = install_dirs(data_dir);
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
        path: db_path(data_dir),
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
                number: RPC_PORT,
                protocol: PortProtocol::Tcp,
                service: RPC_SERVICE.into(),
            },
            PortEntry {
                number: HTTP_PORT,
                protocol: PortProtocol::Tcp,
                service: HTTP_SERVICE.into(),
            },
        ],
        os_keychain_entries: vec![],
        selected_extractors: serde_json::Value::Null,
        inbound_webhook_port: Some(INBOUND_WEBHOOK_PORT),
        backup_schedule: None,
    }
}

fn sign_manifest(
    data_dir: &Path,
    password: &str,
    manifest: InstallManifest,
) -> Result<InstallManifest> {
    let ks = FsKeystore::new(keystore_dir(data_dir));
    let kp = ks
        .load(MASTER_KEY_NAME, password)
        .context("master 키 로드 실패 — keystore 패스워드 확인")?;
    let signature = kp.sign(&manifest.canonical_bytes()?);
    let mut signed = manifest;
    signed.uninstall_token = hex::encode(signature);
    Ok(signed)
}
