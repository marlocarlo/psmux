use super::*;

fn mock_app() -> AppState {
    let mut app = AppState::new("test_session".to_string());
    app.window_base_index = 0;
    app
}

#[test]
fn test_literal_modifier() {
    let app = mock_app();
    assert_eq!(expand_expression("l:hello", &app, 0), "hello");
}

#[test]
fn test_trim_modifier() {
    let app = mock_app();
    let result = expand_expression("=3:session_name", &app, 0);
    assert_eq!(result, "tes");
}

#[test]
fn test_trim_negative() {
    let app = mock_app();
    let result = expand_expression("=-3:session_name", &app, 0);
    assert_eq!(result, "ion");
}

#[test]
fn test_basename() {
    let app = mock_app();
    let val = apply_modifier(&Modifier::Basename, "/usr/src/tmux", &app, 0);
    assert_eq!(val, "tmux");
}

#[test]
fn test_dirname() {
    let app = mock_app();
    let val = apply_modifier(&Modifier::Dirname, "/usr/src/tmux", &app, 0);
    assert_eq!(val, "/usr/src");
}

#[test]
fn test_pad() {
    let app = mock_app();
    let val = apply_modifier(&Modifier::Pad(10), "foo", &app, 0);
    assert_eq!(val, "foo       ");
    let val = apply_modifier(&Modifier::Pad(-10), "foo", &app, 0);
    assert_eq!(val, "       foo");
}

#[test]
fn test_substitute() {
    let app = mock_app();
    let val = apply_modifier(
        &Modifier::Substitute { pattern: "foo".into(), replacement: "bar".into(), case_insensitive: false },
        "foobar", &app, 0
    );
    assert_eq!(val, "barbar");
}

#[test]
fn test_math_add() {
    let app = mock_app();
    let val = apply_modifier(
        &Modifier::MathExpr { op: '+', floating: false, decimals: 0 },
        "3,5", &app, 0
    );
    assert_eq!(val, "8");
}

#[test]
fn test_math_float_div() {
    let app = mock_app();
    let val = apply_modifier(
        &Modifier::MathExpr { op: '/', floating: true, decimals: 4 },
        "10,3", &app, 0
    );
    assert_eq!(val, "3.3333");
}

#[test]
fn test_boolean_or() {
    let app = mock_app();
    assert_eq!(expand_expression("||:1,0", &app, 0), "1");
    assert_eq!(expand_expression("||:0,0", &app, 0), "0");
}

#[test]
fn test_boolean_and() {
    let app = mock_app();
    assert_eq!(expand_expression("&&:1,1", &app, 0), "1");
    assert_eq!(expand_expression("&&:1,0", &app, 0), "0");
}

#[test]
fn test_comparison_eq() {
    let app = mock_app();
    assert_eq!(expand_expression("==:version,version", &app, 0), "1");
}

#[test]
fn test_glob_match_fn() {
    assert!(glob_match("*foo*", "barfoobar", false));
    assert!(!glob_match("*foo*", "barbaz", false));
    assert!(glob_match("*FOO*", "barfoobar", true));
}

#[test]
fn test_quote() {
    let app = mock_app();
    let val = apply_modifier(&Modifier::Quote, "(hello)", &app, 0);
    assert_eq!(val, "\\(hello\\)");
}

// ── Window flags tests ─────────────────────────────────────────

fn mock_window(name: &str) -> crate::types::Window {
    crate::types::Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: name.to_string(),
        id: 0,
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: std::time::Instant::now(),
        last_seen_version: 0,
        manual_rename: false,
        layout_index: 0,
        pane_mru: vec![],
        zoom_saved: None,
    }
}

#[test]
fn test_window_flags_active() {
    let mut app = mock_app();
    app.windows.push(mock_window("win0"));
    app.active_idx = 0;
    assert_eq!(expand_var("window_flags", &app, 0), "*");
}

