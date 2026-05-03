//! xgram wizard — 9단계 init 인터랙티브 마법사.
//!
//! 흐름 점검 (마스터 강조 영역):
//!   - 모든 단계는 Esc/B 로 이전 단계 복귀, Q 로 전체 취소.
//!   - 텍스트 입력 단계는 Enter 로 확정 (alias 비어있으면 차단).
//!   - 토글 단계는 Y/N · 1/2/3 · D/T 등 직관적 키.
//!   - 마지막 Confirm 에서 Enter → Done. Done 은 비대화 명령 시퀀스 출력.
//!
//! 9 단계:
//!   1. Welcome
//!   2. Alias       (텍스트)
//!   3. Role        (1=primary, 2=secondary, 3=worker)
//!   4. DataDir     (텍스트, 비어있으면 기본 ~/.openxgram)
//!   5. SeedMode    (N=new, I=import via XGRAM_SEED)
//!   6. Adapter     (D 토글 discord, T 토글 telegram)
//!   7. Bind        (텍스트, 기본 127.0.0.1:7300)
//!   8. DaemonInstall (Y/N)
//!   9. BackupInstall (Y/N)
//!
//! Confirm → Done

use std::io;

use anyhow::Result;
use ratatui::{
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Primary,
    Secondary,
    Worker,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Worker => "worker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedMode {
    New,
    Import,
}

impl SeedMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Import => "import (XGRAM_SEED)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WizardConfig {
    pub alias: String,
    pub role: Role,
    /// 빈 문자열 = 기본 (~/.openxgram)
    pub data_dir_override: String,
    pub seed_mode: SeedMode,
    pub adapter_discord: bool,
    pub adapter_telegram: bool,
    pub bind: String,
    pub install_daemon: bool,
    pub install_backup_timer: bool,
}

