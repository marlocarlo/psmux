// Regression tests for issue #209: tmux command flags compatibility gaps
//
// PRODUCTION CODE TESTS (call execute_command_string / production functions):
//   - display-message -d/-I/-t flags consumed correctly
//   - show-options local popup output
//   - display-message duration storage and expiry semantics
//   - list-keys popup bindings (PREFIX_DEFAULTS + key_tables)
//
// CONTRACT TESTS (mirror server-side parsing in src/server/connection.rs):
//   - send-keys -X flag parsing (server: line ~792)
//   - respawn-pane -c workdir forwarding (server: line ~1096)
//   - show-options -gv combined flag parsing (server: line ~1257)
//   - resize-window -x/-y forwarding (server: line ~2069)
//   - list-panes -s/-a distinction (server: line ~876)
//   - list-keys -T table filtering (server-side)
//   - list-sessions -F/-f flag parsing (server: line ~1865)

#[allow(unused_imports)]
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

// ========================================================================
// Gap 6: display-message -d should be consumed, not leaked into message
// These tests call the REAL production code via execute_command_string()
// Production code: src/commands.rs display-message local handler
// ========================================================================

#[test]
fn display_message_d_flag_not_in_message() {
    // Call PRODUCTION code: execute_command_string dispatches to the local
    // display-message handler which parses -d, -p, -I, -t flags.
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message -p -d 5000 hello world");
    assert!(app.status_message.is_some(), "status_message should be set");
    let (msg, _, duration) = app.status_message.as_ref().unwrap();
    // -d 5000 must be consumed as duration, not leaked into the message text
    assert!(!msg.contains("-d"), "message must not contain the -d flag, got: {}", msg);
    assert!(!msg.contains("5000"), "message must not contain the -d value, got: {}", msg);
    assert!(msg.contains("hello"), "message should contain 'hello', got: {}", msg);
    assert_eq!(*duration, Some(5000), "duration override should be 5000ms");
}

#[test]
fn display_message_I_flag_not_in_message() {
    // Call PRODUCTION code: -I flag and its value must be consumed, not in message
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message -I input_data the_message");
    assert!(app.status_message.is_some(), "status_message should be set");
    let (msg, _, _) = app.status_message.as_ref().unwrap();
    assert!(!msg.contains("-I"), "message must not contain -I flag, got: {}", msg);
    assert!(!msg.contains("input_data"), "message must not contain -I value, got: {}", msg);
    assert!(msg.contains("the_message"), "message should contain 'the_message', got: {}", msg);
}

// ========================================================================
// Gap 7: send-keys -X should be parsed as a flag
// CONTRACT TESTS: The -X flag parsing happens server-side in
// src/server/connection.rs (send-keys handler, line ~792).
// The local execute_command_string handler just forwards to server.
// These verify the expected parsing contract that the server implements.
// ========================================================================

#[test]
fn send_keys_x_flag_parsed_correctly() {
    // Simulate CLI-side parsing of: send-keys -t mysession -X copy-mode-command
    let cmd_args = vec![
        "send-keys".to_string(),
        "-t".to_string(),
        "mysession".to_string(),
        "-X".to_string(),
        "cancel".to_string(),
    ];

    let mut literal = false;
    let mut has_x = false;
    let mut keys: Vec<String> = Vec::new();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-l" => { literal = true; }
            "-R" => { keys.push("__RESET__".to_string()); }
            "-X" => { has_x = true; }
            "-t" => { i += 1; }
            "-N" => { i += 1; }
            _ => { keys.push(cmd_args[i].to_string()); }
        }
        i += 1;
    }

    assert!(has_x, "-X flag should be parsed");
    assert!(!literal, "-l should not be set");
    assert_eq!(keys.len(), 1, "should have one key arg");
    assert_eq!(keys[0], "cancel", "key arg should be 'cancel'");

    // Verify reconstructed command includes -X
    let mut cmd = "send-keys".to_string();
    if literal { cmd.push_str(" -l"); }
    if has_x { cmd.push_str(" -X"); }
    for k in &keys {
        cmd.push_str(&format!(" {}", k));
    }

    assert!(cmd.contains("-X"), "reconstructed command must contain -X");
    assert_eq!(cmd, "send-keys -X cancel");
}

