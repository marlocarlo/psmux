#[allow(unused_imports)]
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{PtySize, native_pty_system};
use ratatui::prelude::*;

use crate::types::{AppState, Mode, Pane, Node, LayoutKind, DragState, Window, FocusDir};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, get_split_mut, resize_all_panes};
use crate::pane::{detect_shell, build_default_shell, set_tmux_env};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, yank_selection};
use crate::platform::mouse_inject;

/// Mouse debug logger — writes to ~/.psmux/mouse_debug.log when
/// PSMUX_MOUSE_DEBUG=1 is set.
use super::*;

pub fn toggle_zoom(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    if win.zoom_saved.is_none() {
        let mut saved: Vec<(Vec<usize>, Vec<u16>)> = Vec::new();
        for depth in 0..win.active_path.len() {
            let p = win.active_path[..depth].to_vec();
            if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) {
                let idx = win.active_path.get(depth).copied().unwrap_or(0);
                saved.push((p.clone(), sizes.clone()));
                for i in 0..sizes.len() { sizes[i] = if i == idx { 100 } else { 0 }; }
            }
        }
        win.zoom_saved = Some(saved);
    } else {
        if let Some(saved) = app.windows[app.active_idx].zoom_saved.take() {
            let win = &mut app.windows[app.active_idx];
            for (p, sz) in saved.into_iter() {
                if let Some(Node::Split { sizes, .. }) = get_split_mut(&mut win.root, &p) { *sizes = sz; }
            }
        }
    }
    // Resize all panes so child PTYs are notified of the new dimensions.
    // Without this, zoomed panes keep their pre-zoom size and child apps
    // (neovim, bottom, etc.) render in only half the screen. (issue #35)
    resize_all_panes(app);
}

/// Compute tab positions on the server side to match the client's status bar layout.
/// The client renders: "[session_name] idx: window_name idx: window_name ..."
/// NOTE: No longer called — tab clicks are now handled client-side with exact
/// rendered positions.  Kept for reference / potential embedded-mode use.
#[allow(dead_code)]
pub fn update_tab_positions(app: &mut AppState) {
    let mut tab_pos: Vec<(usize, u16, u16)> = Vec::new();
    let mut cursor_x: u16 = 0;
    // Session label: "[session_name] "
    let session_label_len = app.session_name.len() as u16 + 3; // '[' + name + ']' + ' '
    cursor_x += session_label_len;
    // Window tabs: "idx: window_name " for each window
    for (i, w) in app.windows.iter().enumerate() {
        let display_idx = i + app.window_base_index;
        let label = format!("{}: {} ", display_idx, w.name);
        let start_x = cursor_x;
        cursor_x += label.len() as u16;
        tab_pos.push((i, start_x, cursor_x));
    }
    app.tab_positions = tab_pos;
}

pub fn remote_mouse_down(app: &mut AppState, x: u16, y: u16) {
    let (x, y) = map_client_coords(app, x, y);
    // Status bar tab clicks are handled client-side via select-window.
    // Only handle pane focus and border resize here.
    let status_row = app.last_window_area.y + app.last_window_area.height;
    if y == status_row {
        return;
    }

    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    let mut active_area: Option<Rect> = None;
    for (path, area) in rects.iter() {
        if area.contains(ratatui::layout::Position { x, y }) {
            win.active_path = path.clone();
            // Update MRU for clicked pane (tmux parity #70)
            if let Some(pid) = crate::tree::get_active_pane_id(&win.root, path) {
                crate::tree::touch_mru(&mut win.pane_mru, pid);
            }
            active_area = Some(*area);
        }
    }

    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        app.copy_anchor = None;
        if let Some(area) = active_area {
            let (row, col) = copy_cell_for_area(area, x, y);
            app.copy_pos = Some((row, col));
            app.copy_mouse_down_cell = Some((row, col));
        }
        return;
    }

    let mut on_border = false;
    // Skip border detection when zoomed — no visible borders (#82)
    let mut borders: Vec<(Vec<usize>, LayoutKind, usize, u16, u16)> = Vec::new();
    if win.zoom_saved.is_none() {
        compute_split_borders(&win.root, app.last_window_area, &mut borders);
    }
    let tol = 1u16;
    for (path, kind, idx, pos, total_px) in borders.iter() {
        match kind {
            LayoutKind::Horizontal => {
                if x >= pos.saturating_sub(tol) && x <= pos + tol { if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) { app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: *pos, start_y: y, left_initial: left, _right_initial: right, total_pixels: *total_px }); } on_border = true; break; }
            }
            LayoutKind::Vertical => {
                if y >= pos.saturating_sub(tol) && y <= pos + tol { if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) { app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: x, start_y: *pos, left_initial: left, _right_initial: right, total_pixels: *total_px }); } on_border = true; break; }
            }
        }
    }

    // Forward left-click only when active pane wants mouse input.
    if !on_border {
        if let Some(area) = active_area {
            let (col, row) = pane_inner_cell_0based(area, x, y);
            let win_name = win.name.clone();
            if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                if pane_wants_mouse(active) {
                    inject_mouse_combined(active, col, row, 0, true,
                        mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED, 0, &win_name);
                }
            }
        }
    }
}

