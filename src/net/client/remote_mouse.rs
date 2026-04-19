use super::*;
use super::run_remote_state::RunRemoteState;

/// Handle all mouse events (down/drag/up/move/scroll).
pub(crate) fn handle_mouse_event(
    state: &mut RunRemoteState,
    me: &crossterm::event::MouseEvent,
    cmd_batch: &mut Vec<String>,
) {
    use crossterm::event::{MouseEventKind, MouseButton};
    match me.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Status bar tab click
            if me.row == state.client_status_row {
                let mut clicked_tab: Option<usize> = None;
                for &(win_idx, x_start, x_end) in &state.client_tab_positions {
                    if me.column >= x_start && me.column < x_end {
                        clicked_tab = Some(win_idx);
                        break;
                    }
                }
                if let Some(idx) = clicked_tab {
                    let display_idx = idx + state.client_base_index;
                    cmd_batch.push(format!("select-window -t :{}\n", display_idx));
                }
            } else {
                // Border detection
                let mut on_border = false;
                if !state.client_zoomed {
                    let tol = 0u16;
                    for (bpath, bkind, bidx, bpos, btotal, bsizes, barea) in &state.client_borders {
                        let hit = if bkind == "Horizontal" {
                            me.column >= bpos.saturating_sub(tol) && me.column <= bpos + tol
                            && me.row >= barea.y && me.row < barea.y + barea.height
                        } else {
                            me.row >= bpos.saturating_sub(tol) && me.row <= bpos + tol
                            && me.column >= barea.x && me.column < barea.x + barea.width
                        };
                        if hit {
                            state.client_drag = Some(ClientDragState {
                                path: bpath.clone(),
                                kind: bkind.clone(),
                                index: *bidx,
                                start_pos: if bkind == "Horizontal" { me.column } else { me.row },
                                initial_sizes: bsizes.clone(),
                                total_pixels: *btotal,
                            });
                            state.border_drag = true;
                            on_border = true;
                            state.rsel_start = None;
                            state.rsel_end = None;
                            state.selection_changed = true;
                            break;
                        }
                    }
                }

                if !on_border {
                    let clicked_pane = state.client_pane_rects.iter().find(|(_, rect)| {
                        rect.contains(ratatui::layout::Position { x: me.column, y: me.row })
                    });

                    if let Some(&(pane_id, pane_rect)) = clicked_pane {
                        cmd_batch.push(format!("select-pane -t %{}\n", pane_id));
                        let rel_col = me.column as i16 - pane_rect.x as i16;
                        let rel_row = me.row as i16 - pane_rect.y as i16;

                        if state.client_copy_mode {
                            cmd_batch.push(format!("pane-mouse {} 0 {} {} M\n",
                                pane_id, rel_col, rel_row));
                            state.rsel_start = None;
                            state.rsel_end = None;
                            state.rsel_pane_rect = None;
                            state.rsel_block = false;
                            state.selection_changed = true;
                        } else {
                            cmd_batch.push(format!("pane-mouse {} 0 {} {} M\n",
                                pane_id, rel_col, rel_row));
                            state.border_drag = false;

                            let ctrl_extend = state.client_pwsh_selection
                                && me.modifiers.contains(KeyModifiers::CONTROL)
                                && state.rsel_start.is_some()
                                && state.rsel_pane_rect == Some(pane_rect);

                            if ctrl_extend {
                                let r = pane_rect;
                                let col = me.column.clamp(r.x, r.x + r.width.saturating_sub(1));
                                let row = me.row.clamp(r.y, r.y + r.height.saturating_sub(1));
                                state.rsel_end = Some((col, row));
                                state.rsel_dragged = true;
                                state.selection_changed = true;
                            } else if state.client_pwsh_selection {
                                state.rsel_block = me.modifiers.contains(KeyModifiers::ALT);
                                state.rsel_pane_rect = Some(pane_rect);
                                state.rsel_dragged = false;
                                state.selection_changed = true;

                                let now = Instant::now();
                                let is_multi = state.last_click.map_or(false, |(t, (c, r))| {
                                    now.duration_since(t) < Duration::from_millis(400)
                                        && c == me.column && r == me.row
                                });
                                state.click_count = if is_multi { state.click_count + 1 } else { 1 };
                                state.last_click = Some((now, (me.column, me.row)));

                                let word = if state.click_count == 2 {
                                    serde_json::from_str::<DumpState>(&state.prev_dump_buf).ok()
                                        .and_then(|s| word_bounds_at(
                                            &s.layout,
                                            state.last_sent_size.0,
                                            state.last_sent_size.1,
                                            pane_rect,
                                            me.column, me.row,
                                        ))
                                } else {
                                    None
                                };

                                if let Some((ws, we)) = word {
                                    state.rsel_start = Some((ws, me.row));
                                    state.rsel_end = Some((we, me.row));
                                    state.rsel_dragged = true;
                                } else if state.click_count >= 3 {
                                    let left = pane_rect.x;
                                    let right = pane_rect.x + pane_rect.width.saturating_sub(1);
                                    state.rsel_start = Some((left, me.row));
                                    state.rsel_end = Some((right, me.row));
                                    state.rsel_dragged = true;
                                } else {
                                    state.rsel_start = Some((me.column, me.row));
                                    state.rsel_end = None;
                                }
                            } else {
                                // Legacy: start == end for 1-cell hint.
                                state.rsel_start = Some((me.column, me.row));
                                state.rsel_end = Some((me.column, me.row));
                                state.rsel_pane_rect = Some(pane_rect);
                                state.rsel_dragged = false;
                                state.selection_changed = true;
                            }
                        }
                    } else {
                        cmd_batch.push(format!("mouse-down {} {}\n", me.column, me.row));
                    }
                }
            }
        }
        MouseEventKind::Down(MouseButton::Right) => {
            let tui_active = if !state.prev_dump_buf.is_empty() {
                serde_json::from_str::<DumpState>(&state.prev_dump_buf)
                    .map(|s| active_pane_in_alt_screen(&s.layout))
                    .unwrap_or(false)
            } else { false };

            if tui_active {
                if let Some(&(pane_id, pane_rect)) = state.client_pane_rects.iter().find(|(_, r)| {
                    r.contains(ratatui::layout::Position { x: me.column, y: me.row })
                }) {
                    let rel_col = me.column as i16 - pane_rect.x as i16;
                    let rel_row = me.row as i16 - pane_rect.y as i16;
                    cmd_batch.push(format!("pane-mouse {} 2 {} {} M\n",
                        pane_id, rel_col, rel_row));
                }
                state.rsel_start = None;
                state.rsel_end = None;
                state.selection_changed = true;
            } else if state.rsel_start.is_some() && state.rsel_dragged {
                // pwsh-style: right-click with active selection -> copy + clear
                if let (Some(s), Some(e)) = (state.rsel_start, state.rsel_end) {
                    if let Ok(dump) = serde_json::from_str::<DumpState>(&state.prev_dump_buf) {
                        let text = extract_selection_text(
                            &dump.layout,
                            state.last_sent_size.0,
                            state.last_sent_size.1,
                            s, e,
                            state.rsel_block,
                        );
                        if !text.is_empty() {
                            copy_to_system_clipboard(&text);
                            state.pending_osc52 = Some(text);
                        }
                    }
                }
                state.rsel_start = None;
                state.rsel_end = None;
                state.rsel_pane_rect = None;
                state.rsel_block = false;
                state.rsel_dragged = false;
                state.selection_changed = true;
                #[cfg(windows)]
                {
                    state.paste_suppress_until = Some(Instant::now() + Duration::from_millis(200));
                }
            } else {
                // No selection, no TUI  -> paste from clipboard (pwsh-style)
                state.rsel_start = None;
                state.rsel_end = None;
                state.selection_changed = true;
                if let Some(text) = read_from_system_clipboard() {
                    if !text.is_empty() {
                        let encoded = base64_encode(&text);
                        cmd_batch.push(format!("send-paste {}\n", encoded));
                    }
                }
            }
        }
        MouseEventKind::Down(MouseButton::Middle) => {
            if let Some(&(pane_id, pane_rect)) = state.client_pane_rects.iter().find(|(_, r)| {
                r.contains(ratatui::layout::Position { x: me.column, y: me.row })
            }) {
                let rel_col = me.column as i16 - pane_rect.x as i16;
                let rel_row = me.row as i16 - pane_rect.y as i16;
                cmd_batch.push(format!("pane-mouse {} 1 {} {} M\n",
                    pane_id, rel_col, rel_row));
            } else {
                cmd_batch.push(format!("mouse-down-middle {} {}\n", me.column, me.row));
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if state.border_drag {
                if let Some(ref d) = state.client_drag {
                    let current_pos = if d.kind == "Horizontal" { me.column } else { me.row };
                    let pixel_delta = current_pos as i32 - d.start_pos as i32;
                    let total_pct: i32 = d.initial_sizes.iter().map(|&s| s as i32).sum();
                    let total_px = d.total_pixels.max(1) as i32;
                    let pct_delta = (pixel_delta * total_pct) / total_px;
                    let min_pct = 5i32;

                    let mut new_sizes = d.initial_sizes.clone();
                    let left = (d.initial_sizes[d.index] as i32 + pct_delta)
                        .clamp(min_pct, d.initial_sizes[d.index] as i32 + d.initial_sizes[d.index + 1] as i32 - min_pct) as u16;
                    let right = d.initial_sizes[d.index] + d.initial_sizes[d.index + 1] - left;
                    new_sizes[d.index] = left;
                    new_sizes[d.index + 1] = right;

                    let path_str = if d.path.is_empty() { "_".to_string() } else { d.path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(".") };
                    let sizes_str = new_sizes.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(",");
                    cmd_batch.push(format!("split-sizes {} {}\n", path_str, sizes_str));
                }
            } else if state.rsel_start.is_none() {
                if state.client_copy_mode {
                    if let Some(&(pane_id, pane_rect)) = state.client_pane_rects.iter().find(|(_, r)| {
                        r.contains(ratatui::layout::Position { x: me.column, y: me.row })
                    }) {
                        let rel_col = me.column as i16 - pane_rect.x as i16;
                        let rel_row = me.row as i16 - pane_rect.y as i16;
                        cmd_batch.push(format!("pane-mouse {} 32 {} {} M\n",
                            pane_id, rel_col, rel_row));
                    }
                } else {
                    cmd_batch.push(format!("mouse-drag {} {}\n", me.column, me.row));
                }
            } else {
                if let Some(start) = state.rsel_start {
                    let (col, row) = if state.client_pwsh_selection {
                        if let Some(r) = state.rsel_pane_rect {
                            (
                                me.column.clamp(r.x, r.x + r.width.saturating_sub(1)),
                                me.row.clamp(r.y, r.y + r.height.saturating_sub(1)),
                            )
                        } else {
                            (me.column, me.row)
                        }
                    } else {
                        (me.column, me.row)
                    };
                    if (col, row) == start && !state.rsel_dragged {
                        // no-op: micro-drag on initial cell
                    } else {
                        state.rsel_end = Some((col, row));
                        state.rsel_dragged = true;
                        state.selection_changed = true;
                    }
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Right) => {}
        MouseEventKind::Up(MouseButton::Left) => {
            if state.border_drag {
                cmd_batch.push("split-resize-done\n".to_string());
                state.border_drag = false;
                state.client_drag = None;
            } else if state.rsel_dragged {
                if state.client_pwsh_selection {
                    state.selection_changed = true;
                } else {
                    // Legacy: copy-on-release.
                    state.rsel_end = Some((me.column, me.row));
                    if let (Some(s), Some(e)) = (state.rsel_start, state.rsel_end) {
                        if let Ok(dump) = serde_json::from_str::<DumpState>(&state.prev_dump_buf) {
                            let text = extract_selection_text(
                                &dump.layout,
                                state.last_sent_size.0,
                                state.last_sent_size.1,
                                s, e,
                                false,
                            );
                            if !text.is_empty() {
                                copy_to_system_clipboard(&text);
                                state.pending_osc52 = Some(text);
                            }
                        }
                    }
                    state.rsel_start = None;
                    state.rsel_end = None;
                    state.rsel_pane_rect = None;
                    state.rsel_block = false;
                    state.rsel_dragged = false;
                    state.selection_changed = true;
                }
            } else {
                state.rsel_start = None;
                state.rsel_end = None;
                state.rsel_pane_rect = None;
                state.rsel_block = false;
                state.selection_changed = true;
                if state.client_copy_mode {
                    if let Some(&(pane_id, pane_rect)) = state.client_pane_rects.iter().find(|(_, r)| {
                        r.contains(ratatui::layout::Position { x: me.column, y: me.row })
                    }) {
                        let rel_col = me.column as i16 - pane_rect.x as i16;
                        let rel_row = me.row as i16 - pane_rect.y as i16;
                        cmd_batch.push(format!("pane-mouse {} 0 {} {} m\n",
                            pane_id, rel_col, rel_row));
                    }
                } else {
                    cmd_batch.push(format!("mouse-up {} {}\n", me.column, me.row));
                }
            }
        }
        MouseEventKind::Up(MouseButton::Right) => {}
        MouseEventKind::Up(MouseButton::Middle) => {}
        MouseEventKind::Moved => {
            // Detect border hover for visual preview
            let mut new_hover: Option<(u16, String, Rect)> = None;
            if !state.client_zoomed {
                let tol = 0u16;
                for (_, bkind, _, bpos, _, _, barea) in &state.client_borders {
                    let hit = if bkind == "Horizontal" {
                        me.column >= bpos.saturating_sub(tol) && me.column <= bpos + tol
                        && me.row >= barea.y && me.row < barea.y + barea.height
                    } else {
                        me.row >= bpos.saturating_sub(tol) && me.row <= bpos + tol
                        && me.column >= barea.x && me.column < barea.x + barea.width
                    };
                    if hit {
                        new_hover = Some((*bpos, bkind.clone(), *barea));
                        break;
                    }
                }
            }
            if new_hover != state.hovered_border {
                state.hovered_border = new_hover;
                state.selection_changed = true;
            }
            // Forward hover to PTY
            if let Some(&(pane_id, pane_rect)) = state.client_pane_rects.iter().find(|(_, r)| {
                r.contains(ratatui::layout::Position { x: me.column, y: me.row })
            }) {
                let rel_col = me.column as i16 - pane_rect.x as i16;
                let rel_row = me.row as i16 - pane_rect.y as i16;
                cmd_batch.push(format!("pane-mouse {} 35 {} {} M\n",
                    pane_id, rel_col, rel_row));
            } else {
                cmd_batch.push(format!("mouse-move {} {}\n", me.column, me.row));
            }
        }
        MouseEventKind::ScrollUp => {
            state.rsel_start = None;
            state.rsel_end = None;
            state.rsel_dragged = false;
            state.selection_changed = true;
            if let Some(&(pane_id, _)) = state.client_pane_rects.iter().find(|(_, r)| {
                r.contains(ratatui::layout::Position { x: me.column, y: me.row })
            }) {
                cmd_batch.push(format!("pane-scroll {} up\n", pane_id));
            } else {
                cmd_batch.push(format!("scroll-up {} {}\n", me.column, me.row));
            }
        }
        MouseEventKind::ScrollDown => {
            state.rsel_start = None;
            state.rsel_end = None;
            state.rsel_dragged = false;
            state.selection_changed = true;
            if let Some(&(pane_id, _)) = state.client_pane_rects.iter().find(|(_, r)| {
                r.contains(ratatui::layout::Position { x: me.column, y: me.row })
            }) {
                cmd_batch.push(format!("pane-scroll {} down\n", pane_id));
            } else {
                cmd_batch.push(format!("scroll-down {} {}\n", me.column, me.row));
            }
        }
        _ => {}
    }
}
