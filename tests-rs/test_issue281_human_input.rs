// Issue #281 (strategy A): psmux exports live human text-input via the
// `@human-input-marker` session option, so external tooling (e.g.
// claude-loop) can tell a human typing apart from programmatic send-keys
// — busy included, without a nested PTY proxy. These tests pin:
//   1. the key classifier (printable text vs control / Ctrl / Alt / nav),
//   2. that note_human_input touches the marker only on real text input,
//      only when the option is set, and that rapid typing is throttled.

use crate::input::{is_human_text_key, note_human_input};
use crate::types::AppState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn k(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

/// Unique temp path per call so parallel tests don't collide.
fn tmp_marker(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "psmux_test_human_{}_{}_{}.marker",
        tag,
        std::process::id(),
        nanos
    ))
}

// === classifier ===

#[test]
fn plain_text_keys_are_human_input() {
    assert!(is_human_text_key(&k(KeyCode::Char('a'), KeyModifiers::NONE)));
    assert!(is_human_text_key(&k(KeyCode::Char('Z'), KeyModifiers::SHIFT))); // capitals
    assert!(is_human_text_key(&k(KeyCode::Char(' '), KeyModifiers::NONE))); // space is text
    assert!(is_human_text_key(&k(KeyCode::Char('é'), KeyModifiers::NONE))); // non-ASCII
}

#[test]
fn control_and_modified_keys_are_not_human_input() {
    assert!(!is_human_text_key(&k(KeyCode::Char('c'), KeyModifiers::CONTROL))); // Ctrl-C
    assert!(!is_human_text_key(&k(KeyCode::Char('x'), KeyModifiers::ALT))); // Alt-x
    assert!(!is_human_text_key(&k(KeyCode::Enter, KeyModifiers::NONE)));
    assert!(!is_human_text_key(&k(KeyCode::Tab, KeyModifiers::NONE)));
    assert!(!is_human_text_key(&k(KeyCode::Backspace, KeyModifiers::NONE)));
    assert!(!is_human_text_key(&k(KeyCode::Left, KeyModifiers::NONE))); // navigation
}

// === marker side effect ===

#[test]
fn text_key_touches_marker_when_option_set() {
    let marker = tmp_marker("touch");
    let _ = std::fs::remove_file(&marker);
    let mut app = AppState::new("t".into());
    app.user_options
        .insert("@human-input-marker".into(), marker.to_string_lossy().into_owned());
    note_human_input(&mut app, &k(KeyCode::Char('h'), KeyModifiers::NONE));
    assert!(marker.exists(), "a text keystroke must touch the marker");
    let _ = std::fs::remove_file(&marker);
}

#[test]
fn control_key_does_not_touch_marker() {
    let marker = tmp_marker("ctrl");
    let _ = std::fs::remove_file(&marker);
    let mut app = AppState::new("t".into());
    app.user_options
        .insert("@human-input-marker".into(), marker.to_string_lossy().into_owned());
    note_human_input(&mut app, &k(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!marker.exists(), "Enter must not touch the marker");
}

#[test]
fn unset_option_is_a_noop() {
    // No option → no write, no panic, throttle state untouched.
    let mut app = AppState::new("t".into());
    note_human_input(&mut app, &k(KeyCode::Char('h'), KeyModifiers::NONE));
    assert!(app.last_human_input.is_none());
}

#[test]
fn rapid_typing_is_throttled() {
    let marker = tmp_marker("throttle");
    let _ = std::fs::remove_file(&marker);
    let mut app = AppState::new("t".into());
    app.user_options
        .insert("@human-input-marker".into(), marker.to_string_lossy().into_owned());
    note_human_input(&mut app, &k(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(marker.exists());
    std::fs::remove_file(&marker).unwrap();
    // An immediate second keystroke (< 200ms) is throttled → not re-created.
    note_human_input(&mut app, &k(KeyCode::Char('b'), KeyModifiers::NONE));
    assert!(!marker.exists(), "a rapid second keystroke should be throttled");
    let _ = std::fs::remove_file(&marker);
}