pub fn remote_mouse_drag(app: &mut AppState, x: u16, y: u16) {
    let (x, y) = map_client_coords(app, x, y);
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);

    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x, y })) {
            win.active_path = path.clone();
            let (row, col) = copy_cell_for_area(*area, x, y);
            if app.copy_anchor.is_none() {
                // Only start selection when mouse moves to a different cell
                // than the click position. Prevents micro-drag jitter (#199).
                if app.copy_pos == Some((row, col)) {
                    return;
                }
                app.copy_anchor = Some(app.copy_pos.unwrap_or((row, col)));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
            app.copy_pos = Some((row, col));
        }
        return;
    }

    if let Some(d) = &app.drag {
        adjust_split_sizes(&mut win.root, d, x, y);
    } else {
        // Forward drag only when active pane wants mouse input.
        if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
            let (col, row) = pane_inner_cell_0based(area, x, y);
            let win_name = win.name.clone();
            if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                if pane_wants_mouse(active) {
                    inject_mouse_combined(active, col, row, 32, true,
                        mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED, mouse_inject::MOUSE_MOVED, &win_name);
                }
            }
        }
    }
}

pub fn remote_mouse_up(app: &mut AppState, x: u16, y: u16) {
    let (x, y) = map_client_coords(app, x, y);
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);

    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x, y })) {
            win.active_path = path.clone();
            let (row, col) = copy_cell_for_area(*area, x, y);
            app.copy_pos = Some((row, col));
        }
        // If mouse-up is within 1 cell of mouse-down, it was a plain click
        // (any anchor set by jittery drag events is spurious). Clear it. (#199)
        // Mouse jitter during a click can shift the cursor by 1 cell.
        let click_origin = app.copy_mouse_down_cell.take();
        if let (Some((dr, dc)), Some((ur, uc))) = (click_origin, app.copy_pos) {
            let row_diff = (dr as i32 - ur as i32).unsigned_abs();
            let col_diff = (dc as i32 - uc as i32).unsigned_abs();
            if row_diff <= 1 && col_diff <= 1 {
                app.copy_anchor = None;
                app.copy_pos = Some((dr, dc)); // snap to the original click position
                return;
            }
        }
        // Auto-yank if real selection exists (anchor != pos), else clear stale anchor
        if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
            if a != p {
                let _ = yank_selection(app);
            } else {
                app.copy_anchor = None;
            }
        }
        return;
    }

    // If we were dragging a border, resize all panes to match new layout
    let was_dragging = app.drag.is_some();
    app.drag = None;
    if was_dragging {
        resize_all_panes(app);
        return;
    }

    // Forward mouse release only when active pane wants mouse input.
    if let Some(area) = rects.iter().find(|(path, _)| *path == win.active_path).map(|(_, a)| *a) {
        let (col, row) = pane_inner_cell_0based(area, x, y);
        let win_name = win.name.clone();
        if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
            if pane_wants_mouse(active) {
                inject_mouse_combined(active, col, row, 0, false,
                    0, 0, &win_name);
            }
        }
    }
}

pub(crate) fn wheel_cell_for_area(area: Rect, x: u16, y: u16) -> (u16, u16) {
    // Convert global terminal coordinates to 1-based pane-local coordinates (no border offset).
    let col = x.saturating_sub(area.x).min(area.width.saturating_sub(1)).saturating_add(1);
    let row = y.saturating_sub(area.y).min(area.height.saturating_sub(1)).saturating_add(1);
    (col, row)
}

pub(crate) fn copy_cell_for_area(area: Rect, x: u16, y: u16) -> (u16, u16) {
    // Convert global terminal coordinates to 0-based pane-local coordinates (no border offset).
    let col = x.saturating_sub(area.x).min(area.width.saturating_sub(1));
    let row = y.saturating_sub(area.y).min(area.height.saturating_sub(1));
    (row, col)
}

