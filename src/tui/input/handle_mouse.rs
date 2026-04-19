#[allow(unused_imports)]
use std::io::{self, Write};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use portable_pty::native_pty_system;
use ratatui::prelude::*;

use crate::types::{AppState, Mode, FocusDir, LayoutKind, DragState, Node, Pane};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, path_exists, resize_all_panes};
use crate::pane::{create_window, split_active};
use crate::commands::{execute_action, execute_command_prompt, execute_command_string};
use crate::config::normalize_key_for_binding;
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, switch_with_copy_save, move_copy_cursor,
    scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, paste_latest, yank_selection,
    search_copy_mode, search_next, search_prev, scroll_to_top, scroll_to_bottom};
use crate::layout::{cycle_top_layout, apply_layout};
use crate::window_ops::{toggle_zoom, swap_pane, break_pane_to_window};

/// Write a mouse event to the child PTY using the encoding the child requested.
use super::*;

pub fn handle_mouse(app: &mut AppState, me: MouseEvent, window_area: Rect) -> io::Result<()> {
    use crossterm::event::{MouseEventKind, MouseButton};

    // Track last mouse position for #{mouse_x}, #{mouse_y} format variables
    app.last_mouse_x = me.column;
    app.last_mouse_y = me.row;

    // --- MenuMode: handle mouse clicks on menu items ---
    if let Mode::MenuMode { ref mut menu } = app.mode {
        if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) {
            // Recompute menu_area the same way as the renderer (app.rs).
            let full_area = Rect {
                x: 0, y: 0,
                width: window_area.width,
                height: window_area.height + app.status_lines as u16,
            };
            let item_count = menu.items.len();
            let height = (item_count as u16 + 2).min(20);
            let width = menu.items.iter().map(|i| i.name.len()).max().unwrap_or(10).max(menu.title.len()) as u16 + 8;
            let menu_area = if let (Some(x), Some(y)) = (menu.x, menu.y) {
                let x = if x < 0 { (full_area.width as i16 + x).max(0) as u16 } else { x as u16 };
                let y = if y < 0 { (full_area.height as i16 + y).max(0) as u16 } else { y as u16 };
                Rect { x: x.min(full_area.width.saturating_sub(width)), y: y.min(full_area.height.saturating_sub(height)), width, height }
            } else {
                crate::rendering::centered_rect((width * 100 / full_area.width.max(1)).max(30), height, full_area)
            };
            let pos = ratatui::layout::Position { x: me.column, y: me.row };
            if menu_area.contains(pos) {
                // Block border is 1 row top
                let inner_y = me.row.saturating_sub(menu_area.y + 1);
                let idx = inner_y as usize;
                if idx < menu.items.len() && !menu.items[idx].is_separator && !menu.items[idx].command.is_empty() {
                    let cmd = menu.items[idx].command.clone();
                    app.mode = Mode::Passthrough;
                    let _ = execute_command_string(app, &cmd);
                } else {
                    app.mode = Mode::Passthrough;
                }
            } else {
                app.mode = Mode::Passthrough;
            }
            return Ok(());
        }
        if matches!(me.kind, MouseEventKind::ScrollUp) {
            if menu.selected > 0 {
                menu.selected -= 1;
                while menu.selected > 0 && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                    menu.selected -= 1;
                }
            }
            return Ok(());
        }
        if matches!(me.kind, MouseEventKind::ScrollDown) {
            if menu.selected + 1 < menu.items.len() {
                menu.selected += 1;
                while menu.selected + 1 < menu.items.len() && menu.items.get(menu.selected).map(|i| i.is_separator).unwrap_or(false) {
                    menu.selected += 1;
                }
            }
            return Ok(());
        }
        return Ok(());
    }

    // Customize mode: absorb all mouse events
    if matches!(app.mode, Mode::CustomizeMode { .. }) {
        return Ok(());
    }

    // --- Tab click: check if click is on the status bar row ---
    let status_row = window_area.y + window_area.height; // status bar is 1 row below window area
    if matches!(me.kind, MouseEventKind::Down(MouseButton::Left)) && me.row == status_row {
        for &(win_idx, x_start, x_end) in app.tab_positions.iter() {
            if me.column >= x_start && me.column < x_end {
                if win_idx < app.windows.len() {
                    switch_with_copy_save(app, |app| {
                        app.last_window_idx = app.active_idx;
                        app.active_idx = win_idx;
                    });
                }
                return Ok(());
            }
        }
        // Click was on status bar but not on a tab — ignore
        return Ok(());
    }

    // If a left-click lands on a different pane while in copy mode,
    // exit copy mode entirely and switch to the clicked pane (tmux parity #62).
    if matches!(me.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left))
        && matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. })
    {
        let win = &app.windows[app.active_idx];
        let mut rects_check: Vec<(Vec<usize>, Rect)> = Vec::new();
        compute_rects(&win.root, window_area, &mut rects_check);
        let mut clicked_new_path: Option<Vec<usize>> = None;
        for (path, area) in rects_check.iter() {
            if area.contains(ratatui::layout::Position { x: me.column, y: me.row }) {
                if *path != win.active_path {
                    clicked_new_path = Some(path.clone());
                }
                break;
            }
        }
        if let Some(np) = clicked_new_path {
            // Exit copy mode cleanly (resets scroll, clears selection)
            exit_copy_mode(app);
            // Switch active pane path
            {
                let win = &mut app.windows[app.active_idx];
                app.last_pane_path = win.active_path.clone();
                win.active_path = np;
            }
        }
    }

    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, window_area, &mut rects);
    let mut borders: Vec<(Vec<usize>, LayoutKind, usize, u16, u16)> = Vec::new();
    compute_split_borders(&win.root, window_area, &mut borders);
    let mut active_area = rects
        .iter()
        .find(|(path, _)| *path == win.active_path)
        .map(|(_, area)| *area);

    // Helper: convert absolute screen coordinates to 0-based pane-local
    // (row, col) for copy-mode cursor positioning.  Mirrors
    // `copy_cell_for_area` in window_ops.rs.
    fn copy_cell(area: Rect, abs_x: u16, abs_y: u16) -> (u16, u16) {
        let col = abs_x.saturating_sub(area.x).min(area.width.saturating_sub(1));
        let row = abs_y.saturating_sub(area.y).min(area.height.saturating_sub(1));
        (row, col)
    }

    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });

    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // ── Copy-mode: left click positions cursor, clears selection ──
            // tmux parity: single click moves cursor without starting a selection.
            // Selection only starts when dragging (see Drag handler below).
            if in_copy {
                app.copy_anchor = None;
                if let Some(area) = active_area {
                    let (row, col) = copy_cell(area, me.column, me.row);
                    app.copy_pos = Some((row, col));
                    app.copy_mouse_down_cell = Some((row, col));
                }
                return Ok(());
            }

            // Check if click is on a split border (for dragging)
            let mut on_border = false;
            let tol = 1u16;
            for (path, kind, idx, pos, total_px) in borders.iter() {
                match kind {
                    LayoutKind::Horizontal => {
                        if me.column >= pos.saturating_sub(tol) && me.column <= pos + tol {
                            if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) {
                                app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: *pos, start_y: me.row, left_initial: left, _right_initial: right, total_pixels: *total_px });
                            }
                            on_border = true;
                            break;
                        }
                    }
                    LayoutKind::Vertical => {
                        if me.row >= pos.saturating_sub(tol) && me.row <= pos + tol {
                            if let Some((left,right)) = split_sizes_at(&win.root, path.clone(), *idx) {
                                app.drag = Some(DragState { split_path: path.clone(), kind: *kind, index: *idx, start_x: me.column, start_y: *pos, left_initial: left, _right_initial: right, total_pixels: *total_px });
                            }
                            on_border = true;
                            break;
                        }
                    }
                }
            }

            // Switch pane focus if clicking inside a pane
            for (path, area) in rects.iter() {
                if area.contains(ratatui::layout::Position { x: me.column, y: me.row }) {
                    win.active_path = path.clone();
                    // Update MRU for clicked pane
                    if let Some(pid) = crate::tree::get_active_pane_id(&win.root, path) {
                        crate::tree::touch_mru(&mut win.pane_mru, pid);
                    }
                    active_area = Some(*area);
                }
            }

            // Forward left-click only when active pane wants mouse input.
            if !on_border {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if crate::window_ops::pane_wants_mouse(active) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row,
                                crate::platform::mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED, 0,
                                0, true); // SGR button 0 = left, press
                        }
                    }
                }
            }

        }
        MouseEventKind::Down(MouseButton::Right) => {
            // Windows Terminal behaviour: right-click = paste clipboard.
            // When the child has mouse tracking enabled (TUI app), forward
            // the right-click to the app instead.
            if in_copy {
                // In copy mode: paste clipboard (like Windows Terminal)
                let _ = paste_clipboard_to_active(app);
                return Ok(());
            }
            // Forward right-click only when active pane wants mouse input.
            let wants_mouse = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| crate::window_ops::pane_wants_mouse(p));
            if wants_mouse {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if crate::window_ops::pane_wants_mouse(active) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row,
                                crate::platform::mouse_inject::RIGHTMOST_BUTTON_PRESSED, 0,
                                2, true); // SGR button 2 = right, press
                        }
                    }
                }
            } else {
                // Shell prompt — paste clipboard (Windows Terminal parity)
                let _ = paste_clipboard_to_active(app);
                return Ok(());
            }
        }
        MouseEventKind::Down(MouseButton::Middle) => {
            // In copy mode, suppress — don't forward to child
            if in_copy { return Ok(()); }
            if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            crate::platform::mouse_inject::FROM_LEFT_2ND_BUTTON_PRESSED, 0,
                            1, true); // SGR button 1 = middle, press
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // ── Copy-mode: left release finalises position, auto-yank if selection ──
            if in_copy {
                if let Some(area) = active_area {
                    let (row, col) = copy_cell(area, me.column, me.row);
                    app.copy_pos = Some((row, col));
                }
                // If mouse-up is within 1 cell of mouse-down, it was a plain click
                // (any anchor set by jittery drag events is spurious). Clear it. (#199)
                let click_origin = app.copy_mouse_down_cell.take();
                if let (Some((dr, dc)), Some((ur, uc))) = (click_origin, app.copy_pos) {
                    if (dr as i32 - ur as i32).unsigned_abs() <= 1
                        && (dc as i32 - uc as i32).unsigned_abs() <= 1
                    {
                        app.copy_anchor = None;
                        app.copy_pos = Some((dr, dc)); // snap to original click position
                        return Ok(());
                    }
                }
                // Auto-yank if there is a selection (anchor != pos) — tmux parity
                if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                    if a != p {
                        let _ = yank_selection(app);
                        // tmux parity #62: auto-exit copy mode after mouse yank
                        exit_copy_mode(app);
                    } else {
                        // Click without real drag: clear stale anchor so scrolling
                        // does not produce a phantom selection (#199).
                        app.copy_anchor = None;
                    }
                }
                return Ok(());
            }

            let was_dragging = app.drag.is_some();
            app.drag = None;
            if was_dragging {
                resize_all_panes(app);
            } else if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row, 0, 0,
                            0, false); // SGR button 0 = left, release
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Right) => {
            if in_copy { return Ok(()); }
            // Forward right-release only when active pane wants mouse input.
            let wants_mouse = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| crate::window_ops::pane_wants_mouse(p));
            if wants_mouse {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        if crate::window_ops::pane_wants_mouse(active) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row, 0, 0,
                                2, false); // SGR button 2 = right, release
                        }
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Middle) => {
            if in_copy { return Ok(()); }
            if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row, 0, 0,
                            1, false); // SGR button 1 = middle, release
                    }
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left)
        | MouseEventKind::Moved
        | MouseEventKind::ScrollUp
        | MouseEventKind::ScrollDown => {
            return handle_mouse_drag_and_scroll(app, me, window_area);
        }
        _ => {}
    }
    Ok(())
}