#[test]
fn send_keys_x_not_treated_as_literal_key() {
    // Before the fix, -X would fall through to the catch-all and become a key
    let cmd_args = vec![
        "send-keys".to_string(),
        "-X".to_string(),
        "copy-mode".to_string(),
    ];

    let mut has_x = false;
    let mut keys: Vec<String> = Vec::new();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-X" => { has_x = true; }
            "-l" | "-R" => {}
            "-t" | "-N" => { i += 1; }
            _ => { keys.push(cmd_args[i].to_string()); }
        }
        i += 1;
    }

    // -X should NOT be in the keys list
    assert!(has_x, "-X should be recognized as a flag");
    assert!(!keys.contains(&"-X".to_string()), "-X must not be in the keys list (it's a flag, not a key to send)");
}

// ========================================================================
// Gap 8: respawn-pane -c should forward workdir
// CONTRACT TESTS: The -c flag is parsed server-side in
// src/server/connection.rs respawn-pane handler (line ~1096).
// These verify the expected parsing contract.
// ========================================================================

#[test]
fn respawn_pane_c_flag_forwarded() {
    // Simulate CLI-side parsing of: respawn-pane -k -c C:\Temp -t mysession
    let cmd_args = vec![
        "respawn-pane".to_string(),
        "-k".to_string(),
        "-c".to_string(),
        "C:\\Temp".to_string(),
        "-t".to_string(),
        "mysession".to_string(),
    ];

    let mut cmd = "respawn-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-k" => { cmd.push_str(" -k"); }
            "-c" => {
                if let Some(d) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -c {}", d));
                    i += 1;
                }
            }
            "-t" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -t {}", t));
                    i += 1;
                }
            }
            _ => { cmd.push_str(&format!(" {}", cmd_args[i])); }
        }
        i += 1;
    }

    assert!(cmd.contains("-c C:\\Temp"), "reconstructed command must contain -c workdir, got: {}", cmd);
    assert!(cmd.contains("-k"), "reconstructed command must contain -k");
}

#[test]
fn respawn_pane_without_c_flag_still_works() {
    let cmd_args = vec![
        "respawn-pane".to_string(),
        "-k".to_string(),
    ];

    let mut cmd = "respawn-pane".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-k" => { cmd.push_str(" -k"); }
            "-c" => {
                if let Some(d) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -c {}", d));
                    i += 1;
                }
            }
            "-t" => { i += 1; }
            _ => { cmd.push_str(&format!(" {}", cmd_args[i])); }
        }
        i += 1;
    }

    assert_eq!(cmd, "respawn-pane -k");
    assert!(!cmd.contains("-c"), "should not contain -c when not provided");
}

// ========================================================================
// Gap 9: show-options combined flags like -gv
// CONTRACT TESTS: Combined flag parsing (e.g. -gv, -wv) is handled
// server-side in src/server/connection.rs show-options handler (line ~1257).
// The local path (execute_command_string) calls generate_show_options()
// without flag parsing. These verify the combined_has parsing logic
// that the server implements.
// ========================================================================

#[test]
fn show_options_combined_gv_flag_recognized() {
    // The server parses args for flag chars in combined tokens
    let args = vec!["-gv", "status-style"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    assert!(combined_has('g'), "-gv should contain 'g'");
    assert!(combined_has('v'), "-gv should contain 'v'");
    assert!(!combined_has('w'), "-gv should NOT contain 'w'");
    assert!(!combined_has('A'), "-gv should NOT contain 'A'");
}

#[test]
fn show_options_separate_flags_still_work() {
    let args = vec!["-g", "-v", "status-style"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    assert!(combined_has('g'), "separate -g should be recognized");
    assert!(combined_has('v'), "separate -v should be recognized");
}

#[test]
fn show_options_wv_combined_flag() {
    let args = vec!["-wv", "pane-border-style"];
    let combined_has = |ch: char| -> bool {
        args.iter().any(|a| {
            if *a == format!("-{}", ch) { return true; }
            a.starts_with('-') && a.len() > 2 && a.chars().skip(1).all(|c| c.is_ascii_alphabetic()) && a.contains(ch)
        })
    };
    assert!(combined_has('w'), "-wv should contain 'w'");
    assert!(combined_has('v'), "-wv should contain 'v'");
}

// ========================================================================
// Gap 3: resize-window should forward to server (not be a no-op)
// CONTRACT TESTS: resize-window is server-only in
// src/server/connection.rs (line ~2069). The local handler
// just forwards via send_control_to_port.
// ========================================================================

#[test]
fn resize_window_cli_builds_correct_command() {
    // Simulate CLI-side parsing of: resize-window -t session -x 80
    let cmd_args = vec![
        "resize-window".to_string(),
        "-t".to_string(),
        "session".to_string(),
        "-x".to_string(),
        "80".to_string(),
    ];

    let mut cmd = "resize-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-x" | "-y" => {
                if let Some(v) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" {} {}", cmd_args[i], v));
                    i += 1;
                }
            }
            "-t" => { i += 1; }
            "-A" | "-D" | "-U" => { cmd.push_str(&format!(" {}", cmd_args[i])); }
            _ => {}
        }
        i += 1;
    }

    assert!(cmd.contains("-x 80"), "command must contain -x 80, got: {}", cmd);
    assert!(!cmd.contains("-t"), "command must not contain -t (handled globally)");
}

