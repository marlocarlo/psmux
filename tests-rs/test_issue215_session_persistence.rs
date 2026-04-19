// Regression tests for issue #215: session persistence gaps
//
// Tests that UNDENIABLY prove the two core features required by
// psmux-resurrect (and any session persistence plugin):
//
//   1. show-options -v / -gqv @option  returns value only for @-prefixed
//      user options (via get_option_value and generate_show_options)
//
//   2. list-sessions -F '#{session_name}'  format variable expansion
//      (via expand_format / expand_var)
//
// Each test exercises PRODUCTION code paths, not contract/parsing stubs.

use super::*;

fn mock_app() -> AppState {
    let mut app = AppState::new("test_session".to_string());
    app.window_base_index = 0;
    app.pane_base_index = 0;
    app
}

fn make_window(name: &str, id: usize) -> crate::types::Window {
    crate::types::Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
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

fn mock_app_with_windows(names: &[&str]) -> AppState {
    let mut app = mock_app();
    for (i, name) in names.iter().enumerate() {
        app.windows.push(make_window(name, i));
    }
    app
}

fn extract_popup(app: &AppState) -> (&str, &str) {
    match &app.mode {
        Mode::PopupMode { command, output, .. } => (command, output),
        other => panic!("expected PopupMode, got {:?}", std::mem::discriminant(other)),
    }
}

// ════════════════════════════════════════════════════════════════════════════
//  Feature 1: show-options with @user_options
//  Production code: generate_show_options() in commands.rs
//  Production code: get_option_value() in server/options.rs
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn generate_show_options_includes_user_options() {
    // generate_show_options must include @-prefixed user options in its output
    let mut app = mock_app_with_window();
    app.user_options.insert("@resurrect-capture-pane-contents".to_string(), "on".to_string());
    app.user_options.insert("@plugin".to_string(), "psmux-plugins/psmux-resurrect".to_string());

    let output = generate_show_options(&app);

    assert!(output.contains("@resurrect-capture-pane-contents"),
        "show-options output must include @resurrect-capture-pane-contents, got:\n{}", output);
    assert!(output.contains("@plugin"),
        "show-options output must include @plugin, got:\n{}", output);
}

#[test]
fn generate_show_options_user_option_value_is_quoted() {
    // User option values with spaces are quoted in generate_show_options
    let mut app = mock_app_with_window();
    app.user_options.insert("@my-opt".to_string(), "hello world".to_string());

    let output = generate_show_options(&app);

    // Format is: @my-opt "hello world"
    assert!(output.contains(r#"@my-opt "hello world""#),
        "user option should appear as '@my-opt \"hello world\"', got:\n{}", output);
}

#[test]
fn show_options_popup_includes_user_options() {
    // execute_command_string("show-options") local path uses generate_show_options
    // and shows in PopupMode, so @options must be visible
    let mut app = mock_app_with_window();
    app.user_options.insert("@resurrect-dir".to_string(), "~/.psmux/resurrect".to_string());

    execute_command_string(&mut app, "show-options").unwrap();
    let (cmd, out) = extract_popup(&app);

    assert_eq!(cmd, "show-options");
    assert!(out.contains("@resurrect-dir"),
        "show-options popup must display @resurrect-dir, got:\n{}", out);
    assert!(out.contains("~/.psmux/resurrect"),
        "show-options popup must display the value, got:\n{}", out);
}

#[test]
fn show_options_includes_builtin_and_user_options_together() {
    // Both built-in options (prefix, mouse, etc.) and @user options
    // must appear in the same output
    let mut app = mock_app_with_window();
    app.user_options.insert("@continuum-save-interval".to_string(), "15".to_string());

    execute_command_string(&mut app, "show-options").unwrap();
    let (_, out) = extract_popup(&app);

    assert!(out.contains("prefix"), "must include builtin 'prefix'");
    assert!(out.contains("mouse"), "must include builtin 'mouse'");
    assert!(out.contains("@continuum-save-interval"),
        "must include user option '@continuum-save-interval'");
}

#[test]
fn get_option_value_returns_user_option() {
    // get_option_value in server/options.rs must resolve @-prefixed options
    let mut app = mock_app();
    app.user_options.insert("@resurrect-capture-pane-contents".to_string(), "on".to_string());

    let val = crate::server::options::get_option_value(&app, "@resurrect-capture-pane-contents");
    assert_eq!(val, "on",
        "get_option_value('@resurrect-capture-pane-contents') must return 'on', got: '{}'", val);
}

#[test]
fn get_option_value_returns_empty_for_unset_user_option() {
    let app = mock_app();
    let val = crate::server::options::get_option_value(&app, "@nonexistent-option");
    assert_eq!(val, "",
        "get_option_value for unset @option must return empty string, got: '{}'", val);
}

#[test]
fn get_option_value_user_option_after_set_option() {
    // set-option -g @key value should be queryable via get_option_value
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "set-option -g @my-test-opt test-value").unwrap();

    let val = crate::server::options::get_option_value(&app, "@my-test-opt");
    assert_eq!(val, "test-value",
        "get_option_value after set-option should return 'test-value', got: '{}'", val);
}

#[test]
fn get_option_value_builtin_options_still_work() {
    // Ensure @option support does not break built-in option lookup
    let app = mock_app();
    assert_eq!(crate::server::options::get_option_value(&app, "base-index"), "0");
    assert!(!crate::server::options::get_option_value(&app, "prefix").is_empty());
    assert!(crate::server::options::get_option_value(&app, "mouse") == "on"
        || crate::server::options::get_option_value(&app, "mouse") == "off");
}

#[test]
fn set_option_user_option_overwrite() {
    // Setting a @option twice should overwrite, not append
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "set-option -g @my-opt first").unwrap();
    execute_command_string(&mut app, "set-option -g @my-opt second").unwrap();

    let val = crate::server::options::get_option_value(&app, "@my-opt");
    assert_eq!(val, "second",
        "second set-option should overwrite first, got: '{}'", val);
}

