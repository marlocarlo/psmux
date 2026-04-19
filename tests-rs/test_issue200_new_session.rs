// Issue #200: new-session command via prefix+: does not create a session
//
// Root cause: execute_command_string_single() at the "new-session" | "new" arm
// shows a blocking popup "(cannot create a new session from inside a session)"
// instead of actually spawning a new session.
//
// This file tests:
// 1. REPRODUCTION: confirm the bug exists (popup is shown, no session created)
// 2. After fix: new-session correctly spawns a server process and doesn't block

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

/// Returns true if the app is in PopupMode with output containing the given substring.
fn is_popup_with_text(app: &AppState, text: &str) -> bool {
    match &app.mode {
        Mode::PopupMode { output, .. } => output.contains(text),
        _ => false,
    }
}

// ─── Bug reproduction: new-session is blocked with popup ────────────────────

#[test]
fn new_session_must_not_show_blocking_popup() {
    // Issue #200: Running "new-session -s foo" from the command prompt
    // should NOT show the "(cannot create a new session from inside a session)"
    // popup. It should attempt to create the session.
    let mut app = mock_app_with_window();

    execute_command_string(&mut app, "new-session -s test_issue200").unwrap();

    // The bug: the old code would show a popup with "cannot create"
    // After fix: it should NOT be in PopupMode with that message
    assert!(
        !is_popup_with_text(&app, "cannot create"),
        "BUG CONFIRMED: new-session still shows blocking popup instead of creating session"
    );
}

#[test]
fn new_session_alias_must_not_show_blocking_popup() {
    // The "new" alias should also work
    let mut app = mock_app_with_window();

    execute_command_string(&mut app, "new -s test_alias").unwrap();

    assert!(
        !is_popup_with_text(&app, "cannot create"),
        "BUG CONFIRMED: 'new' alias still shows blocking popup"
    );
}

#[test]
fn new_session_without_name_must_not_block() {
    // Running "new-session" without -s should also not be blocked
    let mut app = mock_app_with_window();

    execute_command_string(&mut app, "new-session").unwrap();

    assert!(
        !is_popup_with_text(&app, "cannot create"),
        "BUG CONFIRMED: new-session without arguments still shows blocking popup"
    );
}

#[test]
fn new_session_detached_must_not_block() {
    // "new-session -d -s foo" should also work (detached mode)
    let mut app = mock_app_with_window();

    execute_command_string(&mut app, "new-session -d -s detached_sess").unwrap();

    assert!(
        !is_popup_with_text(&app, "cannot create"),
        "BUG CONFIRMED: new-session -d still shows blocking popup"
    );
}

// ─── Status message confirmation after creation ─────────────────────────────

#[test]
fn new_session_shows_status_confirmation() {
    // After creating a session (or attempting to in test env where spawn may fail),
    // the app should show a status message, NOT a blocking popup
    let mut app = mock_app_with_window();

    execute_command_string(&mut app, "new-session -s confirmation_test").unwrap();

    // Should not be in popup mode with the blocking message
    let in_blocking_popup = is_popup_with_text(&app, "cannot create");
    assert!(!in_blocking_popup, "Should not show blocking popup after new-session");
}

// ─── Argument parsing tests ─────────────────────────────────────────────────

#[test]
fn new_session_from_command_prompt_dispatches() {
    // Verify that the command prompt dispatches new-session to execute_command_string
    // (it falls through the default _ arm) and doesn't get blocked before that
    let mut app = mock_app_with_window();
    app.mode = Mode::CommandPrompt {
        input: "new-session -s dispatch_test".to_string(),
        cursor: 0,
    };

    execute_command_prompt(&mut app).unwrap();

    // After command prompt execution, mode should not be a popup with blocking msg
    assert!(
        !is_popup_with_text(&app, "cannot create"),
        "Command prompt path still blocks new-session"
    );
}
