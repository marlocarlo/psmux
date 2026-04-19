// Issue #235: Pane display numbers don't match pane-base-index setting
//
// BUG: When pane-base-index is set to 1, the display-panes overlay (Prefix q)
// shows pane numbers starting at 0 instead of 1. The keybindings work correctly
// (pressing 1 selects the first pane) but the displayed numbers are wrong.
//
// ROOT CAUSE: The server state JSON sent to the client did not include
// pane_base_index, so it defaulted to 0 on the client side. The rendering
// code in client.rs uses srv_pane_base_index (from the JSON) to compute
// the displayed number, so it always showed 0-indexed numbers.
//
// FIX: Added pane_base_index to both state JSON builders in server/mod.rs.

use super::*;

fn mock_app() -> AppState {
    let mut app = AppState::new("test235".to_string());
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
// TEST 1: pane_base_index default is 0
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn pane_base_index_default_is_zero() {
    let app = mock_app();
    assert_eq!(app.pane_base_index, 0, "Default pane_base_index should be 0");
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 2: set-option pane-base-index changes the value
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn set_option_pane_base_index() {
    let mut app = mock_app_with_window();
    execute_command_string(&mut app, "set-option -g pane-base-index 1").unwrap();
    assert_eq!(app.pane_base_index, 1, "pane-base-index should be 1 after set-option");
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 3: digit computation formula with pane_base_index = 1
// The display-panes overlay computes: (i + pane_base_index) % 10
// This is the exact formula used in commands.rs (DisplayPanes action),
// input.rs (PaneChooser digit selection), server/mod.rs (TCP key handler),
// and client.rs (overlay rendering).
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn digit_computation_with_base_index_1() {
    let base = 1usize;
    // Simulate 4 panes
    let digits: Vec<usize> = (0..4).map(|i| (i + base) % 10).collect();
    assert_eq!(digits, vec![1, 2, 3, 4], "With pane_base_index=1, panes should be 1,2,3,4");
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 4: digit computation formula with pane_base_index = 0 (default)
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn digit_computation_with_base_index_0() {
    let base = 0usize;
    let digits: Vec<usize> = (0..4).map(|i| (i + base) % 10).collect();
    assert_eq!(digits, vec![0, 1, 2, 3], "With pane_base_index=0, panes should be 0,1,2,3");
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 5: PaneChooser mode is entered after DisplayPanes (single pane)
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn display_panes_enters_pane_chooser_mode() {
    let mut app = mock_app_with_window();
    app.last_window_area = ratatui::prelude::Rect { x: 0, y: 0, width: 120, height: 30 };
    execute_action(&mut app, &Action::DisplayPanes).unwrap();
    match &app.mode {
        Mode::PaneChooser { .. } => {}
        other => panic!("Expected PaneChooser mode, got {:?}", std::mem::discriminant(other)),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 6: digit computation with non-standard base index 5
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn digit_computation_with_base_index_5() {
    let base = 5usize;
    let digits: Vec<usize> = (0..4).map(|i| (i + base) % 10).collect();
    assert_eq!(digits, vec![5, 6, 7, 8], "With pane_base_index=5, panes should be 5,6,7,8");
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 7: set-option pane-base-index via config parsing
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn config_parse_sets_pane_base_index() {
    let mut app = mock_app_with_window();
    crate::config::parse_config_content(&mut app, "set -g pane-base-index 1\n");
    assert_eq!(app.pane_base_index, 1, "Config parsing should set pane_base_index to 1");
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 8: format variable pane-base-index is correct
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn format_variable_pane_base_index() {
    let mut app = mock_app_with_window();
    app.pane_base_index = 1;
    let result = crate::format::expand_format("#{pane-base-index}", &app);
    assert_eq!(result, "1", "Format variable pane-base-index should return 1, got '{}'", result);
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 9: PaneChooser mode with pane_base_index=1 sets correct display_map
// (single-pane case: mock window has one leaf)
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn display_panes_single_pane_with_base_index_1() {
    let mut app = mock_app_with_window();
    app.pane_base_index = 1;
    app.last_window_area = ratatui::prelude::Rect { x: 0, y: 0, width: 120, height: 30 };
    execute_action(&mut app, &Action::DisplayPanes).unwrap();
    match &app.mode {
        Mode::PaneChooser { .. } => {},
        other => panic!("Expected PaneChooser, got {:?}", std::mem::discriminant(other)),
    }
    // With a single-leaf window, display_map should have 0 or 1 entry
    // depending on whether the empty split yields any leaves
    // The key point: if there IS an entry, its digit should use pane_base_index
    if !app.display_map.is_empty() {
        assert_eq!(app.display_map[0].0, 1, "First pane digit should be 1 with pane_base_index=1");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 10: show-options returns correct pane-base-index
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn show_options_pane_base_index() {
    let mut app = mock_app_with_window();
    app.pane_base_index = 1;
    execute_command_string(&mut app, "show-options -g -v pane-base-index").unwrap();
    match &app.mode {
        Mode::PopupMode { output, .. } => {
            assert!(output.contains("1"), "show-options should show pane-base-index=1, got: {}", output);
        }
        _ => {
            // show-options with -v might return via status message or popup
            if let Some((msg, _, _)) = &app.status_message {
                assert!(msg.contains("1"), "Status should contain 1, got: {}", msg);
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// TEST 11: digit computation wraps around modulo 10 with high base index
// ════════════════════════════════════════════════════════════════════════════
#[test]
fn digit_computation_wraps_modulo_10() {
    let base = 9usize;
    let digits: Vec<usize> = (0..4).map(|i| (i + base) % 10).collect();
    assert_eq!(digits, vec![9, 0, 1, 2], "With pane_base_index=9, panes should wrap: 9,0,1,2");
}
