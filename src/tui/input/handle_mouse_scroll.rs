#[allow(unused_imports)]
use super::*;

/// Handle mouse drag, motion, and scroll events.
/// Extracted from handle_mouse for the Drag(Left), Moved, ScrollUp, ScrollDown arms.
pub(crate) fn handle_mouse_drag_and_scroll(app: &mut AppState, me: MouseEvent, window_area: Rect) -> io::Result<()> {
    use crossterm::event::{MouseEventKind, MouseButton};

    let in_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });

    fn copy_cell(area: Rect, abs_x: u16, abs_y: u16) -> (u16, u16) {
        let col = abs_x.saturating_sub(area.x).min(area.width.saturating_sub(1));
        let row = abs_y.saturating_sub(area.y).min(area.height.saturating_sub(1));
        (row, col)
    }

    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, window_area, &mut rects);
    let mut active_area = rects
        .iter()
        .find(|(path, _)| *path == win.active_path)
        .map(|(_, area)| *area);

    match me.kind {
        MouseEventKind::Drag(MouseButton::Left) => {
            // ── Copy-mode: drag extends the selection ──
            if in_copy {
                if let Some(area) = active_area {
                    let (row, col) = copy_cell(area, me.column, me.row);
                    if app.copy_anchor.is_none() {
                        // Only start a selection when the mouse actually moves
                        // to a different cell than the click position.  This
                        // prevents micro-drags (sub-cell jitter) from setting a
                        // stale anchor that produces phantom selections (#199).
                        if app.copy_pos == Some((row, col)) {
                            return Ok(());
                        }
                        app.copy_anchor = Some(app.copy_pos.unwrap_or((row, col)));
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_selection_mode = crate::types::SelectionMode::Char;
                    }
                    app.copy_pos = Some((row, col));
                    // tmux parity #62: auto-scroll when dragging at pane edges
                    if me.row <= area.y {
                        scroll_copy_up(app, 1);
                    } else if me.row >= area.y + area.height.saturating_sub(1) {
                        scroll_copy_down(app, 1);
                    }
                }
                return Ok(());
            }

            if let Some(d) = &app.drag {
                adjust_split_sizes(&mut win.root, d, me.column, me.row);
            } else {
                // tmux parity #62: drag from normal mode enters copy mode
                // and starts selection (when child doesn't want mouse).
                let wants_mouse = {
                    let win2 = &app.windows[app.active_idx];
                    active_pane(&win2.root, &win2.active_path)
                        .map_or(false, |p| crate::window_ops::pane_wants_mouse(p))
                };
                if wants_mouse {
                    if let Some(area) = active_area {
                        let win2 = &mut app.windows[app.active_idx];
                        if let Some(active) = active_pane_mut(&mut win2.root, &win2.active_path) {
                            forward_mouse_to_pane_ex(active, area, me.column, me.row,
                                crate::platform::mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED,
                                crate::platform::mouse_inject::MOUSE_MOVED,
                                32, true); // SGR button 32 = left-drag
                        }
                    }
                } else {
                    // Shell prompt: enter copy mode, start selection
                    enter_copy_mode(app);
                    if let Some(area) = active_area {
                        let (row, col) = copy_cell(area, me.column, me.row);
                        app.copy_anchor = Some((row, col));
                        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                        app.copy_selection_mode = crate::types::SelectionMode::Char;
                        app.copy_pos = Some((row, col));
                    }
                }
            }
        }
        MouseEventKind::Moved => {
            // Forward bare mouse motion (hover) only when active pane
            // explicitly wants mouse input. This avoids sending raw
            // SGR motion bytes (ESC[<35;...) to shell-like prompts.
            if app.last_hover_pos == Some((me.column, me.row)) {
                return Ok(());
            }
            app.last_hover_pos = Some((me.column, me.row));

            if let Some(area) = active_area {
                if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                    if crate::window_ops::pane_wants_mouse(active) {
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            0, crate::platform::mouse_inject::MOUSE_MOVED,
                            35, true);
                    }
                }
            }
        }
        MouseEventKind::ScrollUp => {
            // Ignore scroll in popup mode — don't enter copy-mode (#110)
            if matches!(app.mode, Mode::PopupMode { .. }) {
                return Ok(());
            }
            if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
                scroll_copy_up(app, 3);
                return Ok(());
            }
            if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x: me.column, y: me.row })) {
                win.active_path = path.clone();
                active_area = Some(*area);
            }
            // tmux parity: Only forward scroll to child if alternate screen
            // is active (real TUI app like nvim/htop).  If not (shell prompt),
            // auto-enter copy mode.
            let child_in_alt = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| {
                    if let Ok(parser) = p.term.lock() {
                        return parser.screen().alternate_screen();
                    }
                    false
                });
            if child_in_alt {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        let wheel_delta: i16 = 120;
                        let button_state = ((wheel_delta as i32) << 16) as u32;
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            button_state, crate::platform::mouse_inject::MOUSE_WHEELED,
                            64, true); // SGR button 64 = scroll-up
                    }
                }
            } else if app.scroll_enter_copy_mode {
                // Shell prompt — auto-enter copy mode and scroll (tmux parity)
                enter_copy_mode(app);
                scroll_copy_up(app, 3);
                return Ok(());
            } else {
                scroll_pane_scrollback(app, 3, true);
            }
        }
        MouseEventKind::ScrollDown => {
            // Ignore scroll in popup mode — don't enter copy-mode (#110)
            if matches!(app.mode, Mode::PopupMode { .. }) {
                return Ok(());
            }
            if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
                scroll_copy_down(app, 3);
                // Auto-exit copy mode when scrolled back to live output
                // (only when no active selection, to avoid losing a selection in progress)
                if app.copy_scroll_offset == 0 && app.copy_anchor.is_none() {
                    exit_copy_mode(app);
                }
                return Ok(());
            }
            if let Some((path, area)) = rects.iter().find(|(_, area)| area.contains(ratatui::layout::Position { x: me.column, y: me.row })) {
                win.active_path = path.clone();
                active_area = Some(*area);
            }
            // Forward scroll-down to child only if alternate screen is active
            let child_in_alt = active_pane(&win.root, &win.active_path)
                .map_or(false, |p| {
                    if let Ok(parser) = p.term.lock() {
                        return parser.screen().alternate_screen();
                    }
                    false
                });
            if child_in_alt {
                if let Some(area) = active_area {
                    if let Some(active) = active_pane_mut(&mut win.root, &win.active_path) {
                        let wheel_delta: i16 = -120;
                        let button_state = ((wheel_delta as i32) << 16) as u32;
                        forward_mouse_to_pane_ex(active, area, me.column, me.row,
                            button_state, crate::platform::mouse_inject::MOUSE_WHEELED,
                            65, true); // SGR button 65 = scroll-down
                    }
                }
            } else if !app.scroll_enter_copy_mode {
                scroll_pane_scrollback(app, 3, false);
            }
        }
        _ => {}
    }
    Ok(())
}
