//! xgram wizard — 9단계 init 인터랙티브 마법사 (Phase 1 first PR: 3 화면).
//!
//! 마스터 강조 영역: 흐름 점검 (일시중지·재개·이전단계·취소).
//!
//! 화면:
//!   Welcome    → Enter: MachineId / Esc·Q: Cancelled
//!   MachineId  → char 입력 누적 / Backspace 삭제 / Enter (alias 비어있지 않음): Confirm / Esc·B: Welcome
//!   Confirm    → Enter: Done(비대화 명령 출력) / Esc·B: MachineId
//!   Done       → 아무 키: 종료
//!   Cancelled  → 아무 키: 종료
//!
//! Phase 1 first PR: state machine + render. 실제 init 호출은 Done 화면에서
//! 사용자가 보는 "xgram init --alias …" 명령으로 위임. 다음 PR 에서 Done
//! 화면이 직접 init 모듈 호출 + 패스워드 입력 추가.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardState {
    Welcome,
    MachineId { alias: String },
    Confirm { alias: String },
    Done { alias: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardOutcome {
    /// 사용자가 끝까지 진행. alias 확보.
    Completed { alias: String },
    /// 사용자가 Esc/Q 로 취소.
    Cancelled,
}

impl WizardState {
    pub fn initial() -> Self {
        Self::Welcome
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Cancelled)
    }

    /// 키 입력 처리. terminal state 도달 시 outcome 반환.
    pub fn handle(self, key: KeyCode) -> Self {
        match (self, key) {
            // 글로벌: Done/Cancelled 에서 아무 키 = 그대로 (호출자가 종료)
            (s @ (Self::Done { .. } | Self::Cancelled), _) => s,

            // Welcome → Cancel / MachineId
            (Self::Welcome, KeyCode::Esc | KeyCode::Char('q')) => Self::Cancelled,
            (Self::Welcome, KeyCode::Enter) => Self::MachineId {
                alias: String::new(),
            },
            (s @ Self::Welcome, _) => s,

            // MachineId — char 입력 / Backspace / Enter / Esc(B 같이 처리)
            (Self::MachineId { mut alias }, KeyCode::Char(c)) if !c.is_control() => {
                alias.push(c);
                Self::MachineId { alias }
            }
            (Self::MachineId { mut alias }, KeyCode::Backspace) => {
                alias.pop();
                Self::MachineId { alias }
            }
            (Self::MachineId { alias }, KeyCode::Enter) if !alias.is_empty() => {
                Self::Confirm { alias }
            }
            (Self::MachineId { alias: _ }, KeyCode::Esc) => Self::Welcome,
            (s @ Self::MachineId { .. }, _) => s,

            // Confirm — Enter: Done / Esc·B: MachineId 로 복귀(이전단계, 입력 보존)
            (Self::Confirm { alias }, KeyCode::Enter) => Self::Done { alias },
            (Self::Confirm { alias }, KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('B')) => {
                Self::MachineId { alias }
            }
            (s @ Self::Confirm { .. }, _) => s,
        }
    }

    pub fn outcome(&self) -> Option<WizardOutcome> {
        match self {
            Self::Done { alias } => Some(WizardOutcome::Completed {
                alias: alias.clone(),
            }),
            Self::Cancelled => Some(WizardOutcome::Cancelled),
            _ => None,
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
        Span::styled("xgram wizard", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" — OpenXgram init"),
    ]))
    .block(Block::default().borders(Borders::ALL).title("OpenXgram"));
    frame.render_widget(title, chunks[0]);

    let (body, footer_text) = match state {
        WizardState::Welcome => (
            "환영합니다.\n\nOpenXgram 사이드카를 9단계로 설치합니다 (현재 Phase 1: 3단계).\n\n계속하려면 Enter, 종료는 Esc 또는 Q.".to_string(),
            "Enter: 다음  /  Esc · Q: 취소",
        ),
        WizardState::MachineId { alias } => (
            format!(
                "[1/3] 머신 식별\n\n머신 별칭(alias) 을 입력하세요. 예: gcp-main, mac-mini\n\n현재 입력: {}",
                if alias.is_empty() { "(empty)" } else { alias.as_str() }
            ),
            "Enter: 다음 (alias 비어있지 않을 때)  /  Esc: 이전(Welcome)",
        ),
        WizardState::Confirm { alias } => (
            format!(
                "[2/3] 확인\n\n다음 설정으로 진행합니다:\n  alias = {alias}\n  role  = primary (기본)\n  data_dir = ~/.openxgram\n\nEnter 로 완료, B/Esc 로 이전 단계.",
            ),
            "Enter: 완료  /  B · Esc: 이전(MachineId)",
        ),
        WizardState::Done { alias } => (
            format!(
                "[3/3] 완료\n\n다음 명령으로 비대화 모드 init 을 실행하세요:\n\n  XGRAM_KEYSTORE_PASSWORD=<12자+> \\\n    xgram init --alias {alias} --role primary\n\n아무 키나 누르면 종료.",
            ),
            "아무 키: 종료",
        ),
        WizardState::Cancelled => (
            "취소되었습니다.\n\n아무 키나 누르면 종료.".to_string(),
            "아무 키: 종료",
        ),
    };

    let body_widget = Paragraph::new(body)
        .block(Block::default().borders(Borders::ALL).title("Step"))
        .wrap(Wrap { trim: false });
    frame.render_widget(body_widget, chunks[1]);

    let footer =
        Paragraph::new(Line::from(footer_text)).block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
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
            // terminal state 에서 한 키 더 받으면 종료
            if state.is_terminal() {
                return Ok(state.outcome().expect("terminal state has outcome"));
            }
            state = state.handle(code);
        }
    }
}
