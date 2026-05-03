//! wizard 9-step state machine + render 통합 테스트.

use openxgram_cli::wizard::{
    draw, render_done, Role, SeedMode, WizardConfig, WizardOutcome, WizardState,
};
use ratatui::{backend::TestBackend, crossterm::event::KeyCode, Terminal};

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

/// 입력값을 keystrokes 로 분해해서 alias 단계 통과
fn enter_text(state: WizardState, text: &str) -> WizardState {
    let mut s = state;
    for c in text.chars() {
        s = s.handle(KeyCode::Char(c));
    }
    s
}

#[test]
fn welcome_to_done_via_full_9_steps() {
    let mut state = WizardState::initial();
    assert_eq!(state, WizardState::Welcome);

    state = state.handle(KeyCode::Enter); // 1 → 2 Alias
    state = enter_text(state, "gcp-main");
    state = state.handle(KeyCode::Enter); // 2 → 3 Role

    state = state.handle(KeyCode::Char('1')); // primary
    state = state.handle(KeyCode::Enter); // 3 → 4 DataDir

    state = state.handle(KeyCode::Enter); // blank → default

    state = state.handle(KeyCode::Char('N')); // SeedMode = New
    state = state.handle(KeyCode::Enter);

    state = state.handle(KeyCode::Char('D')); // adapter discord ON
    state = state.handle(KeyCode::Enter);

    // bind 기본 유지
    state = state.handle(KeyCode::Enter);

    state = state.handle(KeyCode::Char('Y')); // daemon Y
    state = state.handle(KeyCode::Enter);

    state = state.handle(KeyCode::Char('Y')); // backup Y
    state = state.handle(KeyCode::Enter);

    // Confirm → Done
    assert!(matches!(state, WizardState::Confirm { .. }));
    state = state.handle(KeyCode::Enter);
    assert!(state.is_terminal());

    let outcome = state.outcome().unwrap();
    let WizardOutcome::Completed { cfg } = outcome else {
        panic!("expected Completed");
    };
    assert_eq!(cfg.alias, "gcp-main");
    assert_eq!(cfg.role, Role::Primary);
    assert_eq!(cfg.seed_mode, SeedMode::New);
    assert!(cfg.adapter_discord);
    assert!(!cfg.adapter_telegram);
    assert!(cfg.install_daemon);
    assert!(cfg.install_backup_timer);
    assert_eq!(cfg.bind, "127.0.0.1:7300");
}

#[test]
fn welcome_esc_cancels() {
    let state = WizardState::initial().handle(KeyCode::Esc);
    assert_eq!(state, WizardState::Cancelled);
    assert_eq!(state.outcome().unwrap(), WizardOutcome::Cancelled);
}

#[test]
fn welcome_q_cancels() {
    let state = WizardState::initial().handle(KeyCode::Char('q'));
    assert_eq!(state, WizardState::Cancelled);
}

#[test]
fn alias_empty_does_not_advance() {
    let state = WizardState::initial()
        .handle(KeyCode::Enter)
        .handle(KeyCode::Enter); // 비어있음 → 그대로
    assert!(matches!(state, WizardState::Alias { .. }));
}

#[test]
fn alias_backspace_removes_char() {
    let state =
        enter_text(WizardState::initial().handle(KeyCode::Enter), "abc").handle(KeyCode::Backspace);
    if let WizardState::Alias { cfg } = state {
        assert_eq!(cfg.alias, "ab");
    } else {
        panic!();
    }
}

#[test]
fn alias_esc_returns_to_welcome() {
    let state = WizardState::initial()
        .handle(KeyCode::Enter)
        .handle(KeyCode::Char('x'))
        .handle(KeyCode::Esc);
    assert_eq!(state, WizardState::Welcome);
}

#[test]
fn role_keys_select_correctly() {
    let mut state =
        enter_text(WizardState::initial().handle(KeyCode::Enter), "a").handle(KeyCode::Enter);
    state = state.handle(KeyCode::Char('2'));
    if let WizardState::RoleStep { cfg } = &state {
        assert_eq!(cfg.role, Role::Secondary);
    } else {
        panic!();
    }
    state = state.handle(KeyCode::Char('3'));
    if let WizardState::RoleStep { cfg } = state {
        assert_eq!(cfg.role, Role::Worker);
    } else {
        panic!();
    }
}