impl Default for WizardConfig {
    fn default() -> Self {
        Self {
            alias: String::new(),
            role: Role::Primary,
            data_dir_override: String::new(),
            seed_mode: SeedMode::New,
            adapter_discord: false,
            adapter_telegram: false,
            bind: "127.0.0.1:7300".into(),
            install_daemon: false,
            install_backup_timer: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardState {
    Welcome,
    Alias { cfg: WizardConfig },
    RoleStep { cfg: WizardConfig },
    DataDir { cfg: WizardConfig },
    SeedMode { cfg: WizardConfig },
    Adapter { cfg: WizardConfig },
    Bind { cfg: WizardConfig },
    Daemon { cfg: WizardConfig },
    Backup { cfg: WizardConfig },
    Confirm { cfg: WizardConfig },
    Done { cfg: WizardConfig },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardOutcome {
    Completed { cfg: WizardConfig },
    Cancelled,
}

impl WizardState {
    pub fn initial() -> Self {
        Self::Welcome
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Cancelled)
    }

    pub fn outcome(&self) -> Option<WizardOutcome> {
        match self {
            Self::Done { cfg } => Some(WizardOutcome::Completed { cfg: cfg.clone() }),
            Self::Cancelled => Some(WizardOutcome::Cancelled),
            _ => None,
        }
    }

    pub fn handle(self, key: KeyCode) -> Self {
        match (self, key) {
            (s @ (Self::Done { .. } | Self::Cancelled), _) => s,

            // Welcome
            (Self::Welcome, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q')) => {
                Self::Cancelled
            }
            (Self::Welcome, KeyCode::Enter) => Self::Alias {
                cfg: WizardConfig::default(),
            },
            (s @ Self::Welcome, _) => s,

            // 2. Alias (text)
            (Self::Alias { mut cfg }, KeyCode::Char(c)) if !c.is_control() => {
                cfg.alias.push(c);
                Self::Alias { cfg }
            }
            (Self::Alias { mut cfg }, KeyCode::Backspace) => {
                cfg.alias.pop();
                Self::Alias { cfg }
            }
            (Self::Alias { cfg }, KeyCode::Enter) if !cfg.alias.is_empty() => {
                Self::RoleStep { cfg }
            }
            (Self::Alias { cfg: _ }, KeyCode::Esc) => Self::Welcome,
            (s @ Self::Alias { .. }, _) => s,

            // 3. Role (1/2/3)
            (Self::RoleStep { mut cfg }, KeyCode::Char('1')) => {
                cfg.role = Role::Primary;
                Self::RoleStep { cfg }
            }
            (Self::RoleStep { mut cfg }, KeyCode::Char('2')) => {
                cfg.role = Role::Secondary;
                Self::RoleStep { cfg }
            }
            (Self::RoleStep { mut cfg }, KeyCode::Char('3')) => {
                cfg.role = Role::Worker;
                Self::RoleStep { cfg }
            }
            (Self::RoleStep { cfg }, KeyCode::Enter) => Self::DataDir { cfg },
            (Self::RoleStep { cfg }, KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('B')) => {
                Self::Alias { cfg }
            }
            (s @ Self::RoleStep { .. }, _) => s,

            // 4. DataDir (text, blank = default)
            (Self::DataDir { mut cfg }, KeyCode::Char(c)) if !c.is_control() => {
                cfg.data_dir_override.push(c);
                Self::DataDir { cfg }
            }
            (Self::DataDir { mut cfg }, KeyCode::Backspace) => {
                cfg.data_dir_override.pop();
                Self::DataDir { cfg }
            }
            (Self::DataDir { cfg }, KeyCode::Enter) => Self::SeedMode { cfg },
            (Self::DataDir { cfg }, KeyCode::Esc) => Self::RoleStep { cfg },
            (s @ Self::DataDir { .. }, _) => s,

            // 5. SeedMode (N/I)
            (Self::SeedMode { mut cfg }, KeyCode::Char('n') | KeyCode::Char('N')) => {
                cfg.seed_mode = SeedMode::New;
                Self::SeedMode { cfg }
            }
            (Self::SeedMode { mut cfg }, KeyCode::Char('i') | KeyCode::Char('I')) => {
                cfg.seed_mode = SeedMode::Import;
                Self::SeedMode { cfg }
            }
            (Self::SeedMode { cfg }, KeyCode::Enter) => Self::Adapter { cfg },
            (Self::SeedMode { cfg }, KeyCode::Esc) => Self::DataDir { cfg },
            (s @ Self::SeedMode { .. }, _) => s,

            // 6. Adapter (D/T toggle)
            (Self::Adapter { mut cfg }, KeyCode::Char('d') | KeyCode::Char('D')) => {
                cfg.adapter_discord = !cfg.adapter_discord;
                Self::Adapter { cfg }
            }
            (Self::Adapter { mut cfg }, KeyCode::Char('t') | KeyCode::Char('T')) => {
                cfg.adapter_telegram = !cfg.adapter_telegram;
                Self::Adapter { cfg }
            }
            (Self::Adapter { cfg }, KeyCode::Enter) => Self::Bind { cfg },
            (Self::Adapter { cfg }, KeyCode::Esc) => Self::SeedMode { cfg },
            (s @ Self::Adapter { .. }, _) => s,

            // 7. Bind (text)
            (Self::Bind { mut cfg }, KeyCode::Char(c)) if !c.is_control() => {
                cfg.bind.push(c);
                Self::Bind { cfg }
            }
            (Self::Bind { mut cfg }, KeyCode::Backspace) => {
                cfg.bind.pop();
                Self::Bind { cfg }
            }
            (Self::Bind { cfg }, KeyCode::Enter) if !cfg.bind.is_empty() => Self::Daemon { cfg },
            (Self::Bind { cfg }, KeyCode::Esc) => Self::Adapter { cfg },
            (s @ Self::Bind { .. }, _) => s,

            // 8. Daemon (Y/N)
            (Self::Daemon { mut cfg }, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                cfg.install_daemon = true;
                Self::Daemon { cfg }
            }
            (Self::Daemon { mut cfg }, KeyCode::Char('n') | KeyCode::Char('N')) => {
                cfg.install_daemon = false;
                Self::Daemon { cfg }
            }
            (Self::Daemon { cfg }, KeyCode::Enter) => Self::Backup { cfg },
            (Self::Daemon { cfg }, KeyCode::Esc) => Self::Bind { cfg },
            (s @ Self::Daemon { .. }, _) => s,

            // 9. Backup (Y/N)
            (Self::Backup { mut cfg }, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                cfg.install_backup_timer = true;
                Self::Backup { cfg }
            }
            (Self::Backup { mut cfg }, KeyCode::Char('n') | KeyCode::Char('N')) => {
                cfg.install_backup_timer = false;
                Self::Backup { cfg }
            }
            (Self::Backup { cfg }, KeyCode::Enter) => Self::Confirm { cfg },
            (Self::Backup { cfg }, KeyCode::Esc) => Self::Daemon { cfg },
            (s @ Self::Backup { .. }, _) => s,

            // Confirm
            (Self::Confirm { cfg }, KeyCode::Enter) => Self::Done { cfg },
            (Self::Confirm { cfg }, KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('B')) => {
                Self::Backup { cfg }
            }
            (s @ Self::Confirm { .. }, _) => s,
        }
    }
}

pub fn draw(frame: &mut Frame, state: &WizardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "xgram wizard",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" — OpenXgram init"),
    ]))
    .block(Block::default().borders(Borders::ALL).title("OpenXgram"));
    frame.render_widget(title, chunks[0]);

    let (body, footer_text) = body_for(state);

    let body_widget = Paragraph::new(body)
        .block(Block::default().borders(Borders::ALL).title("Step"))
        .wrap(Wrap { trim: false });
    frame.render_widget(body_widget, chunks[1]);

    let footer =
        Paragraph::new(Line::from(footer_text)).block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