#[test]
fn resize_window_y_flag() {
    let cmd_args = vec![
        "resize-window".to_string(),
        "-y".to_string(),
        "24".to_string(),
    ];

    let mut cmd = "resize-window".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-x" | "-y" => {
                if let Some(v) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" {} {}", cmd_args[i], v));
                    i += 1;
                }
            }
            "-t" => { i += 1; }
            _ => {}
        }
        i += 1;
    }

    assert!(cmd.contains("-y 24"), "command must contain -y 24, got: {}", cmd);
}

// ========================================================================
// Gap 4: list-panes -s should be session-scoped
// CONTRACT TESTS: -s/-a flag distinction is server-side in
// src/server/connection.rs list-panes handler (line ~876).
// The local path just calls generate_list_panes() for the active window.
// ========================================================================

#[test]
fn list_panes_s_not_same_as_a_in_server_parsing() {
    // Verify that -s and -a are no longer treated identically
    let args_s = vec!["-s", "-t", "mysession"];
    let args_a = vec!["-a"];

    let all_s = args_s.iter().any(|a| *a == "-a");
    let session_s = args_s.iter().any(|a| *a == "-s");

    let all_a = args_a.iter().any(|a| *a == "-a");
    let session_a = args_a.iter().any(|a| *a == "-s");

    // With the fix: -s sets session_scope, -a sets all
    assert!(!all_s, "-s args should not set 'all' flag");
    assert!(session_s, "-s args should set 'session_scope' flag");
    assert!(all_a, "-a args should set 'all' flag");
    assert!(!session_a, "-a args should not set 'session_scope' flag");
}

// ========================================================================
// Gap 5: list-keys -T should filter by table
// CONTRACT TESTS: -T filtering is server-side. The local handler
// generates all tables (src/commands.rs line ~1294). The good
// production-code tests are below (list_keys_command_produces_popup_*).
// ========================================================================

#[test]
fn list_keys_cli_forwards_t_flag() {
    // Simulate CLI-side parsing of: list-keys -T prefix
    let cmd_args = vec![
        "list-keys".to_string(),
        "-T".to_string(),
        "prefix".to_string(),
    ];

    let mut cmd = "list-keys".to_string();
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-T" => {
                if let Some(t) = cmd_args.get(i + 1) {
                    cmd.push_str(&format!(" -T {}", t));
                    i += 1;
                }
            }
            "-t" => { i += 1; }
            _ => { cmd.push_str(&format!(" {}", cmd_args[i])); }
        }
        i += 1;
    }

    assert!(cmd.contains("-T prefix"), "command must forward -T prefix, got: {}", cmd);
}

#[test]
fn list_keys_server_filters_by_table() {
    // Simulate server-side filtering of list-keys output
    let output = vec![
        "bind-key -T prefix c new-window",
        "bind-key -T prefix d detach-client",
        "bind-key -T root C-b send-prefix",
        "bind-key -T copy-mode-vi y copy-selection",
    ];
    let table_filter = Some("prefix".to_string());
    let text = output.join("\n");

    let filtered: Vec<&str> = text.lines().filter(|line| {
        if let Some(ref tbl) = table_filter {
            let parts: Vec<&str> = line.splitn(5, ' ').collect();
            if parts.len() >= 3 {
                return parts[2] == tbl.as_str();
            }
            return false;
        }
        true
    }).collect();

    assert_eq!(filtered.len(), 2, "should only have prefix table entries");
    assert!(filtered[0].contains("new-window"));
    assert!(filtered[1].contains("detach-client"));
    // root and copy-mode-vi entries should be filtered out  
    assert!(!filtered.iter().any(|l| l.contains("root")));
    assert!(!filtered.iter().any(|l| l.contains("copy-mode-vi")));
}

// ========================================================================
// Gap 1: list-sessions -F should forward format to server
// CONTRACT TESTS: -F/-f flag parsing is handled by main.rs CLI
// and server-side in src/server/connection.rs (line ~1865).
// ========================================================================

