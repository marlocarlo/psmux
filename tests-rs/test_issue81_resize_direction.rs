// Regression tests for issue #81: pane resize reverses direction when
// the default border does not exist (active pane on window edge).
//
// tmux behaviour: when the active pane is on the bottom/right edge and
// the default border (bottom / right) is absent, tmux moves the
// opposite border in the arrow direction.
//
// psmux bug: the opposite border moved in the REVERSE direction.

use super::*;

// Helper: build a minimal AppState whose single window has a two-child
// split (no real PTY needed; only the `sizes` vec is accessed).
fn app_with_split(kind: LayoutKind, active_child: usize) -> AppState {
    let mut app = AppState::new("test".to_string());
    let placeholder = || Node::Split {
        kind: LayoutKind::Horizontal,
        sizes: vec![],
        children: vec![],
    };
    let root = Node::Split {
        kind,
        sizes: vec![50, 50],
        children: vec![placeholder(), placeholder()],
    };
    let win = Window {
        root,
        active_path: vec![active_child],
        name: "test".to_string(),
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
    app
}

fn get_sizes(app: &AppState) -> Vec<u16> {
    match &app.windows[0].root {
        Node::Split { sizes, .. } => sizes.clone(),
        _ => panic!("expected split root"),
    }
}

// ═══════════════════════════════════════════════════════════
//  Vertical split (top / bottom): resize-pane -D / -U on bottom pane
// ═══════════════════════════════════════════════════════════

#[test]
fn resize_down_on_bottom_pane_shrinks_bottom_pane() {
    // Bottom pane (idx=1) has no bottom border.
    // tmux: -D moves the top border DOWN => bottom pane shrinks.
    let mut app = app_with_split(LayoutKind::Vertical, 1);
    resize_pane_vertical(&mut app, 1); // -D => amount +1
    let s = get_sizes(&app);
    assert!(s[1] < 50, "bottom pane should shrink when -D with no bottom border (got {})", s[1]);
    assert!(s[0] > 50, "top pane should grow when -D with no bottom border (got {})", s[0]);
}

#[test]
fn resize_up_on_bottom_pane_grows_bottom_pane() {
    // -U on bottom pane => top border moves UP => bottom pane grows.
    let mut app = app_with_split(LayoutKind::Vertical, 1);
    resize_pane_vertical(&mut app, -1); // -U => amount -1
    let s = get_sizes(&app);
    assert!(s[1] > 50, "bottom pane should grow when -U with no bottom border (got {})", s[1]);
    assert!(s[0] < 50, "top pane should shrink when -U with no bottom border (got {})", s[0]);
}

// ═══════════════════════════════════════════════════════════
//  Horizontal split (left / right): resize-pane -R / -L on right pane
// ═══════════════════════════════════════════════════════════

#[test]
fn resize_right_on_right_pane_shrinks_right_pane() {
    // Right pane (idx=1) has no right border.
    // tmux: -R moves the left border RIGHT => right pane shrinks.
    let mut app = app_with_split(LayoutKind::Horizontal, 1);
    resize_pane_horizontal(&mut app, 1); // -R => amount +1
    let s = get_sizes(&app);
    assert!(s[1] < 50, "right pane should shrink when -R with no right border (got {})", s[1]);
    assert!(s[0] > 50, "left pane should grow when -R with no right border (got {})", s[0]);
}

#[test]
fn resize_left_on_right_pane_grows_right_pane() {
    // -L on right pane => left border moves LEFT => right pane grows.
    let mut app = app_with_split(LayoutKind::Horizontal, 1);
    resize_pane_horizontal(&mut app, -1); // -L => amount -1
    let s = get_sizes(&app);
    assert!(s[1] > 50, "right pane should grow when -L with no right border (got {})", s[1]);
    assert!(s[0] < 50, "left pane should shrink when -L with no right border (got {})", s[0]);
}

// ═══════════════════════════════════════════════════════════
//  Normal path: first child (has right/bottom neighbor) still works
// ═══════════════════════════════════════════════════════════

#[test]
fn resize_down_on_top_pane_grows_top_pane() {
    // Top pane (idx=0) HAS a bottom border (idx+1 exists).
    // -D => grow top pane, shrink bottom pane.
    let mut app = app_with_split(LayoutKind::Vertical, 0);
    resize_pane_vertical(&mut app, 1);
    let s = get_sizes(&app);
    assert!(s[0] > 50, "top pane should grow when -D with bottom border (got {})", s[0]);
    assert!(s[1] < 50, "bottom pane should shrink when -D with bottom border (got {})", s[1]);
}

#[test]
fn resize_right_on_left_pane_grows_left_pane() {
    // Left pane (idx=0) HAS a right border (idx+1 exists).
    // -R => grow left pane, shrink right pane.
    let mut app = app_with_split(LayoutKind::Horizontal, 0);
    resize_pane_horizontal(&mut app, 1);
    let s = get_sizes(&app);
    assert!(s[0] > 50, "left pane should grow when -R with right border (got {})", s[0]);
    assert!(s[1] < 50, "right pane should shrink when -R with right border (got {})", s[1]);
}
