//! Unit tests for the choose-tree active-window marker and initial selection
//! (the picker opens with the cursor on the current window). The picker's
//! entry-building logic is factored into the pure `build_tree_entries` /
//! `build_fallback_entries` helpers in `client.rs` precisely so this behaviour
//! is verifiable without driving the TUI.
//!
//! The helpers are platform-independent (pure string/index logic), so these
//! tests run on every CI target rather than being gated to Windows.

use super::*;
use crate::util::{WinTree, PaneInfo};

/// Construct a `ChooserWin`: `(id, name, active, panes[(id, title)])`.
fn win(id: usize, name: &str, active: bool, panes: &[(usize, &str)]) -> ChooserWin {
    (id, name.to_string(), active, panes.iter().map(|(i, t)| (*i, t.to_string())).collect())
}

/// Construct a `WinTree` for the cached-fallback path tests.
fn wtree(id: usize, name: &str, active: bool, panes: &[(usize, &str)]) -> WinTree {
    WinTree {
        id,
        name: name.to_string(),
        active,
        panes: panes.iter().map(|(i, t)| PaneInfo { id: *i, title: t.to_string() }).collect(),
    }
}

// ---- build_tree_entries: marker rendering --------------------------------

#[test]
fn active_marker_appended_after_name() {
    let sessions = vec![("work".to_string(), vec![win(0, "editor", true, &[(0, "p")])])];
    let (entries, _) = build_tree_entries(&sessions, "work");
    // entries[0] is the session header; entries[1] is the (only) window row.
    assert_eq!(entries[1].3, "  0: editor* (1 panes)");
}

#[test]
fn inactive_window_has_no_marker() {
    let sessions = vec![("work".to_string(), vec![win(0, "editor", false, &[(0, "p")])])];
    let (entries, _) = build_tree_entries(&sessions, "work");
    assert_eq!(entries[1].3, "  0: editor (1 panes)");
}

// ---- build_tree_entries: initial selection (absolute row index) ----------

#[test]
fn single_active_window_is_selected() {
    let sessions = vec![("work".to_string(), vec![win(0, "only", true, &[(0, "p")])])];
    let (_entries, selected) = build_tree_entries(&sessions, "work");
    // header(0), window(1) -> the window row is index 1.
    assert_eq!(selected, 1);
}

#[test]
fn no_active_window_selects_zero_and_renders_no_marker() {
    let sessions = vec![("work".to_string(), vec![
        win(0, "a", false, &[(0, "p")]),
        win(1, "b", false, &[(1, "q")]),
    ])];
    let (entries, selected) = build_tree_entries(&sessions, "work");
    assert_eq!(selected, 0);
    assert!(entries.iter().all(|e| !e.3.contains('*')));
}

#[test]
fn active_window_not_first_selects_correct_absolute_row() {
    // header(0), a(1), a-pane(2), b ACTIVE(3), b-pane(4)
    let sessions = vec![("work".to_string(), vec![
        win(0, "a", false, &[(10, "p")]),
        win(1, "b", true, &[(11, "q")]),
    ])];
    let (entries, selected) = build_tree_entries(&sessions, "work");
    assert_eq!(selected, 3);
    assert_eq!(entries[selected].3, "  1: b* (1 panes)");
}

#[test]
fn active_window_last_selects_last_window_row() {
    // No panes, so rows are contiguous: header(0), a(1), b(2), c ACTIVE(3).
    let sessions = vec![("work".to_string(), vec![
        win(0, "a", false, &[]),
        win(1, "b", false, &[]),
        win(2, "c", true, &[]),
    ])];
    let (_entries, selected) = build_tree_entries(&sessions, "work");
    assert_eq!(selected, 3);
}

#[test]
fn pane_rows_offset_the_selected_index() {
    // The first window owns TWO panes, pushing the active second window down.
    // header(0), w0(1), w0-p0(2), w0-p1(3), w1 ACTIVE(4), w1-p(5)
    let sessions = vec![("work".to_string(), vec![
        win(0, "a", false, &[(1, "p"), (2, "q")]),
        win(1, "b", true, &[(3, "r")]),
    ])];
    let (entries, selected) = build_tree_entries(&sessions, "work");
    assert_eq!(selected, 4);
    assert_eq!(entries[4].3, "  1: b* (1 panes)"); // pin row identity, not just the index
}

// ---- build_tree_entries: cross-session isolation -------------------------

#[test]
fn other_session_active_window_is_not_marked_or_selected() {
    // The current session "work" has no active window, while a DIFFERENT
    // session "play" does. The cursor must not jump across sessions, and the
    // other (collapsed) session's window must carry no marker.
    let sessions = vec![
        ("work".to_string(), vec![win(0, "a", false, &[(1, "p")])]),
        ("play".to_string(), vec![win(0, "x", true, &[(2, "q")])]),
    ];
    let (entries, selected) = build_tree_entries(&sessions, "work");
    assert_eq!(selected, 0);
    assert!(entries.iter().all(|e| !e.3.contains('*')));
    // The other (non-current) session's window stays collapsed, carries no
    // marker, and keeps its normal label shape. Rows: work header(0),
    // work win a(1), work pane(2), play header(3), play win x(4).
    assert_eq!(entries[4].3, "  0: x (1 panes)");
}

// ---- build_fallback_entries: cached last_tree path -----------------------

#[test]
fn fallback_marks_and_selects_active_window() {
    // rows: a(0), a-pane(1), b ACTIVE(2), b-pane(3)
    let last = vec![
        wtree(0, "a", false, &[(1, "p")]),
        wtree(1, "b", true, &[(2, "q")]),
    ];
    let (entries, selected) = build_fallback_entries(&last, "work");
    assert_eq!(selected, 2);
    assert_eq!(entries[2].3, "b*"); // minimal cached layout: bare name + marker
    assert_eq!(entries[0].3, "a");   // inactive window: no marker
}

#[test]
fn fallback_no_active_window_selects_zero() {
    let last = vec![wtree(0, "a", false, &[]), wtree(1, "b", false, &[])];
    let (entries, selected) = build_fallback_entries(&last, "work");
    assert_eq!(selected, 0);
    assert!(entries.iter().all(|e| !e.3.contains('*')));
}