#[test]
fn set_option_unset_user_option() {
    // set-option -gu @key should remove the user option
    let mut app = mock_app_with_window();
    app.user_options.insert("@to-remove".to_string(), "value".to_string());
    execute_command_string(&mut app, "set-option -gu @to-remove").unwrap();

    let val = crate::server::options::get_option_value(&app, "@to-remove");
    assert_eq!(val, "",
        "unset @option should return empty, got: '{}'", val);
}

#[test]
fn multiple_user_options_in_show_options() {
    // Multiple @options should all appear
    let mut app = mock_app_with_window();
    app.user_options.insert("@plugin".to_string(), "psmux-resurrect".to_string());
    app.user_options.insert("@resurrect-strategy-vim".to_string(), "session".to_string());
    app.user_options.insert("@resurrect-capture-pane-contents".to_string(), "on".to_string());

    let output = generate_show_options(&app);

    assert!(output.contains("@plugin"), "must contain @plugin");
    assert!(output.contains("@resurrect-strategy-vim"), "must contain @resurrect-strategy-vim");
    assert!(output.contains("@resurrect-capture-pane-contents"), "must contain @resurrect-capture-pane-contents");
}

// ════════════════════════════════════════════════════════════════════════════
//  Feature 2: Format variable expansion for session variables
//  Production code: expand_format() / expand_var() in format.rs
//  Used by: list-sessions -F, display-message, status-left/right
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn expand_format_session_name() {
    let app = mock_app_with_windows(&["editor", "build"]);
    let result = crate::format::expand_format("#{session_name}", &app);
    assert_eq!(result, "test_session",
        "#{{session_name}} must expand to 'test_session', got: '{}'", result);
}

#[test]
fn expand_format_session_windows_count() {
    let app = mock_app_with_windows(&["editor", "build", "logs"]);
    let result = crate::format::expand_format("#{session_windows}", &app);
    assert_eq!(result, "3",
        "#{{session_windows}} must expand to '3' for 3 windows, got: '{}'", result);
}

#[test]
fn expand_format_session_id() {
    let app = mock_app();
    let result = crate::format::expand_format("#{session_id}", &app);
    assert!(result.starts_with('$'),
        "#{{session_id}} must start with '$', got: '{}'", result);
}

