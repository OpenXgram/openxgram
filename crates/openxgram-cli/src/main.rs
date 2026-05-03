//! xgram — OpenXgram command-line interface

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use openxgram_cli::init::{self, InitOpts};
use openxgram_cli::uninstall::{self, UninstallOpts};
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::MachineRole;

/// xgram — OpenXgram CLI
///
/// Memory & Credential Infrastructure for Multi-Agent Systems
#[derive(Parser, Debug)]
#[command(
    name = "xgram",
    version,
    about = "OpenXgram — Memory & Credential Infrastructure for Multi-Agent Systems",
    long_about = None
)]
struct Cli {
    /// 상세 로그 출력
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 현재 머신에 OpenXgram을 초기화합니다 (9단계 비대화 워크플로우, Phase 1: Step 1-6 + manifest)
    Init {
        /// 머신 별칭 (예: gcp-main)
        #[arg(long)]
        alias: String,
        /// 머신 역할
        #[arg(long, value_enum, default_value_t = RoleArg::Primary)]
        role: RoleArg,
        /// 데이터 디렉토리 경로 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 기존 설치 덮어쓰기
        #[arg(long)]
        force: bool,
        /// 실제 변경 없이 작업 목록만 출력 (keystore/DB/manifest 미생성)
        #[arg(long)]
        dry_run: bool,
        /// 다른 머신에서 시드를 import — XGRAM_SEED 환경변수 사용
        #[arg(long)]
        import: bool,
    },

    /// 현재 OpenXgram 상태를 출력합니다
    Status,

    /// 환경 진단을 실행합니다 (의존성, 설정, 연결 상태)
    Doctor,

    /// 모든 데이터를 초기화합니다 (주의: 복구 불가)
    Reset {
        /// 확인 없이 진행
        #[arg(long)]
        force: bool,
    },

    /// DB 마이그레이션을 실행합니다
    Migrate {
        /// 적용할 마이그레이션 버전 (기본: latest)
        #[arg(long)]
        target: Option<String>,
    },

    /// OpenXgram을 제거합니다 (Phase 1: --no-backup 비대화 모드)
    Uninstall {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 백업 없이 제거 (Phase 1 유일 지원 옵션)
        #[arg(long)]
        no_backup: bool,
        /// 확인 문자열 — --no-backup 시 "DELETE OPENXGRAM" 정확 일치 필요
        #[arg(long)]
        confirm: Option<String>,
        /// 실제 변경 없이 작업 미리보기
        #[arg(long)]
        dry_run: bool,
    },

    /// 키페어 관리
    Keypair {
        #[command(subcommand)]
        action: KeypairAction,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RoleArg {
    Primary,
    Secondary,
    Worker,
}

impl From<RoleArg> for MachineRole {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::Primary => Self::Primary,
            RoleArg::Secondary => Self::Secondary,
            RoleArg::Worker => Self::Worker,
        }
    }
}

#[derive(Subcommand, Debug)]
enum KeypairAction {
    /// 새 키페어 생성
    New {
        /// 키 이름 (예: eno, akashic)
        #[arg(long)]
        name: String,
        /// 암호화 패스워드 (미입력 시 빈 패스워드)
        #[arg(long, default_value = "")]
        password: String,
    },
    /// 저장된 키 목록 출력
    List,
    /// 키 상세 정보 출력 (주소, 경로, 생성일)
    Show {
        /// 키 이름
        name: String,
    },
    /// 니모닉으로 키 복원
    Import {
        /// 키 이름
        #[arg(long)]
        name: String,
        /// BIP39 니모닉 문구 (24단어)
        #[arg(long)]
        phrase: String,
        /// 암호화 패스워드
        #[arg(long, default_value = "")]
        password: String,
    },
    /// 니모닉 문구 내보내기 (패스워드 필요)
    Export {
        /// 키 이름
        name: String,
        /// 암호화 패스워드
        #[arg(long, default_value = "")]
        password: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // 로그 초기화
    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
        )
        .init();