#[test]
fn adapter_toggle_d_t() {
    // Welcome → Alias → Role → DataDir → SeedMode → Adapter
    let state = enter_text(WizardState::initial().handle(KeyCode::Enter), "a")
        .handle(KeyCode::Enter) // → Role
        .handle(KeyCode::Enter) // → DataDir
        .handle(KeyCode::Enter) // → SeedMode
        .handle(KeyCode::Enter) // → Adapter
        .handle(KeyCode::Char('d'))
        .handle(KeyCode::Char('t'))
        .handle(KeyCode::Char('d')); // 다시 끄기
    if let WizardState::Adapter { cfg } = state {
        assert!(!cfg.adapter_discord);
        assert!(cfg.adapter_telegram);
    } else {
        panic!();
    }
}

#[test]
fn confirm_back_returns_to_backup_preserving_cfg() {
    // 끝까지 가서 Confirm 도달 후 b → Backup, cfg 보존
    let mut state = WizardState::initial().handle(KeyCode::Enter);
    state = enter_text(state, "alpha").handle(KeyCode::Enter); // → Role
    state = state.handle(KeyCode::Enter); // → DataDir
    state = state.handle(KeyCode::Enter); // → SeedMode
    state = state.handle(KeyCode::Enter); // → Adapter
    state = state.handle(KeyCode::Enter); // → Bind
    state = state.handle(KeyCode::Enter); // → Daemon
    state = state.handle(KeyCode::Enter); // → Backup
    state = state.handle(KeyCode::Char('Y')); // backup Y
    state = state.handle(KeyCode::Enter); // → Confirm
    assert!(matches!(state, WizardState::Confirm { .. }));
    state = state.handle(KeyCode::Char('b')); // → Backup, cfg 보존
    if let WizardState::Backup { cfg } = state {
        assert_eq!(cfg.alias, "alpha");
        assert!(cfg.install_backup_timer);
    } else {
        panic!();
    }
}

#[test]
fn render_each_state_shows_step_marker() {
    let cfg = WizardConfig {
        alias: "test".into(),
        ..Default::default()
    };
    let states_and_markers = [
        (WizardState::Welcome, None),
        (WizardState::Alias { cfg: cfg.clone() }, Some("[1/9]")),
        (WizardState::RoleStep { cfg: cfg.clone() }, Some("[2/9]")),
        (WizardState::DataDir { cfg: cfg.clone() }, Some("[3/9]")),
        (WizardState::SeedMode { cfg: cfg.clone() }, Some("[4/9]")),
        (WizardState::Adapter { cfg: cfg.clone() }, Some("[5/9]")),
        (WizardState::Bind { cfg: cfg.clone() }, Some("[6/9]")),
        (WizardState::Daemon { cfg: cfg.clone() }, Some("[7/9]")),
        (WizardState::Backup { cfg: cfg.clone() }, Some("[8/9]")),
        (WizardState::Confirm { cfg: cfg.clone() }, Some("[9/9]")),
        (WizardState::Done { cfg }, None), // Done 은 step 마커 대신 명령 출력
    ];
    for (state, marker) in states_and_markers {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &state)).unwrap();
        let text = buffer_text(&terminal);
        if let Some(m) = marker {
            assert!(text.contains(m), "state={state:?}, expected '{m}'");
        }
    }
}

#[test]
fn render_done_includes_init_command_and_optional_steps() {
    let mut cfg = WizardConfig {
        alias: "gcp-main".into(),
        ..Default::default()
    };
    cfg.install_daemon = true;
    cfg.install_backup_timer = true;
    cfg.adapter_discord = true;
    cfg.seed_mode = SeedMode::Import;

    let body = render_done(&cfg);
    assert!(body.contains("XGRAM_KEYSTORE_PASSWORD"));
    assert!(body.contains("XGRAM_SEED")); // import 모드
    assert!(body.contains("xgram init --alias gcp-main --role primary"));
    assert!(body.contains("--import"));
    assert!(body.contains("xgram vault set --key discord/webhook"));
    assert!(body.contains("xgram daemon-install"));
    assert!(body.contains("xgram backup-install"));
}
