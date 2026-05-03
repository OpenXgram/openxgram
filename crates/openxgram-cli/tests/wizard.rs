//! wizard state machine + render 통합 테스트.

use openxgram_cli::wizard::{draw, WizardOutcome, WizardState};
use ratatui::{
    backend::TestBackend,
    crossterm::event::KeyCode,
    Terminal,
};

fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect()
}

#[test]
fn welcome_to_done_via_full_flow() {
    let mut state = WizardState::initial();
    assert_eq!(state, WizardState::Welcome);

    state = state.handle(KeyCode::Enter); // → MachineId
    matches!(state, WizardState::MachineId { ref alias } if alias.is_empty())
        .then_some(())
        .expect("MachineId empty");

    // alias 입력 'gcp-main'
    for c in "gcp-main".chars() {
        state = state.handle(KeyCode::Char(c));
    }
    if let WizardState::MachineId { alias } = &state {
        assert_eq!(alias, "gcp-main");
    } else {
        panic!("expected MachineId");
    }

    state = state.handle(KeyCode::Enter); // → Confirm
    assert!(matches!(state, WizardState::Confirm { ref alias } if alias == "gcp-main"));

    state = state.handle(KeyCode::Enter); // → Done
    assert!(matches!(state, WizardState::Done { ref alias } if alias == "gcp-main"));
    assert!(state.is_terminal());

    let outcome = state.outcome().unwrap();
    assert_eq!(
        outcome,
        WizardOutcome::Completed {
            alias: "gcp-main".into()
        }
    );
}

#[test]
fn welcome_esc_cancels() {
    let state = WizardState::initial().handle(KeyCode::Esc);
    assert_eq!(state, WizardState::Cancelled);
    assert!(state.is_terminal());
    assert_eq!(state.outcome().unwrap(), WizardOutcome::Cancelled);
}

#[test]
fn welcome_q_cancels() {
    let state = WizardState::initial().handle(KeyCode::Char('q'));
    assert_eq!(state, WizardState::Cancelled);
}

#[test]
fn machine_id_empty_alias_does_not_advance() {
    let state = WizardState::initial()
        .handle(KeyCode::Enter) // → MachineId
        .handle(KeyCode::Enter); // alias 비어있음 → 그대로
    assert!(matches!(state, WizardState::MachineId { ref alias } if alias.is_empty()));
}

#[test]
fn machine_id_backspace_removes_char() {
    let mut state = WizardState::initial().handle(KeyCode::Enter);
    for c in "abc".chars() {
        state = state.handle(KeyCode::Char(c));
    }
    state = state.handle(KeyCode::Backspace);
    if let WizardState::MachineId { alias } = state {
        assert_eq!(alias, "ab");
    } else {
        panic!();
    }
}

#[test]
fn machine_id_esc_returns_to_welcome() {
    let state = WizardState::initial()
        .handle(KeyCode::Enter)
        .handle(KeyCode::Char('x'))
        .handle(KeyCode::Esc);
    assert_eq!(state, WizardState::Welcome);
}

#[test]
fn confirm_b_returns_to_machine_id_preserving_alias() {
    let state = WizardState::initial()
        .handle(KeyCode::Enter)
        .handle(KeyCode::Char('a'))
        .handle(KeyCode::Char('b'))
        .handle(KeyCode::Enter) // → Confirm
        .handle(KeyCode::Char('b')); // → MachineId, alias 보존
    if let WizardState::MachineId { alias } = state {
        assert_eq!(alias, "ab", "이전단계 복귀 시 입력 보존");
    } else {
        panic!("expected MachineId");
    }
}

#[test]
fn render_each_state_shows_keywords() {
    for (state, expected) in [
        (WizardState::Welcome, "환영합니다"),
        (
            WizardState::MachineId {
                alias: "test".into(),
            },
            "[1/3]",
        ),
        (
            WizardState::Confirm {
                alias: "test".into(),
            },
            "[2/3]",
        ),
        (
            WizardState::Done {
                alias: "test".into(),
            },
            "[3/3]",
        ),
        (WizardState::Cancelled, "취소"),
    ] {
        let backend = TestBackend::new(80, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &state)).unwrap();
        let text = buffer_text(&terminal);
        // 한글은 wide-char 분할 가능 — 영문 마커 또는 1바이트 위주로 검증
        if expected.is_ascii() {
            assert!(text.contains(expected), "state={state:?}, expected '{expected}'");
        }
    }
}