#[test]
fn test_window_flags_last() {
    let mut app = mock_app();
    app.windows.push(mock_window("win0"));
    app.windows.push(mock_window("win1"));
    app.active_idx = 1;
    app.last_window_idx = 0;
    assert_eq!(expand_var("window_flags", &app, 0), "-");
}

#[test]
fn test_window_flags_bell() {
    let mut app = mock_app();
    let mut win = mock_window("win0");
    win.bell_flag = true;
    app.windows.push(win);
    app.windows.push(mock_window("win1"));
    app.active_idx = 1;
    app.last_window_idx = 1; // same as active so "-" won't appear
    assert_eq!(expand_var("window_flags", &app, 0), "!");
}

#[test]
fn test_window_flags_silence() {
    let mut app = mock_app();
    let mut win = mock_window("win0");
    win.silence_flag = true;
    app.windows.push(win);
    app.windows.push(mock_window("win1"));
    app.active_idx = 1;
    app.last_window_idx = 1;
    assert_eq!(expand_var("window_flags", &app, 0), "~");
}

#[test]
fn test_window_flags_activity() {
    let mut app = mock_app();
    let mut win = mock_window("win0");
    win.activity_flag = true;
    app.windows.push(win);
    app.windows.push(mock_window("win1"));
    app.active_idx = 1;
    app.last_window_idx = 1;
    assert_eq!(expand_var("window_flags", &app, 0), "#");
}

#[test]
fn test_window_flags_bell_and_activity() {
    let mut app = mock_app();
    let mut win = mock_window("win0");
    win.bell_flag = true;
    win.activity_flag = true;
    app.windows.push(win);
    app.windows.push(mock_window("win1"));
    app.active_idx = 1;
    app.last_window_idx = 1;
    assert_eq!(expand_var("window_flags", &app, 0), "#!");
}

#[test]
fn test_window_activity_flag_var() {
    let mut app = mock_app();
    let mut win = mock_window("win0");
    win.activity_flag = true;
    app.windows.push(win);
    assert_eq!(expand_var("window_activity_flag", &app, 0), "1");
}

#[test]
fn test_window_activity_flag_var_off() {
    let mut app = mock_app();
    app.windows.push(mock_window("win0"));
    assert_eq!(expand_var("window_activity_flag", &app, 0), "0");
}

// ── AppState defaults tests ─────────────────────────────────────

#[test]
fn test_appstate_defaults_allow_rename() {
    let app = mock_app();
    assert!(app.allow_rename);
}

// ── Per-window zoom flag tests (issue #125 follow-up) ──────────────

#[test]
fn test_window_zoomed_flag_default_no_zoom() {
    let mut app = mock_app();
    app.windows.push(mock_window("win0"));
    assert_eq!(expand_var("window_zoomed_flag", &app, 0), "0");
}

#[test]
fn test_window_zoomed_flag_set_on_zoomed_window() {
    let mut app = mock_app();
    let mut win = mock_window("win0");
    win.zoom_saved = Some(vec![(vec![], vec![50, 50])]);
    app.windows.push(win);
    app.active_idx = 0;
    // The zoomed window should report flag=1
    assert_eq!(expand_var("window_zoomed_flag", &app, 0), "1");
}

#[test]
fn test_window_zoomed_flag_per_window_not_global() {
    // Simulates: zoom in window 0, check that window 1 does NOT show zoomed
    let mut app = mock_app();
    let mut win0 = mock_window("win0");
    win0.zoom_saved = Some(vec![(vec![], vec![50, 50])]);
    app.windows.push(win0);
    app.windows.push(mock_window("win1"));
    app.active_idx = 0;
    // Window 0 is zoomed → flag=1
    assert_eq!(expand_var("window_zoomed_flag", &app, 0), "1");
    // Window 1 is NOT zoomed → flag=0
    assert_eq!(expand_var("window_zoomed_flag", &app, 1), "0");
}