fn body_for(state: &WizardState) -> (String, &'static str) {
    match state {
        WizardState::Welcome => (
            "환영합니다.\n\n9단계 마법사로 OpenXgram 을 설치합니다.\n\n계속하려면 Enter, 종료는 Esc/Q.".to_string(),
            "Enter: 다음  /  Esc · Q: 취소",
        ),
        WizardState::Alias { cfg } => (
            format!(
                "[1/9] 머신 별칭\n\n예: gcp-main, mac-mini\n\n현재 입력: {}",
                if cfg.alias.is_empty() { "(empty)" } else { cfg.alias.as_str() }
            ),
            "Enter: 다음 (alias 비어있지 않을 때)  /  Esc: Welcome",
        ),
        WizardState::RoleStep { cfg } => (
            format!(
                "[2/9] 머신 역할\n\n  1) primary   — 마스터 머신 (현재 선택지 default)\n  2) secondary — 백업 / 로테이션\n  3) worker    — 실행 전용\n\n현재: {}",
                cfg.role.as_str()
            ),
            "1/2/3 선택  /  Enter: 다음  /  Esc: 이전",
        ),
        WizardState::DataDir { cfg } => (
            format!(
                "[3/9] 데이터 디렉토리\n\n비어두면 기본 (~/.openxgram). 다른 경로 사용 시 절대 경로 입력.\n\n현재 입력: {}",
                if cfg.data_dir_override.is_empty() { "(default ~/.openxgram)" } else { cfg.data_dir_override.as_str() }
            ),
            "Enter: 다음  /  Esc: 이전",
        ),
        WizardState::SeedMode { cfg } => (
            format!(
                "[4/9] 시드 모드\n\n  N) new    — 새 BIP39 24-word 마스터 시드 생성\n  I) import — 기존 시드 가져오기 (XGRAM_SEED 환경변수에 24단어 미리 export)\n\n현재: {}",
                cfg.seed_mode.as_str()
            ),
            "N/I 선택  /  Enter: 다음  /  Esc: 이전",
        ),
        WizardState::Adapter { cfg } => (
            format!(
                "[5/9] 외부 어댑터 (선택)\n\n  D) Discord webhook  : {}\n  T) Telegram bot     : {}\n\n둘 다 vault 에 자격증명 저장 후 사용. 지금 활성화는 후속 단계에서 안내.",
                if cfg.adapter_discord { "ON" } else { "off" },
                if cfg.adapter_telegram { "ON" } else { "off" },
            ),
            "D/T 토글  /  Enter: 다음  /  Esc: 이전",
        ),
        WizardState::Bind { cfg } => (
            format!(
                "[6/9] Transport bind\n\nxgram daemon 의 HTTP 바인딩 주소. 기본 127.0.0.1:7300 (localhost 전용).\nTailscale/외부 노출은 추후 mTLS 와 함께.\n\n현재 입력: {}",
                cfg.bind
            ),
            "Enter: 다음 (bind 비어있지 않을 때)  /  Esc: 이전",
        ),
        WizardState::Daemon { cfg } => (
            format!(
                "[7/9] systemd sidecar daemon 등록\n\nY 면 backup-install 시점에 함께 실행할 명령 안내.\n\n현재: {}",
                if cfg.install_daemon { "Y (등록)" } else { "N (수동 실행)" }
            ),
            "Y/N  /  Enter: 다음  /  Esc: 이전",
        ),
        WizardState::Backup { cfg } => (
            format!(
                "[8/9] systemd 주기 cold backup\n\nY 면 매주 일요일 03시 KST 자동 backup. unit 파일 생성 명령은 Done 화면에서.\n\n현재: {}",
                if cfg.install_backup_timer { "Y (등록)" } else { "N (수동)" }
            ),
            "Y/N  /  Enter: 다음  /  Esc: 이전",
        ),
        WizardState::Confirm { cfg } => (
            format!(
                "[9/9] 확인\n\n  alias        : {}\n  role         : {}\n  data_dir     : {}\n  seed         : {}\n  discord      : {}\n  telegram     : {}\n  bind         : {}\n  daemon       : {}\n  backup timer : {}\n\nEnter 로 완료, Esc/B 로 이전.",
                cfg.alias,
                cfg.role.as_str(),
                if cfg.data_dir_override.is_empty() { "(default ~/.openxgram)" } else { cfg.data_dir_override.as_str() },
                cfg.seed_mode.as_str(),
                if cfg.adapter_discord { "ON" } else { "off" },
                if cfg.adapter_telegram { "ON" } else { "off" },
                cfg.bind,
                if cfg.install_daemon { "Y" } else { "N" },
                if cfg.install_backup_timer { "Y" } else { "N" },
            ),
            "Enter: 완료  /  Esc/B: 이전",
        ),
        WizardState::Done { cfg } => (render_done(cfg), "아무 키: 종료"),
        WizardState::Cancelled => (
            "취소되었습니다.\n\n아무 키나 누르면 종료.".to_string(),
            "아무 키: 종료",
        ),
    }
}