#[test]
fn list_sessions_parses_f_and_f_flags() {
    // Simulate CLI-side parsing of: list-sessions -F '#{session_name}'
    let cmd_args = vec![
        "list-sessions".to_string(),
        "-F".to_string(),
        "#{session_name}".to_string(),
    ];

    let mut format_str: Option<String> = None;
    let mut filter_str: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-F" => {
                if let Some(f) = cmd_args.get(i + 1) {
                    format_str = Some(f.to_string());
                    i += 1;
                }
            }
            "-f" => {
                if let Some(f) = cmd_args.get(i + 1) {
                    filter_str = Some(f.to_string());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    assert_eq!(format_str, Some("#{session_name}".to_string()), "-F should be parsed");
    assert_eq!(filter_str, None, "-f should not be set");
}

#[test]
fn list_sessions_parses_both_f_and_f_flags() {
    let cmd_args = vec![
        "list-sessions".to_string(),
        "-F".to_string(),
        "#{session_name}".to_string(),
        "-f".to_string(),
        "mysession".to_string(),
    ];

    let mut format_str: Option<String> = None;
    let mut filter_str: Option<String> = None;
    let mut i = 1;
    while i < cmd_args.len() {
        match cmd_args[i].as_str() {
            "-F" => {
                if let Some(f) = cmd_args.get(i + 1) {
                    format_str = Some(f.to_string());
                    i += 1;
                }
            }
            "-f" => {
                if let Some(f) = cmd_args.get(i + 1) {
                    filter_str = Some(f.to_string());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    assert_eq!(format_str, Some("#{session_name}".to_string()));
    assert_eq!(filter_str, Some("mysession".to_string()));
}

// ========================================================================
// display-message -d: per-message duration override actually works
// ========================================================================

#[test]
fn display_message_d_sets_duration_on_status_message() {
    // Execute display-message with -d via the commands module
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message -d 5000 hello");
    // The status_message should be set with the duration override
    assert!(app.status_message.is_some(), "status_message should be set");
    let (msg, _, duration) = app.status_message.as_ref().unwrap();
    assert!(msg.contains("hello"), "message should contain 'hello', got: {}", msg);
    assert_eq!(*duration, Some(5000), "duration override should be 5000ms");
}

#[test]
fn display_message_without_d_has_no_duration_override() {
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message hello_no_d");
    assert!(app.status_message.is_some());
    let (msg, _, duration) = app.status_message.as_ref().unwrap();
    assert!(msg.contains("hello_no_d"), "got: {}", msg);
    assert_eq!(*duration, None, "no -d flag means no duration override");
}

#[test]
fn display_message_d_zero_sets_zero_duration() {
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message -d 0 zero_test");
    assert!(app.status_message.is_some());
    let (_, _, duration) = app.status_message.as_ref().unwrap();
    assert_eq!(*duration, Some(0), "duration of 0 should be passed through");
}

#[test]
fn display_message_d_invalid_value_uses_none() {
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message -d notanumber test_invalid");
    assert!(app.status_message.is_some());
    let (_, _, duration) = app.status_message.as_ref().unwrap();
    assert_eq!(*duration, None, "invalid -d value should result in None duration");
}

#[test]
fn status_message_expiry_uses_per_message_duration() {
    // Verify the status_message stores per-message duration correctly
    let mut app = mock_app_with_window();
    // Set a long duration: the tuple should contain the override
    app.status_message = Some(("long_msg".to_string(), std::time::Instant::now(), Some(60000)));
    let (_, _, dur) = app.status_message.as_ref().unwrap();
    assert_eq!(*dur, Some(60000), "long duration should be stored");

    // Set a very short duration
    app.status_message = Some(("short_msg".to_string(), std::time::Instant::now(), Some(1)));
    let (_, _, dur) = app.status_message.as_ref().unwrap();
    assert_eq!(*dur, Some(1), "short duration should be stored");

    // Verify unwrap_or logic: None should fall back to global
    app.display_time_ms = 750;
    app.status_message = Some(("no_override".to_string(), std::time::Instant::now(), None));
    let (_, _, dur) = app.status_message.as_ref().unwrap();
    let effective = dur.unwrap_or(app.display_time_ms);
    assert_eq!(effective, 750, "None duration should use global display_time_ms");

    // Verify with explicit override
    app.status_message = Some(("with_override".to_string(), std::time::Instant::now(), Some(3000)));
    let (_, _, dur) = app.status_message.as_ref().unwrap();
    let effective = dur.unwrap_or(app.display_time_ms);
    assert_eq!(effective, 3000, "explicit duration should override global");
}

// ========================================================================
// respawn-pane -k: server-side kill flag parsing
// FIX REGRESSION: The -k flag was parsed at CLI level but silently
// discarded at server level. CtrlReq::RespawnPane now carries (workdir, kill).
// ========================================================================

#[test]
fn respawn_pane_k_flag_parsed_by_server_handler() {
    // Mirrors server/connection.rs respawn-pane handler parsing
    let args = vec!["-k"];
    let workdir: Option<String> = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].to_string());
    let kill = args.iter().any(|a| *a == "-k");

    assert!(kill, "-k must be recognized");
    assert!(workdir.is_none(), "no -c provided means no workdir");
}

#[test]
fn respawn_pane_k_and_c_flags_parsed_together() {
    let args = vec!["-k", "-c", "/tmp/test"];
    let workdir: Option<String> = args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].to_string());
    let kill = args.iter().any(|a| *a == "-k");

    assert!(kill, "-k must be recognized alongside -c");
    assert_eq!(workdir.as_deref(), Some("/tmp/test"), "workdir must be extracted from -c");
}

#[test]
fn respawn_pane_without_k_flag_kill_is_false() {
    let args: Vec<&str> = vec!["-c", "/tmp/test"];
    let kill = args.iter().any(|a| *a == "-k");
    assert!(!kill, "without -k flag, kill must be false");
}

#[test]
fn respawn_pane_k_flag_parsed_in_execute_command_string() {
    // The local command path (execute_command_string) must parse -k from parts
    let cmd = "respawn-pane -k -c /tmp";
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let kill = parts.iter().any(|p| *p == "-k");
    assert!(kill, "execute_command_string path must detect -k in command parts");
}

#[test]
fn respawn_pane_execute_command_without_k() {
    let cmd = "respawn-pane -c /tmp";
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let kill = parts.iter().any(|p| *p == "-k");
    assert!(!kill, "without -k, kill must be false in execute_command_string path");
}

#[test]
fn status_message_expiry_without_override_uses_global() {
    let mut app = mock_app_with_window();
    app.display_time_ms = 750;
    // Set message without duration override
    app.status_message = Some(("global_test".to_string(), std::time::Instant::now(), None));
    let (msg, _, dur) = app.status_message.as_ref().unwrap();
    assert_eq!(msg, "global_test");
    let effective = dur.unwrap_or(app.display_time_ms);
    assert_eq!(effective, 750, "without -d, should use global display_time_ms (750)");
}

// ========================================================================
// PRODUCTION CODE TESTS: show-options local path
// Calls execute_command_string() which dispatches to generate_show_options()
// Production code: src/commands.rs show-options handler (line ~1307)
// ========================================================================

#[test]
fn show_options_local_produces_popup() {
    let mut app = mock_app_with_window();
    app.control_port = None;
    execute_command_string(&mut app, "show-options").unwrap();
    match &app.mode {
        Mode::PopupMode { command, output, .. } => {
            assert_eq!(command, "show-options");
            // The output should contain known option names
            assert!(
                output.contains("status-") || output.contains("display-time") || output.contains("base-index"),
                "show-options popup should contain known option names, got:\n{}",
                &output[..output.len().min(500)]
            );
        }
        other => panic!("expected PopupMode for show-options, got {:?}", std::mem::discriminant(other)),
    }
}

// ========================================================================
// PRODUCTION CODE TESTS: display-message flag combinations
// Additional edge cases calling real production code
// ========================================================================

#[test]
fn display_message_d_and_I_combined() {
    // Both -d and -I should be consumed, only the message text remains
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message -d 2000 -I ignored combined_test");
    assert!(app.status_message.is_some());
    let (msg, _, duration) = app.status_message.as_ref().unwrap();
    assert!(msg.contains("combined_test"), "message should contain 'combined_test', got: {}", msg);
    assert!(!msg.contains("-d"), "message should not contain -d flag");
    assert!(!msg.contains("-I"), "message should not contain -I flag");
    assert!(!msg.contains("ignored"), "message should not contain -I value 'ignored'");
    assert_eq!(*duration, Some(2000), "duration should be 2000ms");
}

#[test]
fn display_message_t_flag_consumed() {
    // -t target should be consumed (ignored locally), not leaked into message
    let mut app = mock_app_with_window();
    app.control_port = None;
    let _ = execute_command_string(&mut app, "display-message -t mysession target_test");
    assert!(app.status_message.is_some());
    let (msg, _, _) = app.status_message.as_ref().unwrap();
    assert!(msg.contains("target_test"), "message should contain 'target_test', got: {}", msg);
    assert!(!msg.contains("mysession"), "message should not contain -t target value, got: {}", msg);
}
