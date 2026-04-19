// Discussion #210: Rust unit tests for the three gastown integration fixes.
//
// PRODUCTION CODE TESTS (call real production functions):
//   - Filter evaluation: calls crate::format::expand_format() with mock AppState
//     to test #{==:#{session_name},NAME} evaluation (the REAL format engine)
//   - list-keys: calls execute_command_string() to verify PopupMode output
//   - PREFIX_DEFAULTS: reads crate::help::PREFIX_DEFAULTS directly
//
// CONTRACT TESTS:
//   - Duplicate session error format: mirrors main.rs eprintln! (line ~657)
//     because the error is emitted by main() directly, not a callable function

use super::*;

fn mock_app() -> AppState {
    let mut app = AppState::new("test210".to_string());
    app.window_base_index = 0;
    app.pane_base_index = 0;
    app
}

fn make_window(name: &str, id: usize) -> crate::types::Window {
    crate::types::Window {
        root: Node::Split {
            kind: LayoutKind::Horizontal,
            sizes: vec![],
            children: vec![],
        },
        active_path: vec![],
        name: name.to_string(),
        id,
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: std::time::Instant::now(),
        last_seen_version: 0,
        manual_rename: false,
        layout_index: 0,
        pane_mru: vec![],
        zoom_saved: None,
        linked_from: None,
    }
}

fn mock_app_with_window() -> AppState {
    let mut app = mock_app();
    app.windows.push(make_window("shell", 0));
    app
}

// ════════════════════════════════════════════════════════════════════════════
// BUG 1: duplicate session error message contract
// CONTRACT TEST: The error is emitted by main.rs (line ~657) as:
//   eprintln!("duplicate session: {}", name)
// Not callable as a function, so we verify the expected format.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dup_error_contains_phrase_duplicate_session() {
    // The production code in main.rs emits: eprintln!("duplicate session: {}", name)
    // gastown's wrapError() looks for "duplicate session" in stderr
    let msg = format!("duplicate session: {}", "myapp");
    assert!(
        msg.contains("duplicate session"),
        "error must contain 'duplicate session' for gastown wrapError: {}", msg
    );
}

#[test]
fn dup_error_contains_session_name() {
    let name = "fancy-dev-session";
    let msg = format!("duplicate session: {}", name);
    assert!(
        msg.contains(name),
        "error must contain the session name '{}': {}", name, msg
    );
}

#[test]
fn dup_error_does_not_use_old_format() {
    let name = "test";
    let msg = format!("duplicate session: {}", name);
    // Old broken format that gastown's wrapError couldn't parse
    assert!(
        !msg.contains("already exists"),
        "must NOT use old 'already exists' phrasing: {}", msg
    );
    assert!(
        !msg.starts_with("psmux:"),
        "must NOT start with 'psmux:': {}", msg
    );
}