#[test]
fn test_window_zoomed_flag_stays_on_original_window_after_switch() {
    // Simulates: zoom in window 0, then switch to window 1
    // Window 0 should still show zoomed, window 1 should not
    let mut app = mock_app();
    let mut win0 = mock_window("win0");
    win0.zoom_saved = Some(vec![(vec![], vec![50, 50])]);
    app.windows.push(win0);
    app.windows.push(mock_window("win1"));
    // Switch to window 1
    app.active_idx = 1;
    // Window 0 still zoomed → flag=1 (even though it's not the active window)
    assert_eq!(expand_var("window_zoomed_flag", &app, 0), "1");
    // Window 1 is NOT zoomed → flag=0
    assert_eq!(expand_var("window_zoomed_flag", &app, 1), "0");
}

#[test]
fn test_window_zoomed_flag_multiple_windows_zoomed() {
    // In tmux, multiple windows can each have a zoomed pane independently
    let mut app = mock_app();
    let mut win0 = mock_window("win0");
    win0.zoom_saved = Some(vec![(vec![], vec![50, 50])]);
    let mut win1 = mock_window("win1");
    win1.zoom_saved = Some(vec![(vec![0], vec![30, 70])]);
    app.windows.push(win0);
    app.windows.push(win1);
    app.windows.push(mock_window("win2"));
    app.active_idx = 2;
    // Both window 0 and 1 are zoomed
    assert_eq!(expand_var("window_zoomed_flag", &app, 0), "1");
    assert_eq!(expand_var("window_zoomed_flag", &app, 1), "1");
    // Window 2 is not zoomed
    assert_eq!(expand_var("window_zoomed_flag", &app, 2), "0");
}

#[test]
fn test_window_flags_include_z_when_zoomed() {
    let mut app = mock_app();
    let mut win = mock_window("win0");
    win.zoom_saved = Some(vec![(vec![], vec![50, 50])]);
    app.windows.push(win);
    app.active_idx = 0;
    let flags = expand_var("window_flags", &app, 0);
    assert!(flags.contains('Z'), "window_flags should contain Z when zoomed, got: {}", flags);
    assert!(flags.contains('*'), "window_flags should contain * for active window, got: {}", flags);
}

#[test]
fn test_window_flags_no_z_when_not_zoomed() {
    let mut app = mock_app();
    app.windows.push(mock_window("win0"));
    app.active_idx = 0;
    let flags = expand_var("window_flags", &app, 0);
    assert!(!flags.contains('Z'), "window_flags should not contain Z when not zoomed, got: {}", flags);
}

#[test]
fn test_conditional_window_zoomed_flag_per_window() {
    let mut app = mock_app();
    let mut win0 = mock_window("win0");
    win0.zoom_saved = Some(vec![(vec![], vec![50, 50])]);
    app.windows.push(win0);
    app.windows.push(mock_window("win1"));
    app.active_idx = 1; // active is window 1, but window 0 is zoomed
    // Conditional format should show ZOOMED for window 0
    let result0 = expand_format_for_window("#{?window_zoomed_flag,ZOOMED,normal}", &app, 0);
    assert_eq!(result0, "ZOOMED");
    // Conditional format should show normal for window 1
    let result1 = expand_format_for_window("#{?window_zoomed_flag,ZOOMED,normal}", &app, 1);
    assert_eq!(result1, "normal");
}

#[test]
fn test_appstate_defaults_bell_action() {
    let app = mock_app();
    assert_eq!(app.bell_action, "any");
}

#[test]
fn test_appstate_defaults_activity_action() {
    let app = mock_app();
    assert_eq!(app.activity_action, "other");
}

#[test]
fn test_appstate_defaults_silence_action() {
    let app = mock_app();
    assert_eq!(app.silence_action, "other");
}

#[test]
fn test_appstate_defaults_monitor_silence() {
    let app = mock_app();
    assert_eq!(app.monitor_silence, 0);
}

#[test]
fn test_appstate_defaults_update_environment() {
    let app = mock_app();
    assert!(app.update_environment.contains(&"DISPLAY".to_string()));
    assert!(app.update_environment.contains(&"SSH_AUTH_SOCK".to_string()));
    assert!(app.update_environment.contains(&"SSH_AGENT_PID".to_string()));
}
