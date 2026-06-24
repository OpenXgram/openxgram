//! Session↔identity binding self-heal — pure-logic unit tests.
//!
//! Root fix for the 5x-rebuild bug (.handoff/ROOT-CAUSE-5x-rebuild-CONFIRMED.md):
//! auto-seed must REBIND a peer whose `session_identifier` points at a tmux session
//! that is no longer live (self-heal on restart), while PRESERVING bindings that
//! still point at a live session (do not clobber legit UI overrides to a live session).
//!
//! These tests cover the pure decision helper `daemon::should_rebind`, which the
//! auto-seed UPDATE guard delegates to — so the test exercises the real logic.

use openxgram_cli::daemon::should_rebind;
use std::collections::HashSet;

fn live_set(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| format!("tmux:{n}")).collect()
}

#[test]
fn rebinds_when_current_is_none() {
    let live = live_set(&["aoe_seoul_1"]);
    assert!(should_rebind(None, &live), "NULL binding must be (re)bound");
}

#[test]
fn rebinds_when_current_is_empty() {
    let live = live_set(&["aoe_seoul_1"]);
    assert!(should_rebind(Some(""), &live), "empty-string binding must be (re)bound");
}

#[test]
fn rebinds_when_current_points_at_dead_session() {
    // peer was bound to an OLD session that is no longer in the live set → self-heal.
    let live = live_set(&["aoe_seoul_NEW"]);
    assert!(
        should_rebind(Some("tmux:aoe_seoul_OLD"), &live),
        "stale binding to a dead session must self-heal (rebind)"
    );
}

#[test]
fn preserves_live_binding() {
    // current binding points at a session that IS live → leave it (no clobber).
    let live = live_set(&["aoe_seoul_1", "aoe_gemini_2"]);
    assert!(
        !should_rebind(Some("tmux:aoe_seoul_1"), &live),
        "binding to a live session must be preserved"
    );
}

#[test]
fn preserves_live_binding_even_when_others_live() {
    let live = live_set(&["aoe_gemini_2", "aoe_star_3"]);
    assert!(
        !should_rebind(Some("tmux:aoe_star_3"), &live),
        "a live UI-override binding must be preserved"
    );
}
