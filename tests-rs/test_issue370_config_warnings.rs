// Issue #370 follow-up: surface warnings for unknown / malformed config
// directives instead of silently ignoring them.
//
// These unit tests drive the parser directly (parse_config_content /
// parse_config_line) and assert on app.config_warnings. The end-to-end proof
// (warnings reaching the user's terminal + source-file status) lives in
// tests/test_issue370_config_warnings.ps1.

use super::*;

fn app() -> AppState {
    AppState::new("cfgwarn_test".to_string())
}

fn warns_contain(app: &AppState, needle: &str) -> bool {
    app.config_warnings.iter().any(|w| w.contains(needle))
}

// ---- Unknown commands ----

#[test]
fn unknown_command_warns() {
    let mut a = app();
    crate::config::parse_config_content(&mut a, "totally-bogus-command foo bar\n");
    assert!(warns_contain(&a, "unknown command: totally-bogus-command"),
        "expected unknown-command warning, got: {:?}", a.config_warnings);
}

#[test]
fn typo_of_real_command_warns() {
    let mut a = app();
    crate::config::parse_config_content(&mut a, "bnid-key x new-window\n");
    assert!(warns_contain(&a, "unknown command: bnid-key"),
        "typo'd command should warn, got: {:?}", a.config_warnings);
}

#[test]
fn known_but_unrouted_command_does_not_warn() {
    // new-window / display-message are valid psmux commands the config parser
    // does not itself route; they must stay silent (no false positive).
    let mut a = app();
    crate::config::parse_config_content(&mut a, "new-window -n foo\ndisplay-message hello\n");
    assert!(!warns_contain(&a, "unknown command"),
        "known commands must not warn, got: {:?}", a.config_warnings);
}

#[test]
fn routed_commands_do_not_warn() {
    let mut a = app();
    crate::config::parse_config_content(&mut a,
        "set -g escape-time 100\nbind-key x split-window\nunbind-key y\nset-hook -g after-new-window \"display-message hi\"\n");
    assert!(a.config_warnings.is_empty(),
        "valid routed directives must not warn, got: {:?}", a.config_warnings);
}

#[test]
fn kill_window_missing_target_warns_before_storing_deferred_commands() {
    let mut a = app();

    crate::config::parse_config_content(
        &mut a,
        "kill-window -t\nbind-key x killw -t\nset-hook pane-died kill-window -t\n",
    );

    assert_eq!(
        a.config_warnings
            .iter()
            .filter(|warning| warning.contains("-t expects an argument"))
            .count(),
        3
    );
    assert!(a.key_tables.get("prefix").is_none_or(|bindings| {
        bindings
            .iter()
            .all(|binding| binding.key.0 != crossterm::event::KeyCode::Char('x'))
    }));
    assert!(!a.hooks.contains_key("pane-died"));
}

// ---- Unknown options ----

#[test]
fn unknown_option_warns() {
    let mut a = app();
    crate::config::parse_config_content(&mut a, "set -g this-is-not-a-real-option hello\n");
    assert!(warns_contain(&a, "unknown option 'this-is-not-a-real-option'"),
        "expected unknown-option warning, got: {:?}", a.config_warnings);
}

#[test]
fn at_prefixed_user_option_never_warns() {
    let mut a = app();
    crate::config::parse_config_content(&mut a, "set -g @my-plugin-setting yes\nset -g @plugin 'foo/bar'\n");
    assert!(!warns_contain(&a, "unknown option"),
        "@-options are user/plugin options and must never warn, got: {:?}", a.config_warnings);
}

#[test]
fn array_style_option_does_not_warn() {
    // command-alias[0]=... and terminal-overrides[1] are valid tmux array syntax.
    let mut a = app();
    crate::config::parse_config_content(&mut a,
        "set -g command-alias[0] foo=display-message\nset -g terminal-overrides[1] xterm\n");
    assert!(!warns_contain(&a, "unknown option"),
        "array-style option keys must not warn, got: {:?}", a.config_warnings);
}

#[test]
fn known_options_do_not_warn() {
    let mut a = app();
    crate::config::parse_config_content(&mut a,
        "set -g status-left \"[X]\"\nset -g mouse on\nset -g history-limit 5000\nset -g pane-border-status top\n");
    assert!(a.config_warnings.is_empty(),
        "known options must not warn, got: {:?}", a.config_warnings);
}

// ---- Malformed values ----

#[test]
fn malformed_numeric_value_warns() {
    let mut a = app();
    crate::config::parse_config_content(&mut a, "set -g escape-time notanumber\n");
    assert!(warns_contain(&a, "invalid value 'notanumber' for option 'escape-time'"),
        "expected malformed-number warning, got: {:?}", a.config_warnings);
}

#[test]
fn malformed_numeric_keeps_prior_value() {
    // A bad value must not corrupt the option; the good earlier value stays.
    let mut a = app();
    crate::config::parse_config_content(&mut a, "set -g escape-time 77\nset -g escape-time notanumber\n");
    assert_eq!(a.escape_time_ms, 77, "bad value must not overwrite the good one");
    assert!(warns_contain(&a, "expected a number"));
}

#[test]
fn malformed_boolean_value_warns() {
    let mut a = app();
    crate::config::parse_config_content(&mut a, "set -g mouse maybe\n");
    assert!(warns_contain(&a, "invalid value 'maybe' for option 'mouse'"),
        "expected malformed-boolean warning, got: {:?}", a.config_warnings);
}

#[test]
fn valid_numeric_and_boolean_do_not_warn() {
    let mut a = app();
    crate::config::parse_config_content(&mut a,
        "set -g escape-time 250\nset -g mouse on\nset -g mouse off\nset -g history-limit 1000\nset -g status 2\n");
    assert!(a.config_warnings.is_empty(),
        "valid values must not warn (incl. numeric 'status 2'), got: {:?}", a.config_warnings);
}

// ---- Missing args ----

#[test]
fn bare_set_option_warns() {
    let mut a = app();
    crate::config::parse_config_content(&mut a, "set -g\n");
    assert!(warns_contain(&a, "set-option requires an option name"),
        "bare set must warn, got: {:?}", a.config_warnings);
}

// ---- Line numbers & good-with-bad coexistence ----

#[test]
fn warning_carries_line_number_when_file_known() {
    let mut a = app();
    // Simulate a sourced file context so warn_config prefixes file:line.
    // (current_config_file is set by load_config/source_file in production.)
    let tmp = std::env::temp_dir().join("psmux_iss370_cfgwarn_lineno.conf");
    std::fs::write(&tmp, "set -g escape-time 10\nbogus-directive here\n").unwrap();
    crate::config::source_file(&mut a, &tmp.display().to_string());
    assert!(a.config_warnings.iter().any(|w| w.contains("bogus-directive") && w.contains(":2:")),
        "warning should be tagged with line 2, got: {:?}", a.config_warnings);
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn good_directives_apply_despite_bad_ones() {
    let mut a = app();
    crate::config::parse_config_content(&mut a,
        "set -g escape-time 123\nbogus-cmd x\nset -g status-left \"[OK]\"\nset -g not-real-opt v\n");
    assert_eq!(a.escape_time_ms, 123, "good numeric applied");
    assert_eq!(a.status_left, "[OK]", "good string applied");
    assert!(warns_contain(&a, "unknown command: bogus-cmd"));
    assert!(warns_contain(&a, "unknown option 'not-real-opt'"));
}
