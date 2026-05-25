// `#{pane_last_special_key}` / `_ms` — the last NON-text key on the interactive
// route, by canonical bind-key name. "Special" is the complement of
// `is_text_input_key` (#311): everything that is not printable text input —
// Escape, Enter, Tab, Backspace, arrows, function keys, and Ctrl/Alt chords.
// The name comes from `format_key_binding`, the same renderer `list-keys` uses.
//
// The route separation itself is structural, not tested here: the injected
// route (send-keys / send-paste / send-text) goes through send_text_to_active,
// never forward_key_to_active, so it can't reach this signal.

use crate::config::format_key_binding;
use crate::input::is_text_input_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn is_special(code: KeyCode, mods: KeyModifiers) -> bool {
    !is_text_input_key(&KeyEvent::new(code, mods))
}
fn name(code: KeyCode, mods: KeyModifiers) -> String {
    format_key_binding(&(code, mods))
}

#[test]
fn special_keys_are_classified_and_named() {
    // Each is non-text (so it routes to the special-key signal) and renders to
    // its canonical bind-key name.
    assert!(is_special(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(name(KeyCode::Esc, KeyModifiers::NONE), "Escape");

    assert!(is_special(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(name(KeyCode::Enter, KeyModifiers::NONE), "Enter");

    assert!(is_special(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert_eq!(name(KeyCode::Char('c'), KeyModifiers::CONTROL), "C-c");

    assert!(is_special(KeyCode::Char('a'), KeyModifiers::ALT));
    assert_eq!(name(KeyCode::Char('a'), KeyModifiers::ALT), "M-a");

    assert!(is_special(KeyCode::F(9), KeyModifiers::NONE));
    assert_eq!(name(KeyCode::F(9), KeyModifiers::NONE), "F9");

    assert!(is_special(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(name(KeyCode::Up, KeyModifiers::NONE), "Up");
}

#[test]
fn printable_text_is_not_special() {
    // Plain text routes to #{pane_last_text_input}, never the special-key var.
    assert!(!is_special(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(!is_special(KeyCode::Char('Z'), KeyModifiers::SHIFT)); // capital
    assert!(!is_special(KeyCode::Char(' '), KeyModifiers::NONE)); // space is text
}
