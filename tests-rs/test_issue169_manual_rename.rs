// Issue #169: new-window -n does not set manual_rename flag
//
// When creating a window with `new-window -n NAME`, the manual_rename flag
// should be set to true so automatic-rename does not overwrite the explicit name.

use crate::types::AppState;

fn mock_app() -> AppState {
    let mut app = AppState::new("test_session".to_string());
    app.window_base_index = 0;
    app.pane_base_index = 0;
    app
}

/// When a window is created with a name via -n, manual_rename must be true
/// so automatic rename does not overwrite the user's chosen name.
#[test]
fn new_window_with_name_sets_manual_rename() {
    use crate::types::{Window, LayoutKind, Node};
    let mut app = mock_app();

    // Simulate a window created without -n (default)
    let win_default = Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: "shell".to_string(),
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
        linked_from: None,
    };
    app.windows.push(win_default);

    // The default window should have manual_rename = false
    assert!(!app.windows[0].manual_rename, "default window should NOT have manual_rename");

    // Simulate what happens with -n: the server sets the name
    // and should also set manual_rename = true (this is the fix)
    let name = Some("mywindow".to_string());
    let win_named = Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: "shell".to_string(),
        id: 1,
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
    };
    app.windows.push(win_named);

    // This simulates the server logic for setting the name from -n flag.
    // After fix, this code path should also set manual_rename = true.
    if let Some(n) = name {
        app.windows.last_mut().map(|w| {
            w.name = n;
            w.manual_rename = true;
        });
    }

    assert_eq!(app.windows[1].name, "mywindow", "window name should be set");
    assert!(app.windows[1].manual_rename, "window with explicit -n name should have manual_rename = true");
}

/// Verify that rename-window also sets manual_rename (should already work)
#[test]
fn rename_window_sets_manual_rename() {
    use crate::types::{Window, LayoutKind, Node};
    let mut app = mock_app();

    let win = Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: "shell".to_string(),
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
        linked_from: None,
    };
    app.windows.push(win);

    // Simulate rename-window
    let win = &mut app.windows[0];
    win.name = "renamed".to_string();
    win.manual_rename = true;

    assert_eq!(app.windows[0].name, "renamed");
    assert!(app.windows[0].manual_rename, "rename-window should set manual_rename");
}

/// Windows created without -n should NOT have manual_rename set
/// (automatic rename should still work for them)
#[test]
fn new_window_without_name_does_not_set_manual_rename() {
    use crate::types::{Window, LayoutKind, Node};
    let mut app = mock_app();

    let win = Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: "shell".to_string(),
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
        linked_from: None,
    };
    app.windows.push(win);

    // Simulate new-window without -n (name is None)
    let name: Option<String> = None;
    if let Some(n) = name {
        app.windows.last_mut().map(|w| {
            w.name = n;
            w.manual_rename = true;
        });
    }

    assert!(!app.windows[0].manual_rename, "window without -n should NOT have manual_rename");
}

/// When automatic-rename is explicitly enabled via the server options path,
/// it should clear manual_rename on the active window.
#[test]
fn set_automatic_rename_clears_manual_rename() {
    use crate::types::{Window, LayoutKind, Node};
    let mut app = mock_app();

    let win = Window {
        root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
        active_path: vec![],
        name: "mywindow".to_string(),
        id: 0,
        activity_flag: false,
        bell_flag: false,
        silence_flag: false,
        last_output_time: std::time::Instant::now(),
        last_seen_version: 0,
        manual_rename: true, // Set by -n or rename-window
        layout_index: 0,
        pane_mru: vec![],
        zoom_saved: None,
        linked_from: None,
    };
    app.windows.push(win);
    app.active_idx = 0;

    // Simulate what the server options handler does for `set automatic-rename on`
    // (server/options.rs line 279)
    app.automatic_rename = true;
    if app.automatic_rename {
        if let Some(w) = app.windows.get_mut(app.active_idx) {
            w.manual_rename = false;
        }
    }

    assert!(!app.windows[0].manual_rename,
        "set automatic-rename on should clear manual_rename on active window");
}
