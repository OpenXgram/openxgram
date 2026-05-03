//! TUI render 통합 테스트 — TestBackend 로 buffer 검증.

use std::path::PathBuf;

use openxgram_cli::init::{run_init, InitOpts};
use openxgram_cli::tui::draw;
use openxgram_manifest::{InstallManifest, MachineRole};
use ratatui::{backend::TestBackend, Terminal};
use tempfile::tempdir;

fn buffer_contains(terminal: &Terminal<TestBackend>, needle: &str) -> bool {
    let buffer = terminal.backend().buffer();
    let s: String = buffer.content().iter().map(|c| c.symbol()).collect();
    s.contains(needle)
}

#[test]
fn render_uninstalled_state_shows_install_hint() {
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let dir = PathBuf::from("/nonexistent");

    terminal.draw(|f| draw(f, None, &dir)).unwrap();

    // 영문 키워드로 검사 — 한글은 wide-char placeholder 처리로 분할 가능
    assert!(buffer_contains(&terminal, "xgram init"));
    assert!(buffer_contains(&terminal, "Status"));
    assert!(buffer_contains(&terminal, "Q"));
}

#[test]
fn render_installed_state_shows_machine_info() {
    unsafe {
        std::env::set_var("XGRAM_KEYSTORE_PASSWORD", "test-password-12345");
        std::env::remove_var("XGRAM_SEED");
    }
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("openxgram");
    run_init(&InitOpts {
        alias: "tui-test-machine".into(),
        role: MachineRole::Primary,
        data_dir: data_dir.clone(),
        force: false,
        dry_run: false,
        import: false,
    })
    .unwrap();

    let manifest = InstallManifest::read(data_dir.join("install-manifest.json")).unwrap();

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| draw(f, Some(&manifest), &data_dir))
        .unwrap();

    assert!(buffer_contains(&terminal, "tui-test-machine"));
    assert!(buffer_contains(&terminal, "primary"));
    assert!(buffer_contains(&terminal, "linux"));
    assert!(buffer_contains(&terminal, "Status"));
}