/// 완료 화면 — 사용자가 그대로 복사할 수 있는 비대화 명령 시퀀스.
pub fn render_done(cfg: &WizardConfig) -> String {
    let mut out = String::from("✓ wizard 완료. 아래 명령을 순서대로 실행하세요.\n\n");

    out.push_str("1) keystore 패스워드 export (12자 이상):\n");
    out.push_str("   export XGRAM_KEYSTORE_PASSWORD='<your-secure-password>'\n");
    if matches!(cfg.seed_mode, SeedMode::Import) {
        out.push_str("   export XGRAM_SEED='word1 word2 ... word24'\n");
    }
    out.push('\n');

    out.push_str("2) init:\n");
    let dd = if cfg.data_dir_override.is_empty() {
        String::new()
    } else {
        format!(" --data-dir {}", cfg.data_dir_override)
    };
    let import_flag = if matches!(cfg.seed_mode, SeedMode::Import) {
        " --import"
    } else {
        ""
    };
    out.push_str(&format!(
        "   xgram init --alias {} --role {}{}{}\n\n",
        cfg.alias,
        cfg.role.as_str(),
        dd,
        import_flag,
    ));

    if cfg.adapter_discord || cfg.adapter_telegram {
        out.push_str("3) 어댑터 자격증명 vault 저장:\n");
        if cfg.adapter_discord {
            out.push_str(
                "   xgram vault set --key discord/webhook --value '<URL>' --tags discord\n",
            );
        }
        if cfg.adapter_telegram {
            out.push_str(
                "   xgram vault set --key telegram/bot --value '<TOKEN>' --tags telegram\n",
            );
        }
        out.push('\n');
    }

    if cfg.install_daemon {
        out.push_str("4) systemd sidecar daemon 등록:\n");
        out.push_str(&format!(
            "   xgram daemon-install --binary $(which xgram) --bind {}\n",
            cfg.bind
        ));
        out.push_str("   systemctl --user daemon-reload\n");
        out.push_str("   systemctl --user enable --now openxgram-sidecar.service\n\n");
    }

    if cfg.install_backup_timer {
        out.push_str("5) systemd 주기 cold backup 등록:\n");
        out.push_str("   xgram backup-install --backup-dir ~/.openxgram/backups\n");
        out.push_str("   systemctl --user daemon-reload\n");
        out.push_str("   systemctl --user enable --now openxgram-backup.timer\n\n");
    }

    out.push_str("xgram doctor 로 상태 점검 후 사용하세요.\n");
    out
}

pub fn run_wizard() -> Result<WizardOutcome> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = drive(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    result
}

fn drive<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>) -> Result<WizardOutcome> {
    let mut state = WizardState::initial();
    loop {
        terminal.draw(|f| draw(f, &state))?;
        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            if state.is_terminal() {
                return Ok(state.outcome().expect("terminal state has outcome"));
            }
            state = state.handle(code);
        }
    }
}