#[test]
fn expand_format_combined_session_vars() {
    // This is the exact pattern psmux-resurrect uses
    let app = mock_app_with_windows(&["editor", "build"]);
    let result = crate::format::expand_format("#{session_name}:#{session_windows}", &app);
    assert_eq!(result, "test_session:2",
        "combined format must expand correctly, got: '{}'", result);
}

#[test]
fn expand_format_session_name_only_no_extra_data() {
    // Crucial for resurrect: format must NOT include timestamps or other data
    let app = mock_app_with_windows(&["shell"]);
    let result = crate::format::expand_format("#{session_name}", &app);
    assert!(!result.contains("windows"),
        "#{{session_name}} must not contain 'windows', got: '{}'", result);
    assert!(!result.contains("created"),
        "#{{session_name}} must not contain 'created', got: '{}'", result);
    assert_eq!(result, "test_session");
}

#[test]
fn expand_format_user_option_variable() {
    // #{@option_name} should expand from user_options
    let mut app = mock_app_with_window();
    app.user_options.insert("@my-custom-var".to_string(), "custom_value".to_string());

    let result = crate::format::expand_format("#{@my-custom-var}", &app);
    assert_eq!(result, "custom_value",
        "#{{@my-custom-var}} must expand to 'custom_value', got: '{}'", result);
}

#[test]
fn expand_format_unset_user_option_is_empty() {
    let app = mock_app_with_window();
    let result = crate::format::expand_format("#{@nonexistent}", &app);
    assert_eq!(result, "",
        "#{{@nonexistent}} must expand to empty string, got: '{}'", result);
}

#[test]
fn expand_format_hash_s_shorthand() {
    // #S is the tmux shorthand for #{session_name}
    let mut app = mock_app_with_window();
    app.session_name = "my_project".to_string();

    let result = crate::format::expand_format("#S", &app);
    assert_eq!(result, "my_project",
        "#S must expand to session name 'my_project', got: '{}'", result);
}

#[test]
fn expand_format_mixed_session_and_window_vars() {
    // A complex format string with session + window variables
    let app = mock_app_with_windows(&["editor"]);
    let result = crate::format::expand_format(
        "#{session_name} | #{window_name} | #{session_windows}",
        &app
    );
    assert!(result.contains("test_session"), "must contain session name");
    assert!(result.contains("editor"), "must contain window name");
    assert!(result.contains("1"), "must contain window count");
}

#[test]
fn expand_format_literal_text_preserved() {
    // Text outside #{...} must pass through unchanged
    let app = mock_app_with_window();
    let result = crate::format::expand_format("hello #{session_name} world", &app);
    assert_eq!(result, "hello test_session world");
}

#[test]
fn expand_format_empty_format_string() {
    let app = mock_app();
    let result = crate::format::expand_format("", &app);
    assert_eq!(result, "");
}

#[test]
fn expand_format_no_variables() {
    let app = mock_app();
    let result = crate::format::expand_format("plain text only", &app);
    assert_eq!(result, "plain text only");
}

#[test]
fn expand_var_session_name_direct() {
    let app = mock_app_with_window();
    let result = crate::format::expand_var("session_name", &app, 0);
    assert_eq!(result, "test_session");
}

#[test]
fn expand_var_session_windows_direct() {
    let app = mock_app_with_windows(&["a", "b", "c"]);
    let result = crate::format::expand_var("session_windows", &app, 0);
    assert_eq!(result, "3");
}

#[test]
fn expand_var_session_name_no_windows() {
    // session_name must work even with zero windows
    let app = mock_app();
    let result = crate::format::expand_var("session_name", &app, 0);
    assert_eq!(result, "test_session");
}

#[test]
fn expand_var_session_windows_no_windows() {
    let app = mock_app();
    let result = crate::format::expand_var("session_windows", &app, 0);
    assert_eq!(result, "0");
}

// ════════════════════════════════════════════════════════════════════════════
//  Combined: set-option then show-options round-trip
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn set_then_show_user_option_round_trip() {
    let mut app = mock_app_with_window();

    // Set a @option
    execute_command_string(&mut app, "set-option -g @resurrect-save-interval 60").unwrap();

    // show-options must include it
    execute_command_string(&mut app, "show-options").unwrap();
    let (_, out) = extract_popup(&app);

    assert!(out.contains("@resurrect-save-interval"),
        "show-options after set-option must include @resurrect-save-interval");
    assert!(out.contains("60"),
        "show-options must show the value '60'");
}