    match cli.command {
        Commands::Init {
            alias,
            role,
            data_dir,
            force,
            dry_run,
            import,
        } => {
            let opts = InitOpts {
                alias,
                role: role.into(),
                data_dir: match data_dir {
                    Some(p) => p,
                    None => init::default_data_dir()?,
                },
                force,
                dry_run,
                import,
            };
            init::run_init(&opts)?;
        }

        Commands::Status => {
            println!("xgram status");
            println!();
            println!("[Phase 1 구현 예정]");
            println!("  - 에이전트 신원 (공개키, 별칭)");
            println!("  - 데몬 실행 상태");
            println!("  - DB 크기 및 메모리 레이어 통계");
            println!("  - Tailscale / XMTP 연결 상태");
        }

        Commands::Doctor => {
            println!("xgram doctor");
            println!();
            println!("[Phase 1 구현 예정]");
            println!("  - Rust 버전 확인");
            println!("  - ~/.openxgram/ 디렉토리 권한 확인");
            println!("  - SQLite WAL 모드 확인");
            println!("  - Tailscale 설치 여부 확인");
            println!("  - 네트워크 연결 테스트");
        }

        Commands::Reset { force } => {
            if force {
                println!("xgram reset --force");
                println!();
                println!("[Phase 1 구현 예정]");
                println!("  - ~/.openxgram/ 전체 삭제 후 재초기화");
            } else {
                println!("xgram reset");
                println!();
                println!("경고: 모든 데이터가 삭제됩니다. --force 플래그로 확인하세요.");
                println!("[Phase 1 구현 예정]");
            }
        }

        Commands::Migrate { target } => {
            println!("xgram migrate");
            println!(
                "  target : {}",
                target.as_deref().unwrap_or("latest (default)")
            );
            println!();
            println!("[Phase 1 구현 예정]");
            println!("  - 현재 DB 스키마 버전 확인");
            println!("  - 미적용 마이그레이션 순차 적용");
            println!("  - 적용 결과 보고");
        }

        Commands::Uninstall {
            data_dir,
            no_backup,
            confirm,
            dry_run,
        } => {
            let opts = UninstallOpts {
                data_dir: match data_dir {
                    Some(p) => p,
                    None => init::default_data_dir()?,
                },
                no_backup,
                confirm,
                dry_run,
            };
            uninstall::run_uninstall(&opts)?;
        }

        Commands::Keypair { action } => {
            let ks_dir = FsKeystore::default_path();
            let ks = FsKeystore::new(&ks_dir);
            handle_keypair(ks, action)?;
        }
    }

    Ok(())
}

fn handle_keypair(ks: FsKeystore, action: KeypairAction) -> anyhow::Result<()> {
    match action {
        KeypairAction::New { name, password } => {
            let (address, phrase) = ks.create(&name, &password)?;
            println!("키페어 생성 완료");
            println!("  이름    : {name}");
            println!("  주소    : {address}");
            println!("  경로    : m/44'/60'/0'/0/0");
            println!();
            println!("니모닉 (안전한 곳에 보관하세요 — 다시 표시되지 않습니다):");
            println!("  {phrase}");
        }
        KeypairAction::List => {
            let entries = ks.list()?;
            if entries.is_empty() {
                println!("저장된 키가 없습니다.");
                println!("  xgram keypair new --name <이름> 으로 키를 생성하세요.");
            } else {
                println!("저장된 키 ({} 개):", entries.len());
                for e in &entries {
                    println!("  {} — {}", e.name, e.address);
                }
            }
        }
        KeypairAction::Show { name } => {
            let entries = ks.list()?;
            let entry = entries
                .iter()
                .find(|e| e.name == name)
                .ok_or_else(|| anyhow::anyhow!("키를 찾을 수 없습니다: {name}"))?;
            println!("키 정보: {}", entry.name);
            println!("  주소          : {}", entry.address);
            println!("  파생 경로     : {}", entry.derivation_path);
            println!("  생성일        : {}", entry.created_at);
        }
        KeypairAction::Import {
            name,
            phrase,
            password,
        } => {
            let address = ks.import(&name, &phrase, &password)?;
            println!("키 복원 완료");
            println!("  이름    : {name}");
            println!("  주소    : {address}");
        }
        KeypairAction::Export { name, password } => {
            // Export: 패스워드로 복호화 후 니모닉을 다시 보여줄 수 없음
            // (암호화 저장에 니모닉 원문이 없고 비밀키만 있음)
            // 대신 공개 주소와 공개키를 출력
            let kp = ks.load(&name, &password)?;
            println!("키 정보 (공개): {name}");
            println!("  주소       : {}", kp.address);
            println!("  공개키(압축): {}", hex::encode(kp.public_key_bytes()));
            println!();
            println!("주의: 니모닉은 최초 생성 시에만 표시됩니다.");
        }
    }
    Ok(())
}
