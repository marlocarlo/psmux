/// Issue #179: bind-key with uppercase letters treats them as lowercase
/// (Shift+key not distinguished).
///
/// The user reports: `bind I display-message "test"` from command mode causes
/// Prefix+i to trigger the message but Prefix+Shift+I does nothing.
///
/// Root cause: execute_command_string's explicit "bind-key" handler sends the
/// command to the TCP server (when control_port is set) and does NOT apply it
/// locally. The TCP server at app.rs has no handler for "bind-key" so the
/// command is silently dropped by `_ => {}`. The user sees a pre-existing
/// lowercase `i` binding responding, creating the illusion that uppercase was
/// mapped to lowercase.

use super::*;
use crossterm::event::{KeyCode, KeyModifiers};
use crate::config::{parse_key_name, parse_bind_key, normalize_key_for_binding};

fn mock_app() -> AppState {
    let mut app = AppState::new("test_session".to_string());
    app.window_base_index = 0;
    app.pane_base_index = 0;
    app
}

// ===================================================================
// PART 1: Prove the parsing and matching logic IS correct for uppercase
// (This rules out the parsing layer as the cause.)
// ===================================================================

#[test]
fn parse_key_name_uppercase_i() {
    let result = parse_key_name("I").unwrap();
    assert_eq!(result, (KeyCode::Char('I'), KeyModifiers::NONE),
        "parse_key_name('I') should return Char('I'), not Char('i')");
}

#[test]
fn parse_key_name_lowercase_i() {
    let result = parse_key_name("i").unwrap();
    assert_eq!(result, (KeyCode::Char('i'), KeyModifiers::NONE),
        "parse_key_name('i') should return Char('i')");
}

#[test]
fn parse_key_name_uppercase_r() {
    let result = parse_key_name("R").unwrap();
    assert_eq!(result, (KeyCode::Char('R'), KeyModifiers::NONE),
        "parse_key_name('R') should return Char('R')");
}

#[test]
fn normalize_preserves_uppercase_char() {
    let key = (KeyCode::Char('I'), KeyModifiers::NONE);
    let normalized = normalize_key_for_binding(key);
    assert_eq!(normalized, (KeyCode::Char('I'), KeyModifiers::NONE),
        "normalize should preserve Char('I')");
}

#[test]
fn normalize_shift_i_matches_uppercase_binding() {
    // When user presses Shift+I, crossterm reports Char('I') with SHIFT modifier.
    // After normalization (strip SHIFT from Char keys), this should become Char('I') NONE.
    let input = (KeyCode::Char('I'), KeyModifiers::SHIFT);
    let normalized_input = normalize_key_for_binding(input);

    // The stored binding for uppercase I is (Char('I'), NONE).
    let binding = (KeyCode::Char('I'), KeyModifiers::NONE);

    assert_eq!(normalized_input, binding,
        "Shift+I input should match binding for uppercase 'I'");
}

#[test]
fn lowercase_i_does_not_match_uppercase_binding() {
    let input = (KeyCode::Char('i'), KeyModifiers::NONE);
    let normalized_input = normalize_key_for_binding(input);

    let binding = (KeyCode::Char('I'), KeyModifiers::NONE);

    assert_ne!(normalized_input, binding,
        "plain 'i' input should NOT match binding for uppercase 'I'");
}

// ===================================================================
// PART 2: Prove parse_bind_key correctly registers uppercase bindings
// (This rules out the binding storage layer as the cause.)
// ===================================================================

#[test]
fn parse_bind_key_stores_uppercase_binding() {
    let mut app = mock_app();
    parse_bind_key(&mut app, "bind I display-message \"test\"");

    let has_uppercase_i = app.key_tables.get("prefix")
        .map(|t| t.iter().any(|b| b.key == (KeyCode::Char('I'), KeyModifiers::NONE)))
        .unwrap_or(false);

    assert!(has_uppercase_i,
        "parse_bind_key should store binding with uppercase Char('I')");
}