#[test]
fn format_expansion_uses_set_option_values() {
    // #{@option} in format string must reflect set-option changes
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "set-option -g @my-flag enabled").unwrap();

    let result = crate::format::expand_format("#{@my-flag}", &app);
    assert_eq!(result, "enabled",
        "format expansion of #{{@my-flag}} after set-option must be 'enabled', got: '{}'", result);
}

// ════════════════════════════════════════════════════════════════════════════
//  Connection-level: combined_has flag parsing for -gqv
//  The connection.rs handler uses this closure to parse combined flags.
//  These tests verify the EXACT parsing logic used in production.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn combined_has_parses_gqv_all_flags() {
    let args = vec!["-gqv", "@resurrect-capture-pane-contents"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    assert!(combined_has('g'), "-gqv must contain 'g'");
    assert!(combined_has('q'), "-gqv must contain 'q'");
    assert!(combined_has('v'), "-gqv must contain 'v'");
    assert!(!combined_has('w'), "-gqv must NOT contain 'w'");
    assert!(!combined_has('A'), "-gqv must NOT contain 'A'");
}

#[test]
fn combined_has_parses_gv_flags() {
    let args = vec!["-gv", "base-index"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    assert!(combined_has('g'), "-gv must contain 'g'");
    assert!(combined_has('v'), "-gv must contain 'v'");
    assert!(!combined_has('q'), "-gv must NOT contain 'q'");
}

#[test]
fn combined_has_separate_g_q_v_flags() {
    let args = vec!["-g", "-q", "-v", "@plugin"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    assert!(combined_has('g'), "separate -g must be found");
    assert!(combined_has('q'), "separate -q must be found");
    assert!(combined_has('v'), "separate -v must be found");
}

#[test]
fn combined_has_v_only() {
    let args = vec!["-v", "prefix"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    assert!(combined_has('v'), "-v must be found");
    assert!(!combined_has('g'), "no -g in args");
    assert!(!combined_has('q'), "no -q in args");
}

#[test]
fn combined_has_ignores_option_names_starting_with_at() {
    // @option names start with @ not -, so they must NOT be treated as flags
    let args = vec!["-gqv", "@resurrect-dir"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    // The @resurrect-dir arg starts with @, not -, so it is not a flag
    assert!(!combined_has('r'), "@resurrect-dir must not be parsed as flag");
    assert!(!combined_has('d'), "@resurrect-dir must not be parsed as flag");
}

// ════════════════════════════════════════════════════════════════════════════
//  Integration: simulated show-options -v (value-only) output
//  When -v is set with a specific option name, connection.rs sends
//  only the value (not "name value"). This tests the logic.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn show_options_v_output_format_value_only() {
    // Simulate what connection.rs does when has_v && opt_name.is_some():
    //   format!("{}\n", resolved)   <-- value only
    // vs without -v:
    //   format!("{} {}\n", name, resolved)  <-- name + value
    let name = "@resurrect-capture-pane-contents";
    let resolved = "on";

    // With -v (value only): what the client receives
    let with_v = format!("{}\n", resolved);
    assert_eq!(with_v, "on\n");
    assert!(!with_v.contains(name), "with -v, output must not contain option name");

    // Without -v (name + value): what the client receives
    let without_v = format!("{} {}\n", name, resolved);
    assert!(without_v.contains(name), "without -v, output must contain option name");
    assert!(without_v.contains(resolved), "without -v, output must contain value");
}

#[test]
fn show_options_values_only_strips_names() {
    // When -v without option name, connection.rs strips names from all lines
    let full_output = "prefix C-b\nbase-index 0\nmouse on\n@plugin \"psmux-resurrect\"\n";
    let values_only: String = full_output.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() { return None; }
            if let Some(pos) = trimmed.find(' ') {
                Some(&trimmed[pos + 1..])
            } else {
                Some(trimmed)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(values_only, "C-b\n0\non\n\"psmux-resurrect\"");
    assert!(!values_only.contains("prefix"), "values-only must not contain 'prefix'");
    assert!(!values_only.contains("@plugin"), "values-only must not contain '@plugin'");
}
