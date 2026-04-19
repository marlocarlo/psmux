// Tests for issue #193: scroll-enter-copy-mode option
//
// Verifies that:
// 1. The option defaults to on (true)
// 2. `set -g scroll-enter-copy-mode off` disables it
// 3. `set -g scroll-enter-copy-mode on` re-enables it
// 4. show-options includes the option
// 5. get_option_value returns correct value

use super::*;

fn mock_app() -> crate::types::AppState {
    crate::types::AppState::new("test_session".to_string())
}

#[test]
fn scroll_enter_copy_mode_defaults_to_on() {
    let app = mock_app();
    assert!(app.scroll_enter_copy_mode, "scroll_enter_copy_mode should default to true");
}

#[test]
fn set_scroll_enter_copy_mode_off() {
    let mut app = mock_app();
    parse_config_content(&mut app, "set -g scroll-enter-copy-mode off");
    assert!(!app.scroll_enter_copy_mode);
}

#[test]
fn set_scroll_enter_copy_mode_on() {
    let mut app = mock_app();
    app.scroll_enter_copy_mode = false;
    parse_config_content(&mut app, "set -g scroll-enter-copy-mode on");
    assert!(app.scroll_enter_copy_mode);
}

#[test]
fn show_options_includes_scroll_enter_copy_mode() {
    let app = mock_app();
    let output = crate::server::options::get_option_value(&app, "scroll-enter-copy-mode");
    assert_eq!(output, "on");
}

#[test]
fn show_options_scroll_enter_copy_mode_off() {
    let mut app = mock_app();
    app.scroll_enter_copy_mode = false;
    let output = crate::server::options::get_option_value(&app, "scroll-enter-copy-mode");
    assert_eq!(output, "off");
}

#[test]
fn apply_set_option_scroll_enter_copy_mode_off() {
    let mut app = mock_app();
    assert!(app.scroll_enter_copy_mode);
    crate::server::options::apply_set_option(&mut app, "scroll-enter-copy-mode", "off", false);
    assert!(!app.scroll_enter_copy_mode, "apply_set_option should set scroll_enter_copy_mode to false");
}

#[test]
fn apply_set_option_scroll_enter_copy_mode_on() {
    let mut app = mock_app();
    app.scroll_enter_copy_mode = false;
    crate::server::options::apply_set_option(&mut app, "scroll-enter-copy-mode", "on", false);
    assert!(app.scroll_enter_copy_mode, "apply_set_option should set scroll_enter_copy_mode to true");
}
