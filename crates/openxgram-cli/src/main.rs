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
use openxgram_cli::notify::{self, NotifyAction};
use openxgram_cli::notify_setup::{self, SetupOpts, SetupTarget};
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
        #[command(subcommand)]
        action: KeypairAction,
    },

    /// W3C DID + 한국 OpenDID + OmniOne Open DID 호환 신원 (did/did-document/issue-vc/verify-vc)
    Identity {
        #[command(subcommand)]
        action: openxgram_cli::identity::IdentityCli,
    },

    /// 내장 Channel MCP 서버 (다중 에이전트 채팅 라우팅 허브) — Starian channel-mcp 호환
    Channel {
        #[command(subcommand)]
        cmd: ChannelCli,
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

    /// session 통계 백업을 Discord/Telegram 으로 push
    BackupPush {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        session_id: String,
        #[arg(long, value_enum)]
        target: BackupTargetArg,
    },

    /// 사이드카 데몬 — scheduler + transport server foreground 실행
    Daemon {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// transport bind 주소 (기본 127.0.0.1:7300, --tailscale 우선)
        #[arg(long)]
        bind: Option<std::net::SocketAddr>,
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
        #[arg(long, default_value = "127.0.0.1:7300")]
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
        /// HTTP transport bind 주소 (예: 127.0.0.1:7301). 생략 시 stdio.
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
    /// 기동 중인 서버에 메시지 전송 (role 또는 platform 직접)
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
    /// 등록된 피어 목록
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
        }
    }
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
    /// kind 별 list (pinned 우선)
    List {
        #[arg(long, value_enum)]
        kind: MemoryKindArg,
    },
    /// memory pin
    Pin { id: String },
    /// memory unpin
    Unpin { id: String },
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
            MemoryCli::List { kind } => MemoryAction::List { kind: kind.into() },
            MemoryCli::Pin { id } => MemoryAction::Pin { id },
            MemoryCli::Unpin { id } => MemoryAction::Unpin { id },
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
            PaymentCli::MarkSubmitted { id, tx_hash } => {
                PaymentAction::MarkSubmitted { id, tx_hash }
            }
            PaymentCli::MarkConfirmed { id } => PaymentAction::MarkConfirmed { id },
            PaymentCli::MarkFailed { id, reason } => PaymentAction::MarkFailed { id, reason },
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
        } => {
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

        Commands::Keypair { action } => {
            let ks_dir = FsKeystore::default_path();
            let ks = FsKeystore::new(&ks_dir);
            handle_keypair(ks, action)?;
        }

        Commands::Identity { action } => {
            let ks_dir = FsKeystore::default_path();
            let ks = FsKeystore::new(&ks_dir);
            openxgram_cli::identity::run(ks, action)?;
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

        Commands::Session { data_dir, action } => {
            let dir = resolve_data_dir(data_dir)?;
            session::run_session(&dir, action.into())?;
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

        Commands::Daemon {
            data_dir,
            bind,
            reflection_cron,
            tailscale,
        } => {
            let dir = resolve_data_dir(data_dir)?;
            daemon::run_daemon(DaemonOpts {
                data_dir: dir,
                bind_addr: bind,
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
            payment::run_payment(&dir, action.into())?;
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