#[test]
fn parse_bind_key_stores_lowercase_binding_separately() {
    let mut app = mock_app();
    parse_bind_key(&mut app, "bind i display-message \"lower\"");
    parse_bind_key(&mut app, "bind I display-message \"upper\"");

    let prefix = app.key_tables.get("prefix").expect("prefix table should exist");
    let has_lower = prefix.iter().any(|b| b.key == (KeyCode::Char('i'), KeyModifiers::NONE));
    let has_upper = prefix.iter().any(|b| b.key == (KeyCode::Char('I'), KeyModifiers::NONE));

    assert!(has_lower, "should have lowercase 'i' binding");
    assert!(has_upper, "should have uppercase 'I' binding");
    assert_eq!(prefix.iter().filter(|b| matches!(b.key.0, KeyCode::Char('i') | KeyCode::Char('I'))).count(), 2,
        "lowercase and uppercase bindings should be separate entries");
}

// ===================================================================
// PART 3: Prove the ACTUAL BUG: execute_command_string drops bind-key
// when control_port is set (command mode path).
//
// Root cause: The explicit "bind-key"|"bind" match arm does Either/Or:
//   control_port Some => send to TCP (which drops it)
//   control_port None => parse_config_line (works)
// While the catch-all at the bottom does BOTH: apply locally + forward.
// ===================================================================

#[test]
fn execute_command_string_bind_key_with_control_port_applies_locally() {
    let mut app = mock_app();
    // Set a bogus control port (no server listening, send will silently fail)
    app.control_port = Some(1);
    app.session_key = "test".to_string();

    let _ = execute_command_string(&mut app, "bind I display-message \"test\"");

    let has_binding = app.key_tables.get("prefix")
        .map(|t| t.iter().any(|b| b.key == (KeyCode::Char('I'), KeyModifiers::NONE)))
        .unwrap_or(false);

    // FIX #179: bind-key must always apply locally, even when control_port is set.
    // Previously the explicit handler only sent to TCP (which dropped it) and
    // never called parse_config_line.
    assert!(has_binding,
        "bind-key from command mode must register locally even when control_port is set");
}

#[test]
fn execute_command_string_bind_key_without_control_port_works() {
    let mut app = mock_app();
    app.control_port = None;

    let _ = execute_command_string(&mut app, "bind I display-message \"test\"");

    let has_binding = app.key_tables.get("prefix")
        .map(|t| t.iter().any(|b| b.key == (KeyCode::Char('I'), KeyModifiers::NONE)))
        .unwrap_or(false);

    assert!(has_binding,
        "bind-key should register locally when control_port is None (standalone mode)");
}

#[test]
fn execute_command_string_unbind_key_with_control_port_applies_locally() {
    let mut app = mock_app();
    // First register a binding locally
    parse_bind_key(&mut app, "bind I display-message \"test\"");
    assert!(app.key_tables.get("prefix").unwrap().iter().any(|b| b.key == (KeyCode::Char('I'), KeyModifiers::NONE)),
        "pre-condition: binding should exist");

    // Now set a control port and unbind from command mode
    app.control_port = Some(1);
    app.session_key = "test".to_string();

    let _ = execute_command_string(&mut app, "unbind I");

    // FIX #179: unbind-key must apply locally and actually remove the binding
    let still_has_binding = app.key_tables.get("prefix")
        .map(|t| t.iter().any(|b| b.key == (KeyCode::Char('I'), KeyModifiers::NONE)))
        .unwrap_or(false);

    assert!(!still_has_binding,
        "unbind-key from command mode must remove the binding locally even when control_port is set");
}

// ===================================================================
// PART 4: Prove the same set-option|set bug exists (same pattern)
// The explicit handler only sends to TCP, never applies locally.
// ===================================================================

#[test]
fn execute_command_string_set_option_with_control_port_applies_locally() {
    let mut app = mock_app();
    app.control_port = Some(1);
    app.session_key = "test".to_string();

    let old_prefix = app.prefix_key;
    // Set an option via command mode
    let _ = execute_command_string(&mut app, "set prefix C-a");

    // FIX #179: set-option must apply locally even when control_port is set
    assert_ne!(app.prefix_key, old_prefix,
        "set-option from command mode must apply locally even when control_port is set");
}