pub(crate) fn remote_scroll_wheel(app: &mut AppState, x: u16, y: u16, up: bool) {
    let (x, y) = map_client_coords(app, x, y);
    let mode_str = match &app.mode {
        Mode::Passthrough => "Passthrough",
        Mode::CopyMode => "CopyMode",
        Mode::CopySearch { .. } => "CopySearch",
        _ => "Other",
    };
    mouse_log(&format!("remote_scroll_wheel: x={} y={} up={} mode={}", x, y, up, mode_str));

    // Ignore scroll in popup mode — don't enter copy-mode (#110)
    if matches!(app.mode, Mode::PopupMode { .. }) {
        mouse_log("  -> popup mode, ignoring scroll");
        return;
    }

    // Handle scroll while already in copy mode
    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        mouse_log("  -> already in copy mode, scrolling within");
        if up {
            scroll_copy_up(app, 3);
        } else {
            scroll_copy_down(app, 3);
            // Auto-exit copy mode when scrolled back to live output
            if app.copy_scroll_offset == 0 && app.copy_anchor.is_none() {
                exit_copy_mode(app);
            }
        }
        return;
    }

    // Determine target pane, switch focus, and check if child is in alternate screen.
    //
    // IMPORTANT (tmux parity): For scroll events, we ONLY check alternate_screen()
    // to decide whether to forward to the child or enter copy mode.
    //
    // We do NOT use:
    //   - pane_wants_mouse() / mouse_protocol_mode(): PSReadLine on ConPTY
    //     spuriously enables AnyMotion mouse tracking.
    //   - is_fullscreen_tui() heuristic: A shell prompt after `ls` / `dir` can
    //     fill the last rows + leave the cursor at the bottom, causing a false
    //     positive that prevents scroll-to-copy-mode.
    //
    // alternate_screen() is reliable: all modern TUI apps (nvim, htop, vim,
    // opencode) correctly report alternate screen through ConPTY.  Testing
    // confirms nvim shows alternate_on=1.  Shell prompts always show 0.
    let (child_in_alt_screen, target_area_opt, sgr_btn, button_state) = {
        let win = &mut app.windows[app.active_idx];
        let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
        compute_rects(&win.root, app.last_window_area, &mut rects);

        let mut target_area: Option<Rect> = None;
        for (path, area) in &rects {
            if area.contains(ratatui::layout::Position { x, y }) {
                win.active_path = path.clone();
                target_area = Some(*area);
                break;
            }
        }
        if target_area.is_none() {
            target_area = rects
                .iter()
                .find(|(path, _)| *path == win.active_path)
                .map(|(_, area)| *area);
        }

        let alt = active_pane(&win.root, &win.active_path)
            .map_or(false, |p| {
                if let Ok(parser) = p.term.lock() {
                    return parser.screen().alternate_screen();
                }
                false
            });
        let sgr_btn: u8 = if up { 64 } else { 65 };
        let wheel_delta: i16 = if up { 120 } else { -120 };
        let bs = ((wheel_delta as i32) << 16) as u32;
        (alt, target_area, sgr_btn, bs)
    };

    mouse_log(&format!("  -> alt_screen={}", child_in_alt_screen));

    if child_in_alt_screen {
        // Forward scroll to child TUI app (alternate screen = real TUI)
        mouse_log("  -> forwarding scroll to child TUI (alt screen)");
        let win = &mut app.windows[app.active_idx];
        let (col, row) = target_area_opt.map_or((0, 0), |area| pane_inner_cell_0based(area, x, y));
        let win_name = win.name.clone();
        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
            inject_mouse_combined(p, col, row, sgr_btn, true,
                button_state, mouse_inject::MOUSE_WHEELED, &win_name);
        }
    } else if up && app.scroll_enter_copy_mode {
        // Shell prompt — auto-enter copy mode and scroll up (tmux parity)
        mouse_log("  -> entering copy mode (shell scroll-up)");
        enter_copy_mode(app);
        scroll_copy_up(app, 3);
    } else if !app.scroll_enter_copy_mode {
        // scroll-enter-copy-mode off: scroll scrollback directly (#193)
        mouse_log("  -> direct scrollback (scroll-enter-copy-mode off)");
        scroll_pane_scrollback(app, 3, up);
    } else {
        mouse_log("  -> scroll-down at shell (no-op)");
    }
}

pub fn remote_scroll_up(app: &mut AppState, x: u16, y: u16) { remote_scroll_wheel(app, x, y, true); }

pub fn remote_scroll_down(app: &mut AppState, x: u16, y: u16) { remote_scroll_wheel(app, x, y, false); }
