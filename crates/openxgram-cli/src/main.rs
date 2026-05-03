//! xgram — OpenXgram command-line interface
//!
//! Phase 1: 명령 골격 (stub). 각 명령은 "Phase 1 구현 예정" 메시지를 출력합니다.
//! 실제 구현은 Phase 2 이후.

use clap::{Parser, Subcommand};

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
    /// 현재 머신에 OpenXgram을 초기화합니다
    Init {
        /// 에이전트 별칭 (예: akashic)
        #[arg(long)]
        alias: Option<String>,
        /// 데이터 디렉토리 경로 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<String>,
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

    /// OpenXgram을 제거합니다 (바이너리 및 데이터 삭제)
    Uninstall {
        /// 데이터 디렉토리도 함께 삭제
        #[arg(long)]
        purge: bool,
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
        Commands::Init { alias, data_dir } => {
            println!("xgram init");
            println!("  alias    : {}", alias.as_deref().unwrap_or("(not set)"));
            println!(
                "  data_dir : {}",
                data_dir.as_deref().unwrap_or("~/.openxgram (default)")
            );
            println!();
            println!("[Phase 1 구현 예정]");
            println!("  - secp256k1 HD 키페어 생성 (BIP-39 mnemonic)");
            println!("  - ~/.openxgram/ 디렉토리 초기화");
            println!("  - SQLite DB 생성 및 마이그레이션 적용");
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

        Commands::Uninstall { purge } => {
            println!("xgram uninstall");
            println!("  purge : {}", purge);
            println!();
            println!("[Phase 1 구현 예정]");
            println!("  - 데몬 프로세스 종료");
            if purge {
                println!("  - ~/.openxgram/ 디렉토리 삭제 (--purge)");
            }
            println!("  - xgram 바이너리 제거 안내");
        }
    }

    Ok(())
}
