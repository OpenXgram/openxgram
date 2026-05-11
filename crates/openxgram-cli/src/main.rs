//! xgram — OpenXgram command-line interface

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use openxgram_cli::audit::{self, AuditAction};
use openxgram_cli::backup::{
    create_cold_backup, resolve_backup_target, restore_cold_backup, restore_cold_backup_merge,
};
use openxgram_cli::backup_push::{self, BackupPushOpts, BackupTarget};
use openxgram_cli::channel::{self, ChannelAction};
use openxgram_cli::daemon::{self, DaemonOpts};
use openxgram_cli::doctor::{self, DoctorOpts};
use openxgram_cli::dump;
use openxgram_cli::init::{self, InitOpts};
use openxgram_cli::mcp_serve;
use openxgram_cli::memory::{self, MemoryAction};
use openxgram_cli::migrate::{self, MigrateOpts};
use openxgram_cli::notify::{self, ChannelMode, NotifyAction};
use openxgram_cli::notify_setup::{self, SetupOpts, SetupTarget};
use openxgram_cli::orchestration::{self, ChainAction, ScheduleAction};
use openxgram_cli::patterns::{self, PatternsAction};
use openxgram_cli::payment::{self, PaymentAction};
use openxgram_cli::peer::{self, PeerAction};
use openxgram_cli::peer_send;
use openxgram_cli::reset::{self, ResetOpts};
use openxgram_cli::session::{self, SessionAction};
use openxgram_cli::status::{self, StatusOpts};
use openxgram_cli::systemd::{self, UnitOpts};
use openxgram_cli::traits::{self, TraitsAction};
use openxgram_cli::tui::{self, TuiOpts};
use openxgram_cli::uninstall::{self, UninstallOpts};
use openxgram_cli::vault::{self, VaultAction};
use openxgram_cli::wizard;
use openxgram_keystore::{FsKeystore, Keystore};
use openxgram_manifest::MachineRole;
use openxgram_memory::{Classification, TraitSource};

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
    /// 현재 머신에 OpenXgram을 초기화합니다.
    ///
    /// 플래그 없이 실행하면 인터랙티브 마법사(`xgram wizard`)로 자동 진입.
    /// `--alias`를 넘기면 비대화 모드.
    Init {
        /// 머신 별칭 (예: gcp-main). 미지정 시 인터랙티브 마법사로 진입.
        #[arg(long)]
        alias: Option<String>,
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

    /// 현재 OpenXgram 상태를 출력합니다 (manifest 기반)
    Status {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// 환경 진단을 실행합니다 (Phase 1: manifest·DB·keystore·drift·transport 점검)
    Doctor {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// JSON 형식으로 출력 (다른 도구 통합용)
        #[arg(long)]
        json: bool,
    },

    /// 모든 데이터를 초기화합니다 (Phase 1: --hard, 주의: 복구 불가)
    Reset {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 데이터 + 키 모두 삭제 (Phase 1 유일 지원 옵션)
        #[arg(long)]
        hard: bool,
        /// 확인 문자열 — --hard 시 "RESET OPENXGRAM" 정확 일치 필요
        #[arg(long)]
        confirm: Option<String>,
        /// 실제 변경 없이 작업 미리보기
        #[arg(long)]
        dry_run: bool,
    },

    /// DB 마이그레이션을 실행합니다 (Phase 1: latest 까지)
    Migrate {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 적용할 마이그레이션 버전 (Phase 1.5+ 지원, 현재는 무시)
        #[arg(long)]
        target: Option<String>,
    },

    /// OpenXgram을 제거합니다 (cold backup 또는 --no-backup)
    Uninstall {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// Cold backup 대상 경로 (ChaCha20-Poly1305 + tar.gz). XGRAM_KEYSTORE_PASSWORD 사용.
        #[arg(long)]
        cold_backup_to: Option<PathBuf>,
        /// 백업 없이 제거 (cold-backup-to 와 상호 배타)
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
        /// 데이터 디렉토리 (생략 시 ~/.openxgram). keystore 위치 결정.
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: KeypairAction,
    },

    /// W3C DID + 한국 OpenDID + OmniOne Open DID 호환 신원 (did/did-document/issue-vc/verify-vc)
    Identity {
        #[command(subcommand)]
        action: openxgram_cli::identity::IdentityCli,
    },

    /// 대화 session 관리 (new/list/show/message/reflect)
    Session {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: SessionCli,
    },

    /// L2 memories (fact/decision/reference/rule) 관리
    Memory {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: MemoryCli,
    },

    /// Discord/Telegram 으로 텍스트 알림 전송
    Notify {
        #[command(subcommand)]
        target: NotifyCli,
    },

    /// 외부 채널 (Discord/Telegram 등) 한 번에 연결 — 인터랙티브 마법사.
    /// `xgram setup discord` / `xgram setup telegram` — 토큰 입력 → 검증 → vault 저장 → 테스트 메시지까지.
    /// (별칭: `xgram notify setup-discord` / `notify setup-telegram` — 동일 동작)
    Setup {
        #[command(subcommand)]
        target: SetupCli,
    },

    /// session 통계 백업을 Discord/Telegram 으로 push
    BackupPush {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        session_id: String,
        #[arg(long, value_enum)]
        target: BackupTargetArg,
    },

    /// 메인 에이전트 런타임 (Phase 1 v1) — inbox 폴링 + 처리 + 채널 forward.
    ///
    /// daemon 이 inbox-* 세션에 저장한 inbound 메시지를 폴링해서
    /// 콘솔 로그 + Discord webhook outbound (옵션) 으로 전달.
    /// daemon 과 같은 머신에서 별도 프로세스로 가동.
    Agent {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// Discord webhook URL (outbound forward). XGRAM_DISCORD_WEBHOOK_URL env 폴백.
        #[arg(long)]
        discord_webhook_url: Option<String>,
        /// Discord bot token (inbound polling). XGRAM_DISCORD_BOT_TOKEN env 폴백.
        #[arg(long)]
        discord_bot_token: Option<String>,
        /// Discord channel id (inbound polling). XGRAM_DISCORD_CHANNEL_ID env 폴백.
        #[arg(long)]
        discord_channel_id: Option<String>,
        /// Anthropic API key (LLM 응답 활성). XGRAM_ANTHROPIC_API_KEY env 폴백.
        #[arg(long)]
        anthropic_api_key: Option<String>,
        /// Telegram bot token (옵션). XGRAM_TELEGRAM_BOT_TOKEN env 폴백.
        #[arg(long)]
        telegram_bot_token: Option<String>,
        /// Telegram chat id (회신 대상). XGRAM_TELEGRAM_CHAT_ID env 폴백.
        #[arg(long)]
        telegram_chat_id: Option<String>,
        /// 폴링 주기 (초)
        #[arg(long, default_value_t = 5)]
        poll_interval_secs: u64,
    },

    /// 사이드카 데몬 — scheduler + transport server foreground 실행
    Daemon {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// transport bind 주소 (기본 127.0.0.1:47300, --tailscale 우선)
        #[arg(long)]
        bind: Option<std::net::SocketAddr>,
        /// GUI HTTP API (`/v1/gui/*`) bind 주소. 기본 127.0.0.1:47302.
        /// Tauri 데스크톱 앱·기타 클라이언트가 원격 daemon 데이터에 접근.
        /// Tailscale IP 로 bind 시 mcp-token 인증 필수 (loopback 도 동일 — silent 노출 금지).
        #[arg(long)]
        gui_bind: Option<std::net::SocketAddr>,
        /// reflection cron 표현식 (기본 0 0 15 * * * = 자정 KST)
        #[arg(long)]
        reflection_cron: Option<String>,
        /// tailscale IPv4 로 자동 bind (WireGuard mTLS 활용 — PRD §15)
        #[arg(long)]
        tailscale: bool,
    },

    /// systemd user unit 생성/제거 (~/.config/systemd/user/openxgram-sidecar.service)
    DaemonInstall {
        /// xgram binary 경로 (기본: 현재 실행 중인 binary)
        #[arg(long)]
        binary: Option<PathBuf>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long, default_value = "127.0.0.1:47300")]
        bind: String,
        /// unit 파일 출력 경로 (기본: ~/.config/systemd/user/openxgram-sidecar.service)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    DaemonUninstall {
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// systemd backup .service + .timer 생성 (주기 cold backup 자동화)
    BackupInstall {
        #[arg(long)]
        binary: Option<PathBuf>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// cold backup 출력 디렉토리 (timestamped 파일 생성)
        #[arg(long)]
        backup_dir: PathBuf,
        /// systemd OnCalendar 표현식 (기본 "Sun 03:00:00")
        #[arg(long)]
        on_calendar: Option<String>,
    },
    BackupUninstall,

    /// MCP JSON-RPC 서버 — Claude Code 통합용 (stdio 또는 --bind 시 HTTP)
    McpServe {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// HTTP transport bind 주소 (예: 127.0.0.1:47301). 생략 시 stdio.
        #[arg(long)]
        bind: Option<std::net::SocketAddr>,
    },

    /// MCP HTTP caller 인증 토큰 관리 (Bearer)
    McpToken {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: McpTokenCli,
    },

    /// 암호화 자격증명 vault (PRD §8) — set/get/list/delete
    Vault {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: VaultCli,
    },

    /// L4 traits (정체성·성향) — set/get/list (manual source 만 CLI 노출)
    Traits {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: TraitsCli,
    },

    /// L3 patterns (행동/발화 분류) — observe/list (NEW/RECURRING/ROUTINE)
    Patterns {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: PatternsCli,
    },

    /// 인터랙티브 init 마법사 (state machine — Welcome/MachineId/Confirm/Done)
    Wizard,

    /// cold backup 파일 복원 — ChaCha20-Poly1305 복호화 + tar.gz 해제
    Restore {
        /// 백업 파일 경로
        #[arg(long)]
        input: PathBuf,
        /// 복원 대상 데이터 디렉토리 (기본: ~/.openxgram, 비어있어야 함)
        #[arg(long)]
        target_dir: Option<PathBuf>,
        /// 비어있지 않은 디렉토리에 덮어쓰기 (충돌 파일 = 백업 우선, 백업에 없는 파일 보존)
        #[arg(long)]
        merge: bool,
    },

    /// 비파괴 cold backup 생성 — ChaCha20-Poly1305 + tar.gz. systemd timer/cron 으로 주기 실행 권장.
    Backup {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 백업 파일 경로 (디렉토리면 timestamped 파일 생성, 파일이면 정확 그 경로)
        #[arg(long)]
        to: PathBuf,
    },

    /// 인터랙티브 TUI (welcome + status)
    Tui {
        /// 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// GUI(Tauri) 데스크톱 앱 실행 — 별 바이너리 `xgram-desktop` 호출
    Gui {
        /// xgram-desktop 에 그대로 전달할 추가 인자
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// 데스크탑 페어링 — `oxg://alias@host:port#token=xxx` URL 로 원격 daemon 연결.
    /// 이후 GUI/CLI 가 환경변수 없이도 자동으로 원격 daemon 사용.
    Link {
        /// 페어링 URL (서버의 install.sh / `xgram pair-desktop` 출력)
        url: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// 서버측 데스크탑 페어링 — Tailscale IP + mcp-token 발급 + `oxg://...` URL 출력.
    /// 데스크탑에서 받은 URL 을 `xgram link <url>` 로 적용.
    PairDesktop {
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Peer 레지스트리 — 머신 간 통신 주소록 (cross-machine 메시지 baseline)
    Peer {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: PeerCli,
    },

    /// USDC payment intent — PRD §16 인프라 (실 on-chain 제출은 후속)
    Payment {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: PaymentCli,
    },

    /// audit chain 무결성·체크포인트 관리 (PRD-AUDIT-03)
    Audit {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        cmd: AuditCli,
    },

    /// 자체 호스팅 Nostr relay (PRD-NOSTR-06) — 다른 머신과 메시지 중계
    Relay {
        /// bind 주소 (default 127.0.0.1)
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// bind 포트 (default 7400)
        #[arg(long, default_value_t = openxgram_nostr::DEFAULT_RELAY_PORT)]
        port: u16,
        /// NIP-13 PoW 최소 difficulty (0~32)
        #[arg(long)]
        min_pow: Option<u8>,
        /// 동시 연결 제한
        #[arg(long)]
        max_connections: Option<usize>,
    },

    /// 예약 메시지 — 특정 시각 또는 cron 표현식으로 미래 전송 (PRD-ORCH-01)
    Schedule {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: ScheduleAction,
    },

    /// 메시지 체인 — 순차 단계 + 조건 분기 (PRD-ORCH-01)
    Chain {
        #[arg(long, global = true)]
        data_dir: Option<PathBuf>,
        #[command(subcommand)]
        action: ChainAction,
    },

    /// 쉘 자동 완성 스크립트 출력 (bash/zsh/fish/elvish/powershell)
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// JSON 통합 출력 — Tauri/스크립트/Prometheus 친화. kind: sessions/episodes/memories/patterns/traits/vault/acl/pending/peers/payments/mcp-tokens
    Dump {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        kind: String,
    },

    /// 빌드 정보 출력 (버전 / target / 활성 feature / 의존 crate)
    Version {
        /// JSON 으로 출력 (다른 도구 통합용)
        #[arg(long)]
        json: bool,
    },

    /// 내장 Channel MCP 서버 (다중 에이전트 채팅 라우팅 허브) — Starian channel-mcp 호환
    Channel {
        #[command(subcommand)]
        cmd: ChannelCli,
    },

    /// 다른 LLM 에 붙여넣어 자동 온보딩을 시작하는 프롬프트 출력
    Onboard {
        #[command(subcommand)]
        cmd: OnboardCli,
    },

    /// 1머신 N봇 — 봇 추가/목록/제거/링크 (a 작업)
    Bot {
        #[command(subcommand)]
        cmd: BotCli,
    },

    /// MCP 서버 자기 자신을 Claude Code (.claude.json) 또는 프로젝트 (.mcp.json) 에 등록
    McpInstall {
        /// user(~/.claude.json) / project(./.mcp.json) / 임의 경로
        #[arg(long, value_enum, default_value_t = McpInstallScope::Project)]
        scope: McpInstallScope,
        /// scope=custom 일 때 사용할 경로
        #[arg(long)]
        config: Option<PathBuf>,
        /// data_dir 경로 (생략 시 ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// XGRAM_KEYSTORE_PASSWORD 를 config 에 평문 포함 (편리 / 보안 trade-off)
        #[arg(long)]
        with_password: bool,
    },

    /// 프로젝트 CLAUDE.md 에 OpenXgram identity context 블록 주입/갱신 (idempotent)
    IdentityInject {
        /// 주입 대상 파일 (기본: ./CLAUDE.md)
        #[arg(long, default_value = "CLAUDE.md")]
        target: PathBuf,
        /// data_dir 경로 (생략 시 ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// CLAUDE.md 에서 OpenXgram identity 블록 제거 (uninstall)
    IdentityUninject {
        #[arg(long, default_value = "CLAUDE.md")]
        target: PathBuf,
    },

    /// 친구 초대 URL + QR 출력 (oxg-friend://)
    Invite {
        /// 데이터 디렉토리 (기본 ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// alias 표시 (없으면 manifest 의 머신 alias)
        #[arg(long)]
        alias: Option<String>,
        /// 외부 접속 가능한 transport address (예: http://1.2.3.4:47300)
        #[arg(long, default_value = "http://127.0.0.1:47300")]
        address: String,
    },

    /// 친구 추가 — 받은 oxg-friend://... URL 로 양방향 peer 등록
    Friend {
        #[command(subcommand)]
        cmd: FriendCli,
    },

    /// HITL — 봇이 사람한테 물어본 질문 목록 / 응답 (d 작업)
    Human {
        #[command(subcommand)]
        cmd: HumanCli,
    },

    /// 채널 등록 (Discord / Telegram / xgram-peer / ...) — 다채널 라우팅 (e 작업)
    Channels {
        #[command(subcommand)]
        cmd: ChannelsCli,
    },

    /// 핸들 → 채널 디렉터리 cache 관리 (e 작업)
    Directory {
        #[command(subcommand)]
        cmd: DirectoryCli,
    },

    /// `xgram find @<h>` — 핸들 검색 (directory cache → ENS resolver → indexer)
    Find {
        /// `@handle` 또는 keyword
        query: String,
        /// indexer URL (--indexer 사용 시 indexer 의 search 결과)
        #[arg(long)]
        indexer: Option<String>,
    },

    /// `xgram openagentx call <agent> <prompt> [--pay <micros>]` — OpenAgentX 마켓 에이전트 호출 (step 11/12)
    Openagentx {
        #[command(subcommand)]
        cmd: OpenagentxCli,
    },

    /// `xgram eas list/count/attest` — EAS 어테스테이션 (step 18)
    Eas {
        #[command(subcommand)]
        cmd: EasCli,
    },

    /// `xgram send @<h> <body>` — 다채널 자동 라우팅 (e 작업 마무리)
    Send {
        /// `@handle`
        handle: String,
        /// 메시지 본문
        body: String,
        /// 강제 채널 종류 (discord / telegram / xgram-peer / ...)
        #[arg(long)]
        kind: Option<String>,
        /// 동일 thread 로 묶을 conversation_id
        #[arg(long)]
        conversation_id: Option<String>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum EasCli {
    /// 최근 attestation 목록
    List {
        #[arg(long, default_value = "20")]
        limit: usize,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// kind 별 카운트
    Count {
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// 수동 attestation 추가 (시연용)
    Attest {
        /// kind: message | payment | endorsement
        #[arg(long)]
        kind: String,
        /// fields JSON
        fields: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum OpenagentxCli {
    /// 마켓 에이전트 호출 — 응답 받기 + (옵션) USDC 결제 draft 생성
    Call {
        /// agent id (예: "@translator-pro")
        agent: String,
        /// prompt
        prompt: String,
        /// 유료 호출 시 micro USDC (1 USDC = 1_000_000)
        #[arg(long)]
        pay: Option<u64>,
        /// 결제 메모
        #[arg(long)]
        memo: Option<String>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum ChannelsCli {
    /// 본 노드의 채널 추가
    Add {
        kind: String,
        address: String,
        #[arg(long, default_value = "public")]
        visibility: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// 등록된 채널 목록
    List {
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// 채널 제거
    Remove {
        kind: String,
        address: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum DirectoryCli {
    /// 핸들의 채널 lookup
    Lookup { handle: String },
    /// 핸들의 채널 수동 등록 (cache 갱신)
    Set {
        handle: String,
        /// JSON: [{"kind":"discord","address":"..","visibility":"public"}, ...]
        channels_json: String,
    },
    /// step 13 — 자기 봇을 외부 디렉터리에 등록 ("홍보")
    Register {
        /// indexer 서비스 URL (예: https://openxgram.org/registry)
        #[arg(long)]
        to: String,
        /// 평판 카운트 동봉 (messages / payments / endorsements)
        #[arg(long)]
        with_counts: bool,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum HumanCli {
    /// 미응답 HITL 요청 목록
    Pending {
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// 봇 질문에 응답
    Respond {
        request_id: String,
        answer: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum FriendCli {
    /// 받은 invite URL 로 친구 추가 (자동 handshake)
    Accept {
        /// oxg-friend:// URL
        url: String,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum McpInstallScope {
    /// ~/.claude.json — 모든 프로젝트
    User,
    /// ./.mcp.json — 이 프로젝트만
    Project,
    /// 임의 경로 (--config 필요)
    Custom,
}

#[derive(Subcommand, Debug)]
enum BotCli {
    /// 새 봇 등록 (data_dir 자동 생성, 포트 자동 할당, 레지스트리 갱신)
    Add {
        /// 봇 이름 (영숫자/-/_)
        name: String,
        /// 봇 alias (생략 시 name 그대로)
        #[arg(long)]
        alias: Option<String>,
    },
    /// 등록된 봇 목록
    List,
    /// 봇 제거 (data_dir 까지 삭제)
    Remove {
        name: String,
        /// 가동 중이어도 강제 종료 + 제거
        #[arg(long)]
        force: bool,
    },
    /// 봇 가동 (data_dir 의 daemon + agent 백그라운드 spawn)
    Start { name: String },
    /// 봇 종료 (TERM)
    Stop { name: String },
    /// 두 봇을 양방향 peer 로 등록 (같은 머신 내)
    Link { a: String, b: String },
    /// 한 번에 add + init + auto-link + start (interactive prompt 없음, XGRAM_KEYSTORE_PASSWORD env 필요)
    Register {
        /// 봇 이름 (영숫자/-/_)
        name: String,
        /// 봇 alias (생략 시 name 그대로)
        #[arg(long)]
        alias: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum OnboardCli {
    /// 온보딩 프롬프트를 출력 (Claude/ChatGPT/Gemini 등에 붙여넣기)
    Prompt {
        /// 출력 언어 (ko / en / both, 기본 ko)
        #[arg(long, value_enum, default_value_t = openxgram_cli::onboard::OnboardLang::Ko)]
        lang: openxgram_cli::onboard::OnboardLang,
        /// 클립보드에 직접 복사 (wl-copy / xclip / xsel / pbcopy / clip.exe 자동 감지)
        #[arg(long)]
        copy: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ChannelCli {
    /// HTTP 서버 기동 (loopback 만 허용 — 절대 규칙)
    Serve {
        /// 바인딩 주소 (기본 127.0.0.1:7250). 0.0.0.0 등 외부 바인딩 금지.
        #[arg(long, default_value = "127.0.0.1:7250")]
        bind: String,
        /// Bearer 인증 토큰 (생략 시 인증 없음 — loopback 만 신뢰)
        #[arg(long)]
        auth_token: Option<String>,
    },
    /// 기동 중인 서버에 메시지 전송 (role 라우팅 또는 platform 직접)
    Send {
        #[arg(long, default_value = "http://127.0.0.1:7250")]
        server: String,
        #[arg(long)]
        auth_token: Option<String>,
        /// peer registry 의 role (예: eno / qua / master)
        #[arg(long)]
        to_role: Option<String>,
        /// platform 직접 (discord / telegram / slack / kakaotalk)
        #[arg(long)]
        platform: Option<String>,
        #[arg(long)]
        channel_id: Option<String>,
        #[arg(long)]
        text: String,
        #[arg(long)]
        reply_to: Option<String>,
        #[arg(long, default_value = "info")]
        msg_type: String,
    },
    /// 등록된 어댑터 목록
    ListAdapters {
        #[arg(long, default_value = "http://127.0.0.1:7250")]
        server: String,
        #[arg(long)]
        auth_token: Option<String>,
    },
    /// 등록된 peer (역할별) 목록
    ListPeers {
        #[arg(long, default_value = "http://127.0.0.1:7250")]
        server: String,
        #[arg(long)]
        auth_token: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCli {
    /// 새 session 생성
    New {
        #[arg(long)]
        title: String,
    },
    /// session 목록
    List,
    /// session 상세 (episodes 포함)
    Show {
        /// session ID
        id: String,
    },
    /// session 에 메시지 추가
    Message {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        sender: String,
        #[arg(long)]
        body: String,
    },
    /// session 의 messages 를 episode 로 reflection
    Reflect {
        #[arg(long)]
        session_id: String,
    },
    /// 쿼리와 가장 유사한 메시지 K 개 검색 (sqlite-vec KNN)
    Recall {
        #[arg(long)]
        query: String,
        #[arg(long, default_value_t = 5)]
        k: usize,
    },
    /// session 통째로 export — JSON text-package-v1 (PRD §17)
    Export {
        #[arg(long)]
        session_id: String,
        /// 출력 파일 경로 (생략 시 stdout)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// text-package-v1 JSON 을 새 session 으로 import (PRD §20 F)
    Import {
        /// 입력 파일 경로 (생략 시 stdin)
        #[arg(long)]
        input: Option<PathBuf>,
        /// 메시지 signature 를 master_public_key 로 검증 (불일치 시 raise)
        #[arg(long)]
        verify: bool,
    },
    /// session 삭제 (messages·episodes CASCADE, memories.session_id NULL)
    Delete { id: String },
    /// 모든 session 에 reflection 일괄 실행 (cron 전 단계)
    ReflectAll,
    /// step 5/6 — 다른 LLM 앱 (Claude/ChatGPT/Gemini) 에 복붙 가능한 transcript 출력
    Transcript {
        session_id: String,
        /// 출력 포맷 — claude / chatgpt / gemini / plain (기본 plain)
        #[arg(long, default_value = "plain")]
        format: String,
    },
    /// step 5/6/8 — AI 앱 export 파일을 OpenXgram session 으로 import
    ImportApp {
        /// 파일 경로 (예: conversations.json / MyActivity.json / session.jsonl)
        file: PathBuf,
        /// 포맷 — chatgpt | gemini | claude-code
        #[arg(long)]
        format: String,
        /// (옵션) session 제목 override
        #[arg(long)]
        title: Option<String>,
    },
}

impl From<SessionCli> for SessionAction {
    fn from(c: SessionCli) -> Self {
        match c {
            SessionCli::New { title } => SessionAction::New { title },
            SessionCli::List => SessionAction::List,
            SessionCli::Show { id } => SessionAction::Show { id },
            SessionCli::Message {
                session_id,
                sender,
                body,
            } => SessionAction::Message {
                session_id,
                sender,
                body,
            },
            SessionCli::Reflect { session_id } => SessionAction::Reflect { session_id },
            SessionCli::Recall { query, k } => SessionAction::Recall { query, k },
            SessionCli::Export { session_id, out } => SessionAction::Export { session_id, out },
            SessionCli::Import { input, verify } => SessionAction::Import { input, verify },
            SessionCli::Delete { id } => SessionAction::Delete { id },
            SessionCli::ReflectAll => SessionAction::ReflectAll,
            SessionCli::Transcript { session_id, format } => {
                SessionAction::Transcript { session_id, format }
            }
            // ImportApp 은 SessionAction 변환 안 함 — main dispatch 에서 직접 처리.
            SessionCli::ImportApp { .. } => unreachable!("ImportApp 은 dispatch 에서 직접 처리"),
        }
    }
}

#[derive(Subcommand, Debug)]
enum NotifyCli {
    /// Discord webhook (송신)
    Discord {
        /// Webhook URL (생략 시 DISCORD_WEBHOOK_URL 환경변수)
        #[arg(long)]
        webhook_url: Option<String>,
        #[arg(long)]
        text: String,
    },
    /// Telegram bot 송신 (sendMessage)
    Telegram {
        /// Bot token (생략 시 TELEGRAM_BOT_TOKEN 환경변수)
        #[arg(long)]
        bot_token: Option<String>,
        /// Chat ID (생략 시 TELEGRAM_CHAT_ID 환경변수)
        #[arg(long)]
        chat_id: Option<String>,
        #[arg(long)]
        text: String,
    },
    /// Discord Gateway 봇 — 채널/DM 메시지 수신 (WebSocket).
    /// 다중 에이전트 시스템에서 채팅방 허브 역할.
    DiscordListen {
        /// Bot token (생략 시 DISCORD_BOT_TOKEN 환경변수)
        #[arg(long)]
        bot_token: Option<String>,
        /// 특정 channel_id 만 수신 (없으면 봇이 속한 모든 channel + DM)
        #[arg(long)]
        channel_id: Option<u64>,
        /// 받은 메시지를 L0 messages 로 저장. session ID 또는 title 을 넘긴다
        /// (`xgram session new --title <NAME>` 로 미리 생성).
        #[arg(long)]
        store_session: Option<String>,
        /// 데이터 디렉토리 (store-session 사용 시; 기본 ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 사람 친화 출력. 미지정 시 한 줄 JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Telegram bot 받기 (long-polling, 양방향). 옵션으로 L0 message 저장.
    TelegramListen {
        /// Bot token (생략 시 TELEGRAM_BOT_TOKEN 환경변수)
        #[arg(long)]
        bot_token: Option<String>,
        /// 이 chat_id 에서 온 메시지만 통과 (생략 시 모든 chat 수신)
        #[arg(long)]
        chat_id: Option<i64>,
        /// 지정 시 OpenXgram L0 message 로 저장. session title 로 사용.
        #[arg(long)]
        store_session: Option<String>,
        /// L0 저장 시 데이터 디렉토리 (기본: ~/.openxgram)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 한 번만 polling 후 종료 (테스트·debug 용)
        #[arg(long, default_value_t = false)]
        once: bool,
    },
    /// Telegram 인터랙티브 마법사 — 토큰 검증 + chat_id 자동 감지 + 저장 + 테스트
    SetupTelegram {
        /// `~/.openxgram` 대신 임의 경로 (테스트용)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Discord 인터랙티브 마법사 — 토큰 검증 + 채널/webhook 입력 + 저장 + 테스트
    SetupDiscord {
        /// `~/.openxgram` 대신 임의 경로 (테스트용)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Starian Channel MCP HTTP gateway 호출 (다중 에이전트 메시지 라우팅 허브).
    ///
    /// 모드 (배타):
    /// - `--platform <p> --channel-id <c> --text <t>` : send_to_platform
    /// - `--to-role <r> --summary <s> [--type <t>]`   : send_message (피어 라우팅)
    /// - `--list-adapters`                            : 등록 어댑터 목록
    Channel {
        /// channel-mcp gateway URL (기본 OPENXGRAM_CHANNEL_MCP_URL)
        #[arg(long)]
        mcp_url: Option<String>,
        /// 선택 bearer 토큰 (기본 OPENXGRAM_CHANNEL_MCP_TOKEN)
        #[arg(long)]
        auth_token: Option<String>,

        /// send_to_platform 모드 — 플랫폼 (discord/telegram/slack/kakaotalk/webhook)
        #[arg(long, conflicts_with_all = ["to_role", "list_adapters"])]
        platform: Option<String>,
        /// send_to_platform 모드 — 채널 ID (discord channel id, telegram chat id 등)
        #[arg(long, requires = "platform")]
        channel_id: Option<String>,
        /// send_to_platform / send_message 의 메시지 본문
        #[arg(long)]
        text: Option<String>,
        /// 답글 대상 메시지 ID (선택, send_to_platform 만)
        #[arg(long)]
        reply_to: Option<String>,

        /// send_message 모드 — 대상 역할명 (master/starian/res/eno/...)
        #[arg(long, conflicts_with_all = ["platform", "list_adapters"])]
        to_role: Option<String>,
        /// send_message 모드 — 한 줄 요약
        #[arg(long, requires = "to_role")]
        summary: Option<String>,
        /// send_message 모드 — request|result|info|alert (기본 info)
        #[arg(long, default_value = "info")]
        msg_type: String,

        /// list_adapters 모드 — 등록된 어댑터 출력
        #[arg(long, default_value_t = false)]
        list_adapters: bool,
    },
}

/// `xgram setup <target>` — Discord/Telegram 연결 인터랙티브 마법사 단일 진입점.
/// `xgram notify setup-discord` / `notify setup-telegram` 와 동일한 마법사를 호출 (호환 유지).
#[derive(Subcommand, Debug)]
enum SetupCli {
    /// Discord 봇 연결 — 토큰 검증 + 채널/webhook 입력 + vault 저장 + 테스트 메시지
    Discord {
        /// `~/.openxgram` 대신 임의 경로 (테스트용)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Telegram 봇 연결 — 토큰 검증 + chat_id 자동 감지 + vault 저장 + 테스트 메시지
    Telegram {
        /// `~/.openxgram` 대신 임의 경로 (테스트용)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

/// 송신·수신 서브명령은 `NotifyAction` 으로, setup-* 마법사는 직접 dispatch.
enum NotifyDispatch {
    Action(NotifyAction),
    Setup(SetupTarget, SetupOpts),
}

impl From<NotifyCli> for NotifyDispatch {
    fn from(c: NotifyCli) -> Self {
        match c {
            NotifyCli::Discord { webhook_url, text } => {
                NotifyDispatch::Action(NotifyAction::Discord { webhook_url, text })
            }
            NotifyCli::Telegram {
                bot_token,
                chat_id,
                text,
            } => NotifyDispatch::Action(NotifyAction::Telegram {
                bot_token,
                chat_id,
                text,
            }),
            NotifyCli::DiscordListen {
                bot_token,
                channel_id,
                store_session,
                data_dir,
                pretty,
            } => NotifyDispatch::Action(NotifyAction::DiscordListen {
                bot_token,
                channel_id,
                store_session,
                data_dir,
                pretty,
            }),
            NotifyCli::TelegramListen {
                bot_token,
                chat_id,
                store_session,
                data_dir,
                once,
            } => NotifyDispatch::Action(NotifyAction::TelegramListen {
                bot_token,
                chat_id_filter: chat_id,
                store_session_title: store_session,
                data_dir,
                once,
            }),
            NotifyCli::SetupTelegram { data_dir } => NotifyDispatch::Setup(
                SetupTarget::Telegram,
                SetupOpts {
                    data_dir,
                    detect_attempts: None,
                },
            ),
            NotifyCli::SetupDiscord { data_dir } => NotifyDispatch::Setup(
                SetupTarget::Discord,
                SetupOpts {
                    data_dir,
                    detect_attempts: None,
                },
            ),
            NotifyCli::Channel {
                mcp_url,
                auth_token,
                platform,
                channel_id,
                text,
                reply_to,
                to_role,
                summary,
                msg_type,
                list_adapters,
            } => {
                let mode = build_channel_mode(
                    list_adapters,
                    platform,
                    channel_id,
                    text,
                    reply_to,
                    to_role,
                    summary,
                    msg_type,
                );
                NotifyDispatch::Action(NotifyAction::Channel {
                    mcp_url,
                    auth_token,
                    mode,
                })
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_channel_mode(
    list_adapters: bool,
    platform: Option<String>,
    channel_id: Option<String>,
    text: Option<String>,
    reply_to: Option<String>,
    to_role: Option<String>,
    summary: Option<String>,
    msg_type: String,
) -> ChannelMode {
    if list_adapters {
        return ChannelMode::ListAdapters;
    }
    if let Some(pf) = platform {
        let cid = channel_id.unwrap_or_else(|| {
            eprintln!("xgram notify channel: --platform 사용 시 --channel-id 필요");
            std::process::exit(2);
        });
        let body = text.unwrap_or_else(|| {
            eprintln!("xgram notify channel: --platform 사용 시 --text 필요");
            std::process::exit(2);
        });
        return ChannelMode::Platform {
            platform: pf,
            channel_id: cid,
            text: body,
            reply_to,
        };
    }
    if let Some(role) = to_role {
        let s = summary.unwrap_or_else(|| {
            eprintln!("xgram notify channel: --to-role 사용 시 --summary 필요");
            std::process::exit(2);
        });
        return ChannelMode::Peer {
            to_role: role,
            summary: s,
            msg_type,
        };
    }
    eprintln!(
        "xgram notify channel: 모드를 지정하세요 \
         (--platform | --to-role | --list-adapters)"
    );
    std::process::exit(2);
}

#[derive(Subcommand, Debug)]
enum MemoryCli {
    /// L2 memory 추가
    Add {
        #[arg(long, value_enum)]
        kind: MemoryKindArg,
        #[arg(long)]
        content: String,
        #[arg(long)]
        session_id: Option<String>,
    },
    /// list (pinned 우선). `--kind` 없으면 모든 kind 출력.
    List {
        #[arg(long, value_enum)]
        kind: Option<MemoryKindArg>,
    },
    /// memory pin
    Pin { id: String },
    /// memory unpin
    Unpin { id: String },
    /// 같은 conversation_id 로 묶인 모든 메시지 출력 (timestamp 오름차순)
    Show {
        /// conversation_id (UUID)
        #[arg(long)]
        conversation: String,
    },
    /// L2 memories + L4 traits 를 Claude 호환 markdown 으로 export
    Export {
        /// 결과 파일 경로 (생략 시 stdout)
        #[arg(long)]
        output: Option<PathBuf>,
        /// export 포맷 (현재 claude 만 지원)
        #[arg(long, value_enum, default_value_t = MemoryExportFormat::Claude)]
        format: MemoryExportFormat,
    },
    /// Claude 호환 markdown 을 import (memory/trait 생성)
    Import {
        /// 입력 파일 경로
        #[arg(long)]
        input: PathBuf,
        /// import 포맷 (현재 claude 만 지원)
        #[arg(long, value_enum, default_value_t = MemoryExportFormat::Claude)]
        format: MemoryExportFormat,
    },
    /// Claude 공식 권장 export prompt 출력 (LLM 에 그대로 붙여넣기)
    ExportPrompt,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum MemoryExportFormat {
    /// Claude 공식 호환 markdown (카테고리별 entry)
    Claude,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum MemoryKindArg {
    Fact,
    Decision,
    Reference,
    Rule,
}

impl From<MemoryKindArg> for openxgram_memory::MemoryKind {
    fn from(k: MemoryKindArg) -> Self {
        match k {
            MemoryKindArg::Fact => Self::Fact,
            MemoryKindArg::Decision => Self::Decision,
            MemoryKindArg::Reference => Self::Reference,
            MemoryKindArg::Rule => Self::Rule,
        }
    }
}

impl From<MemoryCli> for MemoryAction {
    fn from(c: MemoryCli) -> Self {
        match c {
            MemoryCli::Add {
                kind,
                content,
                session_id,
            } => MemoryAction::Add {
                kind: kind.into(),
                content,
                session_id,
            },
            MemoryCli::List { kind } => MemoryAction::List {
                kind: kind.map(Into::into),
            },
            MemoryCli::Pin { id } => MemoryAction::Pin { id },
            MemoryCli::Unpin { id } => MemoryAction::Unpin { id },
            MemoryCli::Show { conversation } => MemoryAction::ShowConversation { id: conversation },
            // Export/Import/ExportPrompt 는 main dispatch 에서 직접 처리 — 이 변환에 도달 불가.
            MemoryCli::Export { .. } | MemoryCli::Import { .. } | MemoryCli::ExportPrompt => {
                unreachable!("export/import/export-prompt 는 dispatch 에서 처리되어야 함")
            }
        }
    }
}

impl From<MemoryExportFormat> for memory::MemoryExportFmt {
    fn from(f: MemoryExportFormat) -> Self {
        match f {
            MemoryExportFormat::Claude => Self::Claude,
        }
    }
}

#[derive(Subcommand, Debug)]
enum McpTokenCli {
    /// 새 Bearer 토큰 발급 (64자 hex). 평문은 발급 직후 1회만 표시.
    Create {
        #[arg(long)]
        agent: String,
        /// 마스터 메모용 (예: "claude-code-laptop")
        #[arg(long)]
        label: Option<String>,
    },
    /// 토큰 목록 (평문 노출 안 함)
    List,
    /// 토큰 폐기
    Revoke {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum PeerCli {
    /// Peer 등록
    Add {
        #[arg(long)]
        alias: String,
        /// secp256k1 압축 공개키 hex (66자)
        #[arg(long)]
        public_key: String,
        /// 호출 가능 주소 — http://ip:port, xmtp://address 등
        #[arg(long)]
        address: String,
        #[arg(long, value_enum, default_value_t = PeerRoleArg::Worker)]
        role: PeerRoleArg,
        /// 메모 (선택)
        #[arg(long)]
        notes: Option<String>,
    },
    /// 모든 peer list
    List,
    /// 단건 상세
    Show { alias: String },
    /// last_seen 갱신 (수동, 보통 transport 가 자동으로 호출)
    Touch { alias: String },
    /// peer 삭제
    Delete { alias: String },
    /// peer 의 /v1/message 로 envelope 전송 (master ECDSA 서명, 성공 시 last_seen 갱신)
    Send {
        #[arg(long)]
        alias: String,
        #[arg(long)]
        body: String,
        /// sender 주소 (생략 시 master 주소)
        #[arg(long)]
        sender: Option<String>,
    },
    /// 여러 peer 에 동시 전송 (concurrent, 부분 실패 격리)
    Broadcast {
        /// 콤마 구분 alias 목록 (예: --aliases mac,gcp,laptop)
        #[arg(long)]
        aliases: String,
        #[arg(long)]
        body: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PeerRoleArg {
    Primary,
    Secondary,
    Worker,
}

impl From<PeerRoleArg> for openxgram_peer::PeerRole {
    fn from(r: PeerRoleArg) -> Self {
        match r {
            PeerRoleArg::Primary => Self::Primary,
            PeerRoleArg::Secondary => Self::Secondary,
            PeerRoleArg::Worker => Self::Worker,
        }
    }
}

#[derive(Subcommand, Debug)]
enum PaymentCli {
    /// 새 payment intent draft (state=draft, 서명 전)
    New {
        /// USDC 단위 (예: 1.50, 0.001). USDC 는 6 decimals 까지.
        #[arg(long)]
        amount: String,
        #[arg(long, default_value = "base")]
        chain: String,
        /// 수취인 ETH 주소 (0x...)
        #[arg(long)]
        to: String,
        #[arg(long)]
        memo: Option<String>,
    },
    /// master ECDSA 서명 (XGRAM_KEYSTORE_PASSWORD 필요)
    Sign { id: String },
    /// 모든 intent list
    List,
    /// 단건 상세
    Show { id: String },
    /// 지원 chain 목록
    Chains,
    /// 수익 요약 (받은 금액 / 보낸 금액 / 순수익) — step 14
    Summary {
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// signed intent 를 직접 RPC 로 on-chain 제출 (USDC transfer). 성공 시 자동으로 state=submitted.
    Submit {
        /// payment intent id
        id: String,
        /// RPC URL override. 미지정 시 chain 의 default 또는 env (XGRAM_BASE_RPC_PRIMARY / XGRAM_BASE_SEPOLIA_RPC)
        #[arg(long)]
        rpc_url: Option<String>,
        /// 송금 직후 수취인 peer (alias) 에게 결제 통지 envelope 자동 발송.
        /// 수취인 daemon 이 받으면 inbox 에 구조화 영수증 (xgr-payment-receipt-v1) 으로 기록 → 양쪽 메모리 일관.
        #[arg(long)]
        notify: Option<String>,
    },
    /// 외부 도구로 제출 후 호출 — state=submitted
    MarkSubmitted {
        #[arg(long)]
        id: String,
        #[arg(long)]
        tx_hash: String,
    },
    /// block 확정 후 호출 — state=confirmed
    MarkConfirmed { id: String },
    /// 실패 시 호출 — state=failed
    MarkFailed {
        #[arg(long)]
        id: String,
        #[arg(long)]
        reason: String,
    },
}

impl From<PaymentCli> for PaymentAction {
    fn from(c: PaymentCli) -> Self {
        match c {
            PaymentCli::New {
                amount,
                chain,
                to,
                memo,
            } => PaymentAction::New {
                amount_usdc: amount,
                chain,
                to,
                memo,
            },
            PaymentCli::Sign { id } => PaymentAction::Sign { id },
            PaymentCli::List => PaymentAction::List,
            PaymentCli::Show { id } => PaymentAction::Show { id },
            PaymentCli::Chains => PaymentAction::Chains,
            // Submit 은 비동기 RPC 호출이므로 dispatcher 가 별도 경로(run_payment_submit)로 처리한다.
            // 여기로 흘러오면 dispatcher 버그.
            PaymentCli::Submit { .. } => {
                unreachable!("Submit is handled async in the dispatcher, not via PaymentAction")
            }
            PaymentCli::MarkSubmitted { id, tx_hash } => {
                PaymentAction::MarkSubmitted { id, tx_hash }
            }
            PaymentCli::MarkConfirmed { id } => PaymentAction::MarkConfirmed { id },
            PaymentCli::MarkFailed { id, reason } => PaymentAction::MarkFailed { id, reason },
            PaymentCli::Summary { .. } => {
                unreachable!("Summary 는 별도 dispatch 처리")
            }
        }
    }
}

impl From<PeerCli> for PeerAction {
    fn from(c: PeerCli) -> Self {
        match c {
            PeerCli::Add {
                alias,
                public_key,
                address,
                role,
                notes,
            } => PeerAction::Add {
                alias,
                public_key_hex: public_key,
                address,
                role: role.into(),
                notes,
            },
            PeerCli::List => PeerAction::List,
            PeerCli::Show { alias } => PeerAction::Show { alias },
            PeerCli::Touch { alias } => PeerAction::Touch { alias },
            PeerCli::Delete { alias } => PeerAction::Delete { alias },
            PeerCli::Send { .. } | PeerCli::Broadcast { .. } => {
                unreachable!("Send/Broadcast 는 main.rs 에서 별도 처리")
            }
        }
    }
}

#[derive(Subcommand, Debug)]
enum VaultCli {
    /// 자격증명 저장 (XGRAM_KEYSTORE_PASSWORD 로 암호화)
    Set {
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
        /// 콤마(,) 로 구분된 태그 목록
        #[arg(long, default_value = "")]
        tags: String,
    },
    /// 자격증명 평문 출력
    Get {
        #[arg(long)]
        key: String,
    },
    /// 메타데이터 list (값은 노출 안 함)
    List,
    /// 자격증명 제거
    Delete {
        #[arg(long)]
        key: String,
    },
    /// ACL 등록 — 비-master agent 호출 권한·일일 한도 설정
    AclSet {
        /// key 정확 일치 또는 '*' 와일드카드
        #[arg(long)]
        key_pattern: String,
        /// agent 식별자 (예: 0xAlice) 또는 '*' 모든 에이전트
        #[arg(long)]
        agent: String,
        /// 콤마 구분 (get,set,delete)
        #[arg(long, default_value = "get")]
        actions: String,
        /// 일일 한도 (0 = 무제한)
        #[arg(long, default_value_t = 0)]
        daily_limit: i64,
        /// auto / confirm / mfa (Phase 1 enforcement: auto 만)
        #[arg(long, default_value = "auto")]
        policy: String,
    },
    /// ACL list
    AclList,
    /// ACL 삭제
    AclDelete {
        #[arg(long)]
        key_pattern: String,
        #[arg(long)]
        agent: String,
    },
    /// confirm 정책으로 보류 중인 요청 list
    Pending,
    /// confirm 요청 승인 (1회 소비)
    Approve { id: String },
    /// confirm 요청 거부
    Deny { id: String },
    /// agent 별 TOTP secret 발급 (mfa 정책)
    MfaIssue {
        #[arg(long)]
        agent: String,
    },
}

#[derive(Subcommand, Debug)]
enum TraitsCli {
    /// trait set (manual). 같은 name 이 있으면 value 갱신
    Set {
        #[arg(long)]
        name: String,
        #[arg(long)]
        value: String,
        /// 도출 근거 (콤마(,) 구분 — memory id, episode id 등)
        #[arg(long, default_value = "")]
        refs: String,
    },
    /// trait 단건 조회
    Get {
        #[arg(long)]
        name: String,
    },
    /// 모든 trait list (updated_at DESC)
    List,
    /// L3 ROUTINE pattern → derived trait 도출 (수동 트리거; 야간 reflection 도 자동)
    Derive,
}

impl From<TraitsCli> for TraitsAction {
    fn from(c: TraitsCli) -> Self {
        match c {
            TraitsCli::Set { name, value, refs } => TraitsAction::Set {
                name,
                value,
                source: TraitSource::Manual,
                refs: refs
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            },
            TraitsCli::Get { name } => TraitsAction::Get { name },
            TraitsCli::List => TraitsAction::List,
            TraitsCli::Derive => TraitsAction::Derive,
        }
    }
}

#[derive(Subcommand, Debug)]
enum PatternsCli {
    /// pattern observe — 같은 text 면 frequency+1, 없으면 NEW 로 insert
    Observe {
        #[arg(long)]
        text: String,
    },
    /// classification 별 list (frequency DESC)
    List {
        #[arg(long, value_enum)]
        classification: ClassificationArg,
    },
}

#[derive(Subcommand, Debug)]
enum AuditCli {
    /// chain 무결성 + 체크포인트 서명 검증
    Verify,
    /// chain hash 가 누락된 audit row 에 backfill (master 패스워드 불필요)
    Backfill,
    /// 현재 chain 끝 seq 까지 master 서명 체크포인트 생성 (master 패스워드 필요)
    Checkpoint,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ClassificationArg {
    New,
    Recurring,
    Routine,
}

impl From<ClassificationArg> for Classification {
    fn from(c: ClassificationArg) -> Self {
        match c {
            ClassificationArg::New => Self::New,
            ClassificationArg::Recurring => Self::Recurring,
            ClassificationArg::Routine => Self::Routine,
        }
    }
}

impl From<PatternsCli> for PatternsAction {
    fn from(c: PatternsCli) -> Self {
        match c {
            PatternsCli::Observe { text } => PatternsAction::Observe { text },
            PatternsCli::List { classification } => PatternsAction::List {
                classification: classification.into(),
            },
        }
    }
}

impl TryFrom<VaultCli> for VaultAction {
    type Error = anyhow::Error;
    fn try_from(c: VaultCli) -> anyhow::Result<Self> {
        Ok(match c {
            VaultCli::Set { key, value, tags } => VaultAction::Set {
                key,
                value,
                tags: tags
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            },
            VaultCli::Get { key } => VaultAction::Get { key },
            VaultCli::List => VaultAction::List,
            VaultCli::Delete { key } => VaultAction::Delete { key },
            VaultCli::AclSet {
                key_pattern,
                agent,
                actions,
                daily_limit,
                policy,
            } => VaultAction::AclSet {
                key_pattern,
                agent,
                actions: parse_acl_actions(&actions)?,
                daily_limit,
                policy: openxgram_vault::AclPolicy::parse(&policy)
                    .map_err(|e| anyhow::anyhow!("policy 파싱 실패: {e}"))?,
            },
            VaultCli::AclList => VaultAction::AclList,
            VaultCli::AclDelete { key_pattern, agent } => {
                VaultAction::AclDelete { key_pattern, agent }
            }
            VaultCli::Pending => VaultAction::Pending,
            VaultCli::Approve { id } => VaultAction::Approve { id },
            VaultCli::Deny { id } => VaultAction::Deny { id },
            VaultCli::MfaIssue { agent } => VaultAction::MfaIssue { agent },
        })
    }
}

fn parse_acl_actions(s: &str) -> anyhow::Result<Vec<openxgram_vault::AclAction>> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| match t {
            "get" => Ok(openxgram_vault::AclAction::Get),
            "set" => Ok(openxgram_vault::AclAction::Set),
            "delete" => Ok(openxgram_vault::AclAction::Delete),
            other => Err(anyhow::anyhow!("invalid action: {other}")),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackupTargetArg {
    Discord,
    Telegram,
}

impl From<BackupTargetArg> for BackupTarget {
    fn from(t: BackupTargetArg) -> Self {
        match t {
            BackupTargetArg::Discord => BackupTarget::Discord,
            BackupTargetArg::Telegram => BackupTarget::Telegram,
        }
    }
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

    // 로그 초기화 — `XGRAM_LOG_FORMAT=json` 시 구조화 로그 (운영·SRE 친화),
    // 그 외 사람용 pretty.
    let log_level = if cli.verbose { "debug" } else { "info" };
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));
    let json_mode = std::env::var("XGRAM_LOG_FORMAT").as_deref() == Ok("json");
    if json_mode {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    match cli.command {
        Commands::Init {
            alias,
            role,
            data_dir,
            force,
            dry_run,
            import,
        } => match alias {
            // 비대화 모드 — 명시적 alias.
            Some(alias) => {
                let opts = InitOpts {
                    alias,
                    role: role.into(),
                    data_dir: resolve_data_dir(data_dir)?,
                    force,
                    dry_run,
                    import,
                };
                init::run_init(&opts)?;
            }
            // 플래그 없으면 인터랙티브 마법사로 진입 — `xgram wizard` 와 동일.
            None => {
                let outcome = wizard::run_wizard()?;
                match outcome {
                    wizard::WizardOutcome::Completed { cfg } => {
                        print!("{}", wizard::render_done(&cfg));
                    }
                    wizard::WizardOutcome::Cancelled => {
                        println!("취소됨.");
                    }
                }
            }
        },

        Commands::Status { data_dir } => {
            let opts = StatusOpts {
                data_dir: resolve_data_dir(data_dir)?,
            };
            status::run_status(&opts)?;
        }

        Commands::Doctor { data_dir, json } => {
            let opts = DoctorOpts {
                data_dir: resolve_data_dir(data_dir)?,
            };
            let report = doctor::run_doctor(&opts)?;
            if json {
                println!("{}", report.to_json()?);
            } else {
                report.print();
            }
            std::process::exit(report.exit_code());
        }

        Commands::Reset {
            data_dir,
            hard,
            confirm,
            dry_run,
        } => {
            let opts = ResetOpts {
                data_dir: resolve_data_dir(data_dir)?,
                hard,
                confirm,
                dry_run,
            };
            reset::run_reset(&opts)?;
        }

        Commands::Migrate { data_dir, target } => {
            let opts = MigrateOpts {
                data_dir: resolve_data_dir(data_dir)?,
                target,
            };
            migrate::run_migrate(&opts)?;
        }

        Commands::Uninstall {
            data_dir,
            cold_backup_to,
            no_backup,
            confirm,
            dry_run,
        } => {
            let opts = UninstallOpts {
                data_dir: resolve_data_dir(data_dir)?,
                cold_backup_to,
                no_backup,
                confirm,
                dry_run,
            };
            uninstall::run_uninstall(&opts)?;
        }

        Commands::Keypair { data_dir, action } => {
            let ks_dir = match data_dir {
                Some(d) => openxgram_core::paths::keystore_dir(&d),
                None => FsKeystore::default_path(),
            };
            let ks = FsKeystore::new(&ks_dir);
            handle_keypair(ks, action)?;
        }

        Commands::Identity { action } => {
            let ks_dir = FsKeystore::default_path();
            let ks = FsKeystore::new(&ks_dir);
            openxgram_cli::identity::run(ks, action)?;
        }

        Commands::Session { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            // ImportApp 은 별도 함수 호출
            if let SessionCli::ImportApp { file, format, title } = action {
                let fmt = openxgram_cli::import_app::ImportFormat::parse(&format)?;
                let summary = openxgram_cli::import_app::run_import_app(
                    &dir,
                    &file,
                    fmt,
                    title.as_deref(),
                )?;
                println!(
                    "✓ import 완료 — session '{}' (id={}) / 메시지 {} 개",
                    summary.title, summary.session_id, summary.messages_inserted
                );
            } else {
                session::run_session(&dir, action.into())?;
            }
        }

        Commands::Memory { data_dir, action } => match action {
            MemoryCli::ExportPrompt => {
                println!("{}", openxgram_memory::CLAUDE_EXPORT_PROMPT);
            }
            MemoryCli::Export { output, format } => {
                let dir = resolve_data_dir(data_dir)?;
                memory::run_export(&dir, output.as_deref(), format.into())?;
            }
            MemoryCli::Import { input, format } => {
                let dir = resolve_data_dir(data_dir)?;
                memory::run_import(&dir, &input, format.into())?;
            }
            other => {
                let dir = resolve_data_dir(data_dir)?;
                memory::run_memory(&dir, other.into())?;
            }
        },

        Commands::Notify { target } => match target.into() {
            NotifyDispatch::Action(mut action) => {
                // store-session 모드는 data_dir 미지정 시 기본 경로로 보강.
                if let NotifyAction::DiscordListen {
                    store_session: Some(_),
                    data_dir,
                    ..
                } = &mut action
                {
                    if data_dir.is_none() {
                        *data_dir = Some(resolve_data_dir(None)?);
                    }
                }
                notify::run_notify(action).await?;
            }
            NotifyDispatch::Setup(target, opts) => {
                notify_setup::run_setup(target, opts).await?;
            }
        },

        Commands::Setup { target } => match target {
            SetupCli::Discord { data_dir } => {
                notify_setup::run_setup(
                    SetupTarget::Discord,
                    SetupOpts {
                        data_dir,
                        detect_attempts: None,
                    },
                )
                .await?;
            }
            SetupCli::Telegram { data_dir } => {
                notify_setup::run_setup(
                    SetupTarget::Telegram,
                    SetupOpts {
                        data_dir,
                        detect_attempts: None,
                    },
                )
                .await?;
            }
        },

        Commands::BackupPush {
            data_dir,
            session_id,
            target,
        } => {
            backup_push::run_backup_push(BackupPushOpts {
                data_dir: resolve_data_dir(data_dir)?,
                session_id,
                target: target.into(),
            })
            .await?;
        }

        Commands::Agent {
            data_dir,
            discord_webhook_url,
            discord_bot_token,
            discord_channel_id,
            anthropic_api_key,
            telegram_bot_token,
            telegram_chat_id,
            poll_interval_secs,
        } => {
            let dir = resolve_data_dir(data_dir)?;
            let discord =
                discord_webhook_url.or_else(|| std::env::var("XGRAM_DISCORD_WEBHOOK_URL").ok());
            let bot_token =
                discord_bot_token.or_else(|| std::env::var("XGRAM_DISCORD_BOT_TOKEN").ok());
            let channel_id =
                discord_channel_id.or_else(|| std::env::var("XGRAM_DISCORD_CHANNEL_ID").ok());
            let api_key =
                anthropic_api_key.or_else(|| std::env::var("XGRAM_ANTHROPIC_API_KEY").ok());
            let tg_token =
                telegram_bot_token.or_else(|| std::env::var("XGRAM_TELEGRAM_BOT_TOKEN").ok());
            let tg_chat = telegram_chat_id.or_else(|| std::env::var("XGRAM_TELEGRAM_CHAT_ID").ok());
            // alias 는 manifest 에서 — 실패 시 None.
            let agent_alias = openxgram_manifest::InstallManifest::read(
                &openxgram_core::paths::manifest_path(&dir),
            )
            .ok()
            .map(|m| m.machine.alias);
            openxgram_cli::agent::run_agent(openxgram_cli::agent::AgentOpts {
                data_dir: dir,
                poll_interval_secs,
                discord_webhook_url: discord,
                discord_bot_token: bot_token,
                discord_channel_id: channel_id,
                anthropic_api_key: api_key,
                agent_alias,
                telegram_bot_token: tg_token,
                telegram_chat_id: tg_chat,
            })
            .await?;
        }

        Commands::Daemon {
            data_dir,
            bind,
            gui_bind,
            reflection_cron,
            tailscale,
        } => {
            let dir = resolve_data_dir(data_dir)?;
            daemon::run_daemon(DaemonOpts {
                data_dir: dir,
                bind_addr: bind,
                gui_bind,
                reflection_cron,
                tailscale,
            })
            .await?;
        }

        Commands::DaemonInstall {
            binary,
            data_dir,
            bind,
            output,
        } => {
            let bin = match binary {
                Some(p) => p,
                None => std::env::current_exe()?.canonicalize()?,
            };
            let dir = resolve_data_dir(data_dir)?;
            let target = match output {
                Some(p) => p,
                None => systemd::default_user_unit_path()?,
            };
            let opts = UnitOpts {
                binary: bin,
                data_dir: dir,
                bind,
            };
            systemd::install_user_unit(&target, &opts)?;
            println!("✓ systemd user unit 생성: {}", target.display());
            println!();
            println!("활성화:");
            println!("  systemctl --user daemon-reload");
            println!("  systemctl --user enable --now openxgram-sidecar.service");
            println!();
            println!(
                "주의: XGRAM_KEYSTORE_PASSWORD 는 systemd-creds 또는 EnvironmentFile 로 별도 주입."
            );
        }

        Commands::BackupInstall {
            binary,
            data_dir,
            backup_dir,
            on_calendar,
        } => {
            let bin = match binary {
                Some(p) => p,
                None => std::env::current_exe()?.canonicalize()?,
            };
            let dir = resolve_data_dir(data_dir)?;
            let opts = systemd::BackupUnitOpts {
                binary: bin,
                data_dir: dir,
                backup_dir,
                on_calendar: on_calendar
                    .unwrap_or_else(|| systemd::DEFAULT_BACKUP_ON_CALENDAR.to_string()),
            };
            let svc = systemd::default_backup_service_path()?;
            let tim = systemd::default_backup_timer_path()?;
            systemd::install_backup_units(&svc, &tim, &opts)?;
            println!("✓ systemd backup units 생성");
            println!("  service: {}", svc.display());
            println!("  timer  : {}", tim.display());
            println!();
            println!("활성화:");
            println!("  systemctl --user daemon-reload");
            println!("  systemctl --user enable --now openxgram-backup.timer");
            println!();
            println!(
                "주의: XGRAM_KEYSTORE_PASSWORD 는 systemd-creds 또는 EnvironmentFile 로 별도 주입."
            );
        }

        Commands::BackupUninstall => {
            let svc = systemd::default_backup_service_path()?;
            let tim = systemd::default_backup_timer_path()?;
            systemd::uninstall_backup_units(&svc, &tim)?;
            println!("✓ systemd backup units 제거");
            println!("  service: {}", svc.display());
            println!("  timer  : {}", tim.display());
            println!();
            println!("정리:");
            println!("  systemctl --user disable --now openxgram-backup.timer");
            println!("  systemctl --user daemon-reload");
        }

        Commands::DaemonUninstall { output } => {
            let target = match output {
                Some(p) => p,
                None => systemd::default_user_unit_path()?,
            };
            systemd::uninstall_user_unit(&target)?;
            println!("✓ systemd user unit 제거: {}", target.display());
            println!();
            println!("정리:");
            println!("  systemctl --user disable --now openxgram-sidecar.service");
            println!("  systemctl --user daemon-reload");
        }

        Commands::McpServe { data_dir, bind } => {
            let dir = resolve_data_dir(data_dir)?;
            match bind {
                Some(addr) => mcp_serve::run_http_serve(&dir, addr).await?,
                None => mcp_serve::run_serve(&dir)?,
            }
        }

        Commands::McpToken { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            let mut db = openxgram_cli::mcp_tokens::open_db(&dir)?;
            match action {
                McpTokenCli::Create { agent, label } => {
                    let (id, plain) =
                        openxgram_cli::mcp_tokens::create_token(&mut db, &agent, label.as_deref())?;
                    println!("✓ MCP 토큰 발급 (이 값은 다시 표시되지 않습니다)");
                    println!("  id    : {id}");
                    println!("  agent : {agent}");
                    println!("  token : {plain}");
                    println!();
                    println!("클라이언트 헤더 사용 예: `Authorization: Bearer {plain}`");
                }
                McpTokenCli::List => {
                    let entries = openxgram_cli::mcp_tokens::list_tokens(&mut db)?;
                    if entries.is_empty() {
                        println!("MCP 토큰 없음.");
                    } else {
                        println!("MCP 토큰 ({})", entries.len());
                        for e in &entries {
                            let last = e.last_used.as_deref().unwrap_or("(미사용)");
                            let label = e.label.as_deref().unwrap_or("");
                            println!(
                                "  {} — agent={} label={:?} created={} last_used={}",
                                e.id, e.agent, label, e.created_at, last
                            );
                        }
                    }
                }
                McpTokenCli::Revoke { id } => {
                    openxgram_cli::mcp_tokens::revoke_token(&mut db, &id)?;
                    println!("✓ MCP 토큰 폐기: {id}");
                }
            }
        }

        Commands::Vault { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            vault::run_vault(&dir, action.try_into()?)?;
        }

        Commands::Traits { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            traits::run_traits(&dir, action.into())?;
        }

        Commands::Patterns { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            patterns::run_patterns(&dir, action.into())?;
        }

        Commands::Audit { data_dir, cmd } => {
            let dir = resolve_data_dir(data_dir)?;
            let report = match cmd {
                AuditCli::Verify => audit::run_audit(&dir, AuditAction::Verify)?,
                AuditCli::Backfill => audit::run_audit(&dir, AuditAction::Backfill)?,
                AuditCli::Checkpoint => {
                    let pw = openxgram_core::env::require_password()?;
                    audit::run_audit_checkpoint(&dir, &pw)?
                }
            };
            println!("{report}");
        }

        Commands::Wizard => {
            let outcome = wizard::run_wizard()?;
            match outcome {
                wizard::WizardOutcome::Completed { cfg } => {
                    print!("{}", wizard::render_done(&cfg));
                }
                wizard::WizardOutcome::Cancelled => {
                    println!("취소됨.");
                }
            }
        }

        Commands::Restore {
            input,
            target_dir,
            merge,
        } => {
            let dir = resolve_data_dir(target_dir)?;
            let pw = openxgram_core::env::require_password()?;
            let info = if merge {
                restore_cold_backup_merge(&input, &dir, &pw)?
            } else {
                restore_cold_backup(&input, &dir, &pw)?
            };
            println!(
                "✓ cold backup 복원 완료{}",
                if merge { " (merge)" } else { "" }
            );
            println!("  source       : {}", input.display());
            println!("  target_dir   : {}", info.target_dir.display());
            println!("  bytes_restored: {}", info.bytes_restored);
        }

        Commands::Backup { data_dir, to } => {
            let dir = resolve_data_dir(data_dir)?;
            let pw = openxgram_core::env::require_password()?;
            let target = resolve_backup_target(&to)?;
            let info = create_cold_backup(&dir, &target, &pw)?;
            println!("✓ cold backup 생성");
            println!("  source     : {}", dir.display());
            println!("  path       : {}", info.path.display());
            println!("  size_bytes : {}", info.size_bytes);
            println!("  sha256     : {}", info.sha256);
        }

        Commands::Tui { data_dir } => {
            let opts = TuiOpts {
                data_dir: resolve_data_dir(data_dir)?,
            };
            tui::run_tui(&opts)?;
        }

        Commands::Gui { args } => {
            openxgram_cli::gui::run_gui(&args)?;
        }

        Commands::Link { url, data_dir } => {
            let dir = resolve_data_dir(data_dir)?;
            openxgram_cli::link::run_link(&dir, &url).await?;
        }

        Commands::PairDesktop { data_dir } => {
            let dir = resolve_data_dir(data_dir)?;
            openxgram_cli::pair_desktop::run_pair_desktop(&dir)?;
        }

        Commands::Peer { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            // Send/Broadcast 는 async/transport 필요 — 다른 액션과 분리 처리
            match action {
                PeerCli::Send {
                    alias,
                    body,
                    sender,
                } => {
                    let pw = openxgram_core::env::require_password()?;
                    peer_send::run_peer_send(&dir, &alias, sender.as_deref(), &body, &pw).await?;
                }
                PeerCli::Broadcast { aliases, body } => {
                    let pw = openxgram_core::env::require_password()?;
                    let alias_list: Vec<String> = aliases
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                    let results =
                        peer_send::run_peer_broadcast(&dir, &alias_list, &body, &pw).await?;
                    let total = results.len();
                    let succ = results.iter().filter(|(_, r)| r.is_ok()).count();
                    println!("✓ broadcast 완료 — {succ}/{total} 성공");
                    for (alias, res) in &results {
                        match res {
                            Ok(()) => println!("  ✓ {alias}"),
                            Err(e) => println!("  ✗ {alias}: {e}"),
                        }
                    }
                }
                other => {
                    peer::run_peer(&dir, other.into())?;
                }
            }
        }

        Commands::Payment { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            match action {
                PaymentCli::Submit {
                    id,
                    rpc_url,
                    notify,
                } => {
                    payment::run_payment_submit(&dir, &id, rpc_url.as_deref(), notify.as_deref())
                        .await?;
                }
                PaymentCli::Summary { data_dir: dd } => {
                    let summary_dir = match dd {
                        Some(p) => p,
                        None => dir.clone(),
                    };
                    openxgram_cli::payment_summary::run_summary(&summary_dir)?;
                }
                other => payment::run_payment(&dir, other.into())?,
            }
        }

        Commands::Relay {
            bind,
            port,
            min_pow,
            max_connections,
        } => {
            let addr: std::net::IpAddr = bind
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid bind addr {bind}: {e}"))?;
            let cfg = openxgram_nostr::RelayConfig {
                addr,
                port,
                min_pow,
                max_connections,
            };
            let relay = openxgram_nostr::NostrRelay::serve(cfg).await?;
            println!("openxgram relay listening at {}", relay.url());
            // SIGINT 까지 블록
            tokio::signal::ctrl_c()
                .await
                .map_err(|e| anyhow::anyhow!("ctrl_c handler: {e}"))?;
            println!("shutting down relay");
            relay.shutdown();
        }

        Commands::Schedule { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            orchestration::run_schedule(&dir, action)?;
        }

        Commands::Chain { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            orchestration::run_chain(&dir, action)?;
        }

        Commands::Completions { shell } => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
        }

        Commands::Dump { data_dir, kind } => {
            let dir = resolve_data_dir(data_dir)?;
            dump::run_dump(&dir, &kind)?;
        }

        Commands::Version { json } => {
            let info = build_info();
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("xgram {} ({})", info.version, info.target);
                println!();
                println!("features:");
                for f in &info.features {
                    println!("  - {f}");
                }
                println!();
                println!("주요 의존 crate:");
                for (name, ver) in &info.deps {
                    println!("  {name} {ver}");
                }
            }
        }

        Commands::Channel { cmd } => {
            let action = match cmd {
                ChannelCli::Serve { bind, auth_token } => ChannelAction::Serve { bind, auth_token },
                ChannelCli::Send {
                    server,
                    auth_token,
                    to_role,
                    platform,
                    channel_id,
                    text,
                    reply_to,
                    msg_type,
                } => ChannelAction::Send {
                    server,
                    auth_token,
                    to_role,
                    platform,
                    channel_id,
                    text,
                    reply_to,
                    msg_type,
                },
                ChannelCli::ListAdapters { server, auth_token } => {
                    ChannelAction::ListAdapters { server, auth_token }
                }
                ChannelCli::ListPeers { server, auth_token } => {
                    ChannelAction::ListPeers { server, auth_token }
                }
            };
            channel::run(action).await?;
        }

        Commands::Onboard { cmd } => match cmd {
            OnboardCli::Prompt { lang, copy } => {
                openxgram_cli::onboard::run_onboard_prompt(lang, copy)?;
            }
        },

        Commands::Bot { cmd } => match cmd {
            BotCli::Add { name, alias } => {
                openxgram_cli::bot::bot_add(&name, alias.as_deref())?;
            }
            BotCli::List => {
                openxgram_cli::bot::bot_list()?;
            }
            BotCli::Remove { name, force } => {
                openxgram_cli::bot::bot_remove(&name, force)?;
            }
            BotCli::Start { name } => {
                openxgram_cli::bot::bot_start(&name)?;
            }
            BotCli::Stop { name } => {
                openxgram_cli::bot::bot_stop(&name)?;
            }
            BotCli::Link { a, b } => {
                openxgram_cli::bot::bot_link(&a, &b)?;
            }
            BotCli::Register { name, alias } => {
                openxgram_cli::bot::bot_register(&name, alias.as_deref())?;
            }
        },

        Commands::McpInstall {
            scope,
            config,
            data_dir,
            with_password,
        } => {
            let resolved_scope = match scope {
                McpInstallScope::User => openxgram_cli::mcp_install::McpScope::User,
                McpInstallScope::Project => openxgram_cli::mcp_install::McpScope::Project,
                McpInstallScope::Custom => openxgram_cli::mcp_install::McpScope::Custom(
                    config.ok_or_else(|| anyhow::anyhow!("scope=custom 일 때 --config 필수"))?,
                ),
            };
            let dir = resolve_data_dir(data_dir)?;
            openxgram_cli::mcp_install::run_install(resolved_scope, &dir, with_password)?;
        }

        Commands::IdentityInject { target, data_dir } => {
            let dir = resolve_data_dir(data_dir)?;
            openxgram_cli::mcp_install::run_inject(&target, &dir)?;
        }

        Commands::IdentityUninject { target } => {
            openxgram_cli::mcp_install::run_uninject(&target)?;
        }

        Commands::Invite { data_dir, alias, address } => {
            let dir = resolve_data_dir(data_dir)?;
            let alias = alias.unwrap_or_else(|| "me".into());
            openxgram_cli::invite::run_invite(&dir, &alias, &address)?;
        }

        Commands::Friend { cmd } => match cmd {
            FriendCli::Accept { url, data_dir } => {
                let dir = resolve_data_dir(data_dir)?;
                openxgram_cli::invite::run_friend_accept(&dir, &url)?;
            }
        },

        Commands::Channels { cmd } => match cmd {
            ChannelsCli::Add {
                kind,
                address,
                visibility,
                data_dir,
            } => {
                let dir = resolve_data_dir(data_dir)?;
                openxgram_cli::channels::channel_add(&dir, &kind, &address, &visibility)?;
            }
            ChannelsCli::List { data_dir } => {
                let dir = resolve_data_dir(data_dir)?;
                let list = openxgram_cli::channels::channel_list(&dir)?;
                if list.is_empty() {
                    println!("(등록된 채널 없음)");
                } else {
                    for c in list {
                        println!("{:<12} {:<40} {}", c.kind, c.address, c.visibility);
                    }
                }
            }
            ChannelsCli::Remove {
                kind,
                address,
                data_dir,
            } => {
                let dir = resolve_data_dir(data_dir)?;
                openxgram_cli::channels::channel_remove(&dir, &kind, &address)?;
            }
        },

        Commands::Directory { cmd } => match cmd {
            DirectoryCli::Lookup { handle } => {
                let chans = openxgram_cli::channels::directory_lookup(&handle)?;
                if chans.is_empty() {
                    println!("(채널 없음 — directory cache miss; xgram identity publish 시 등록 권장)");
                } else {
                    for c in chans {
                        println!("{:<12} {:<40} {}", c.kind, c.address, c.visibility);
                    }
                }
            }
            DirectoryCli::Set {
                handle,
                channels_json,
            } => {
                let chans: Vec<openxgram_cli::channels::Channel> =
                    serde_json::from_str(&channels_json)
                        .map_err(|e| anyhow::anyhow!("channels_json 파싱 (JSON): {e}"))?;
                openxgram_cli::channels::directory_set(&handle, chans)?;
                println!("✓ {handle} 디렉터리 cache 갱신");
            }
            DirectoryCli::Register {
                to,
                with_counts,
                data_dir,
            } => {
                let dir = resolve_data_dir(data_dir)?;
                openxgram_cli::identity_handle::register_to_directory(&dir, &to, with_counts)
                    .await?;
            }
        },

        Commands::Find { query, indexer } => {
            openxgram_cli::find::run_find(openxgram_cli::find::FindOpts { query, indexer })
                .await?;
        }

        Commands::Eas { cmd } => match cmd {
            EasCli::List { limit, data_dir } => {
                let dir = resolve_data_dir(data_dir)?;
                openxgram_cli::eas::run_list(&dir, limit)?;
            }
            EasCli::Count { data_dir } => {
                let dir = resolve_data_dir(data_dir)?;
                openxgram_cli::eas::run_count(&dir)?;
            }
            EasCli::Attest { kind, fields, data_dir } => {
                let dir = resolve_data_dir(data_dir)?;
                let k = match kind.as_str() {
                    "message" => openxgram_eas::AttestationKind::Message,
                    "payment" => openxgram_eas::AttestationKind::Payment,
                    "endorsement" => openxgram_eas::AttestationKind::Endorsement,
                    other => anyhow::bail!("kind = message | payment | endorsement (got: {other})"),
                };
                openxgram_cli::eas::run_attest(&dir, k, &fields)?;
            }
        },

        Commands::Openagentx { cmd } => match cmd {
            OpenagentxCli::Call {
                agent,
                prompt,
                pay,
                memo,
                data_dir,
            } => {
                let dir = resolve_data_dir(data_dir)?;
                let answer = openxgram_cli::openagentx::run_call(
                    &dir,
                    openxgram_cli::openagentx::CallOpts {
                        agent,
                        prompt,
                        pay_micros: pay,
                        memo,
                    },
                )
                .await?;
                println!("{answer}");
            }
        },

        Commands::Send {
            handle,
            body,
            kind,
            conversation_id,
            data_dir,
        } => {
            let dir = resolve_data_dir(data_dir)?;
            openxgram_cli::send::run_send(
                &dir,
                openxgram_cli::send::SendOpts {
                    handle,
                    body,
                    prefer_kind: kind,
                    conversation_id,
                },
            )
            .await?;
        }

        Commands::Human { cmd } => match cmd {
            HumanCli::Pending { data_dir } => {
                let dir = resolve_data_dir(data_dir)?;
                let pending = openxgram_cli::hitl::list_pending_requests(&dir)?;
                if pending.is_empty() {
                    println!("(미응답 요청 없음)");
                } else {
                    println!("미응답 HITL 요청 {} 건", pending.len());
                    for r in pending {
                        println!("  [{}] {}", r.id, r.question);
                        for o in r.options {
                            println!("    - {o}");
                        }
                    }
                }
            }
            HumanCli::Respond {
                request_id,
                answer,
                data_dir,
            } => {
                let dir = resolve_data_dir(data_dir)?;
                openxgram_cli::hitl::respond_human(&dir, &request_id, &answer)?;
            }
        },
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct BuildInfo {
    version: &'static str,
    target: String,
    features: Vec<&'static str>,
    deps: Vec<(&'static str, &'static str)>,
}

fn build_info() -> BuildInfo {
    let mut features: Vec<&'static str> = vec!["base"];
    if cfg!(feature = "fastembed") {
        features.push("fastembed");
    }
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        features,
        deps: vec![
            ("axum", "0.8"),
            ("reqwest", "0.13"),
            ("rusqlite", "0.39"),
            ("tokio", "1"),
            ("clap", "4"),
            ("ratatui", "0.29"),
            ("k256", "0.13"),
            ("totp-rs", "5"),
        ],
    }
}

fn resolve_data_dir(arg: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match arg {
        Some(p) => Ok(p),
        None => Ok(openxgram_core::paths::default_data_dir()?),
    }
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