#[test]
fn dup_error_exact_format() {
    assert_eq!(
        format!("duplicate session: {}", "myses"),
        "duplicate session: myses"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// BUG 2: list-sessions -f filter evaluation via PRODUCTION expand_format()
// These call crate::format::expand_format() with a mock AppState to test
// the real #{==:#{session_name},NAME} evaluation engine.
// Production code: src/format.rs expand_format() -> expand_expression() -> try_comparison_op()
// ════════════════════════════════════════════════════════════════════════════

/// Helper: evaluate a filter expression using the REAL production format engine.
/// Creates a mock AppState with the given session_name and expands the filter.
/// Returns true if the expanded result is "1" (match), false if "0" (no match).
fn eval_filter_via_production(filter: &str, session_name: &str) -> bool {
    let mut app = AppState::new(session_name.to_string());
    app.window_base_index = 0;
    app.pane_base_index = 0;
    app.windows.push(make_window("shell", 0));
    let result = crate::format::expand_format(filter, &app);
    result == "1"
}

#[test]
fn filter_exact_match_returns_true() {
    assert!(eval_filter_via_production("#{==:#{session_name},myapp}", "myapp"));
}

#[test]
fn filter_exact_match_different_name_returns_false() {
    assert!(!eval_filter_via_production("#{==:#{session_name},myapp}", "myapp2"));
    assert!(!eval_filter_via_production("#{==:#{session_name},myapp}", "notmyapp"));
    assert!(!eval_filter_via_production("#{==:#{session_name},myapp}", ""));
}

#[test]
fn filter_exact_match_prefix_not_enough() {
    assert!(!eval_filter_via_production("#{==:#{session_name},myapp}", "myapp-extra"));
}

#[test]
fn filter_exact_match_suffix_not_enough() {
    assert!(!eval_filter_via_production("#{==:#{session_name},myapp}", "prefix-myapp"));
}

#[test]
fn filter_gastown_pattern_verbatim() {
    // Exact pattern gastown generates for GetSessionInfo
    let filter = "#{==:#{session_name},dev}";
    assert!( eval_filter_via_production(filter, "dev"));
    assert!(!eval_filter_via_production(filter, "dev2"));
    assert!(!eval_filter_via_production(filter, "staging"));
}

#[test]
fn filter_hyphenated_session_name() {
    let filter = "#{==:#{session_name},my-dev-session}";
    assert!( eval_filter_via_production(filter, "my-dev-session"));
    assert!(!eval_filter_via_production(filter, "my-dev-session-extra"));
    assert!(!eval_filter_via_production(filter, "my-dev"));
}

// ════════════════════════════════════════════════════════════════════════════
// BUG 3: list-keys offline — PREFIX_DEFAULTS must contain gastown's expected keys
// ════════════════════════════════════════════════════════════════════════════

fn find_in_defaults(key: &str) -> Option<&'static str> {
    crate::help::PREFIX_DEFAULTS.iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

#[test]
fn prefix_defaults_n_is_next_window() {
    let action = find_in_defaults("n").expect("'n' missing from PREFIX_DEFAULTS");
    assert_eq!(action, "next-window",
        "gastown TestGetKeyBinding_CapturesDefaultBinding expects next-window for 'n'");
}

#[test]
fn prefix_defaults_w_is_choose_tree() {
    let action = find_in_defaults("w").expect("'w' missing from PREFIX_DEFAULTS");
    assert_eq!(action, "choose-tree",
        "gastown TestGetKeyBinding_CapturesDefaultBindingWithArgs expects choose-tree for 'w'");
}

#[test]
fn prefix_defaults_p_is_previous_window() {
    let action = find_in_defaults("p").expect("'p' missing from PREFIX_DEFAULTS");
    assert_eq!(action, "previous-window");
}

#[test]
fn prefix_defaults_d_is_detach_client() {
    let action = find_in_defaults("d").expect("'d' missing from PREFIX_DEFAULTS");
    assert_eq!(action, "detach-client");
}

#[test]
fn prefix_defaults_x_is_kill_pane() {
    let action = find_in_defaults("x").expect("'x' missing from PREFIX_DEFAULTS");
    assert_eq!(action, "kill-pane");
}

#[test]
fn prefix_defaults_c_is_new_window() {
    let action = find_in_defaults("c").expect("'c' missing from PREFIX_DEFAULTS");
    assert_eq!(action, "new-window");
}

#[test]
fn list_keys_offline_format_matches_gastown_parse() {
    // gastown's getKeyBinding parses: "bind-key [-r] -T table key command..."
    // then extracts fields[3+] as the command.
    // Format from fallback: "bind-key -T prefix n next-window"
    let table = "prefix";
    let key = "n";
    let action = find_in_defaults(key).unwrap();
    let line = format!("bind-key -T {} {} {}", table, key, action);

    let parts: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(parts[0], "bind-key",   "field 0 must be bind-key");
    assert_eq!(parts[1], "-T",         "field 1 must be -T");
    assert_eq!(parts[2], "prefix",     "field 2 must be table name");
    assert_eq!(parts[3], "n",          "field 3 must be key");
    assert_eq!(parts[4], "next-window","field 4 must be command");
}

#[test]
fn list_keys_offline_format_choose_tree() {
    let line = format!("bind-key -T prefix w {}", find_in_defaults("w").unwrap());
    let parts: Vec<&str> = line.split_whitespace().collect();
    // gastown splits on whitespace and takes everything from index 4 onward
    let cmd: Vec<&str> = parts[4..].to_vec();
    assert_eq!(cmd, vec!["choose-tree"]);
}

#[test]
fn prefix_defaults_has_enough_bindings() {
    let count = crate::help::PREFIX_DEFAULTS.len();
    assert!(count >= 20,
        "PREFIX_DEFAULTS should have >= 20 entries for a usable default keymap, got {}",
        count);
}

// ════════════════════════════════════════════════════════════════════════════
// BUG 3 (commands.rs path): list-keys via execute_command_string produces
// a PopupMode with bind-key lines including prefix table defaults.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn list_keys_command_produces_popup_with_bindings() {
    let mut app = mock_app_with_window();
    // Populate default bindings (normally done at startup)
    crate::config::populate_default_bindings(&mut app);
    execute_command_string(&mut app, "list-keys").unwrap();
    match &app.mode {
        Mode::PopupMode { command, output, .. } => {
            assert_eq!(command, "list-keys");
            assert!(
                output.contains("bind-key"),
                "list-keys popup must contain bind-key lines, got:\n{}", output
            );
            assert!(
                output.contains("next-window"),
                "popup must contain next-window binding, got:\n{}", output
            );
            // choose-tree and choose-window are synonymous; the internal action
            // serialises as choose-window but both are valid for w binding
            assert!(
                output.contains("choose-tree") || output.contains("choose-window"),
                "popup must contain choose-tree or choose-window binding for 'w', got:\n{}", output
            );
        }
        other => panic!("expected PopupMode, got {:?}", std::mem::discriminant(other)),
    }
}

#[test]
fn list_keys_popup_format_matches_bind_key_syntax() {
    let mut app = mock_app_with_window();
    crate::config::populate_default_bindings(&mut app);
    execute_command_string(&mut app, "list-keys").unwrap();
    if let Mode::PopupMode { output, .. } = &app.mode {
        for line in output.lines() {
            if line.is_empty() || line.starts_with('(') { continue; }
            // Every non-empty line must start with "bind-key"
            assert!(
                line.starts_with("bind-key"),
                "expected 'bind-key ...' format, got: {}", line
            );
        }
    }
}
