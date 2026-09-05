// kill-window with an unresolvable -t target must kill NOTHING and surface
// "can't find window: X" (tmux parity, verified against live tmux 3.4).
//
// The bug: the target was applied by a temporary-focus step that silently
// no-opped on a bad name/index/@id, and the following untargeted kill then
// removed whatever window was active — ending the session outright when that
// was the last window. These tests pin the command-dispatch path
// (execute_command_string), which hooks and the command prompt share.

use super::*;

fn make_window(name: &str, id: usize) -> crate::types::Window {
    crate::types::Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: name.to_string(),
        id,
        area: ratatui::layout::Rect::new(0, 0, 120, 30),
        window_size: None,
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
        floating: Vec::new(),
        floating_focus: None,
    }
}

fn app_with_windows(names: &[&str]) -> AppState {
    let mut app = AppState::new("kwtest".to_string());
    app.window_base_index = 0;
    for (i, name) in names.iter().enumerate() {
        app.windows.push(make_window(name, i));
    }
    app.window_indices = (0..names.len()).collect();
    app
}

fn window_names(app: &AppState) -> Vec<String> {
    app.windows.iter().map(|w| w.name.clone()).collect()
}

#[test]
fn bad_name_target_kills_nothing_and_reports() {
    let mut app = app_with_windows(&["alpha", "beta", "gamma"]);
    app.active_idx = 1;

    execute_command_string(&mut app, "kill-window -t kwtest:definitelynotawindow").unwrap();

    assert_eq!(
        window_names(&app),
        vec!["alpha", "beta", "gamma"],
        "BUG: a bad NAME target must not remove any window"
    );
    assert_eq!(app.active_idx, 1, "focus must not move on a failed kill");
    let (msg, _, _) = app.status_message.as_ref().expect("an error must be surfaced");
    assert!(
        msg.contains("can't find window"),
        "tmux-parity error expected, got: {}",
        msg
    );
}

#[test]
fn bad_index_target_kills_nothing() {
    let mut app = app_with_windows(&["alpha", "beta"]);
    execute_command_string(&mut app, "kill-window -t kwtest:99").unwrap();
    assert_eq!(window_names(&app), vec!["alpha", "beta"]);
    assert!(app.status_message.is_some(), "bad index must surface an error");
}

#[test]
fn bad_id_target_kills_nothing() {
    let mut app = app_with_windows(&["alpha", "beta"]);
    execute_command_string(&mut app, "kill-window -t @99").unwrap();
    assert_eq!(window_names(&app), vec!["alpha", "beta"]);
    assert!(app.status_message.is_some(), "bad @id must surface an error");
}

#[test]
fn bad_name_on_last_window_does_not_empty_the_session() {
    // The reported death scenario: one window, typoed target. The old code
    // killed the only window, which ends the whole session.
    let mut app = app_with_windows(&["only"]);
    execute_command_string(&mut app, "kill-window -t kwtest:typo").unwrap();
    assert_eq!(window_names(&app), vec!["only"], "the last window must survive a bad target");
}

#[test]
fn killw_alias_honors_bad_target() {
    let mut app = app_with_windows(&["alpha", "beta"]);
    execute_command_string(&mut app, "killw -t kwtest:nope").unwrap();
    assert_eq!(window_names(&app), vec!["alpha", "beta"]);
}

#[test]
fn valid_name_target_kills_exactly_that_window() {
    let mut app = app_with_windows(&["alpha", "beta", "gamma"]);
    app.active_idx = 0;

    execute_command_string(&mut app, "kill-window -t kwtest:beta").unwrap();

    assert_eq!(window_names(&app), vec!["alpha", "gamma"]);
    assert_eq!(app.active_idx, 0, "killing a later window keeps focus in place");
}

#[test]
fn valid_id_target_kills_exactly_that_window() {
    let mut app = app_with_windows(&["alpha", "beta", "gamma"]);
    app.active_idx = 2;

    execute_command_string(&mut app, "kill-window -t @0").unwrap();

    assert_eq!(window_names(&app), vec!["beta", "gamma"]);
    assert_eq!(
        app.active_idx, 1,
        "removing a window before the active one shifts focus down to the same window"
    );
    assert_eq!(app.windows[app.active_idx].name, "gamma", "focus stays on the same window");
}

#[test]
fn no_target_still_kills_the_active_window() {
    let mut app = app_with_windows(&["alpha", "beta"]);
    app.active_idx = 1;

    execute_command_string(&mut app, "kill-window").unwrap();

    assert_eq!(window_names(&app), vec!["alpha"]);
}

#[test]
fn bare_session_target_kills_the_active_window() {
    // `kill-window -t kwtest` names only the session; tmux kills the session's
    // current window in that case.
    let mut app = app_with_windows(&["alpha", "beta"]);
    app.active_idx = 0;

    execute_command_string(&mut app, "kill-window -t kwtest").unwrap();

    assert_eq!(window_names(&app), vec!["beta"]);
}

#[test]
fn missing_target_value_fails_before_local_mutation() {
    let mut app = app_with_windows(&["alpha", "beta"]);

    let error = execute_command_string(&mut app, "kill-window -t").unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "-t expects an argument");
    assert_eq!(window_names(&app), vec!["alpha", "beta"]);
    assert_eq!(
        app.status_message.as_ref().map(|message| message.0.as_str()),
        Some("-t expects an argument")
    );
}

#[test]
fn missing_target_value_rejects_local_deferred_commands() {
    let mut app = app_with_windows(&["alpha", "beta"]);

    for command in [
        "confirm-before kill-window -t",
        "confirm-before 'kill-window -t'",
        "bind-key x kill-window -t",
        "set-hook pane-died killw -t",
    ] {
        let error = execute_command_string(&mut app, command).unwrap_err();
        assert_eq!(error.to_string(), "-t expects an argument");
    }
    assert!(!matches!(app.mode, Mode::ConfirmMode { .. }));
    assert!(app.key_tables.get("prefix").is_none_or(|bindings| {
        bindings
            .iter()
            .all(|binding| binding.key.0 != crossterm::event::KeyCode::Char('x'))
    }));
    assert!(!app.hooks.contains_key("pane-died"));
}

#[test]
fn repeated_targets_use_the_last_value() {
    let mut app = app_with_windows(&["alpha", "beta", "gamma"]);

    execute_command_string(&mut app, "kill-window -t @1 -t@2").unwrap();

    assert_eq!(window_names(&app), vec!["alpha", "beta"]);
}

#[test]
fn deferred_command_nesting_is_bounded() {
    let mut app = app_with_windows(&["alpha", "beta"]);
    let command = format!("{}kill-window", "confirm-before ".repeat(65));

    let error = execute_command_string(&mut app, &command).unwrap_err();

    assert_eq!(
        error.to_string(),
        "command nesting exceeds 64 levels"
    );
    assert_eq!(window_names(&app), vec!["alpha", "beta"]);
}
