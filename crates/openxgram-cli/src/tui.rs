//! xgram tui — ratatui welcome 화면 (Phase 1 baseline).
//!
//! 단일 화면: 헤더 / Status (manifest 정보 또는 미설치 안내) / Footer.
//! Q / Esc 키 → 종료. SPEC §10 의 9단계 인터랙티브 마법사는 후속 PR.
//!
//! 통합 테스트는 ratatui::backend::TestBackend 로 draw() 만 검증한다 (실제
//! 터미널 raw mode 진입은 unit test 환경에서 불안정).

use std::io;
use std::path::{Path, PathBuf};

use anyhow::Result;
use openxgram_manifest::InstallManifest;
use ratatui::{
    crossterm::{
        event::{self, Event, KeyCode},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};

#[derive(Debug, Clone)]
pub struct TuiOpts {
    pub data_dir: PathBuf,
}

pub fn run_tui(opts: &TuiOpts) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let manifest = InstallManifest::read(opts.data_dir.join("install-manifest.json")).ok();

    let outcome = (|| -> Result<()> {
        loop {
            terminal.draw(|f| draw(f, manifest.as_ref(), &opts.data_dir))?;
            if let Event::Key(k) = event::read()? {
                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    return Ok(());
                }
            }
        }
    })();

    // 종료 시점에 raw mode·alt screen 항상 복구 (silent fail 방지)
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    outcome
}

pub fn draw(frame: &mut Frame, manifest: Option<&InstallManifest>, data_dir: &Path) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);

    let title = Paragraph::new(Line::from(vec![
        Span::styled("xgram TUI", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" — OpenXgram (Phase 1)"),
    ]))
    .block(Block::default().borders(Borders::ALL).title("OpenXgram"));
    frame.render_widget(title, chunks[0]);

    let body_text = match manifest {
        Some(m) => format!(
            "alias    : {}\nrole     : {}\nos/arch  : {} / {}\nhostname : {}\ndata_dir : {}\nkeys     : {}\nports    : {}",
            m.machine.alias,
            m.machine.role,
            m.machine.os,
            m.machine.arch,
            m.machine.hostname,
            data_dir.display(),
            m.registered_keys.len(),
            m.ports.len(),
        ),
        None => format!(
            "미설치 ({}).\n`xgram init --alias <NAME>` 으로 설치하세요.",
            data_dir.display()
        ),
    };
    let body = Paragraph::new(body_text)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, chunks[1]);

    let footer =
        Paragraph::new(Line::from("Q / Esc: 종료")).block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}
