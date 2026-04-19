use super::*;
use super::run_remote_state::RunRemoteState;
use super::run_remote_types::*;

/// Render the client-side text selection overlay (left-click drag).
pub(crate) fn render_selection_overlay(
    f: &mut Frame,
    root: &LayoutJson,
    area: Rect,
    sel_s: Option<(u16, u16)>,
    sel_e: Option<(u16, u16)>,
    sel_rect: Option<Rect>,
    sel_pwsh: bool,
    sel_block: bool,
) {
    if let (Some(s), Some(e)) = (sel_s, sel_e) {
        if !active_pane_in_copy_mode(root) {
            let (r0, c0, r1, c1) = normalize_selection(s, e, sel_block);
            let (pane_left, pane_right) = if sel_pwsh {
                if let Some(r) = sel_rect {
                    (r.x, r.x + r.width.saturating_sub(1))
                } else {
                    (0, area.width.saturating_sub(1))
                }
            } else {
                (0, area.width.saturating_sub(1))
            };
            let buf = f.buffer_mut();
            let buf_area = buf.area;
            for row in r0..=r1 {
                let col_start = if sel_block {
                    c0.max(pane_left)
                } else if row == r0 { c0.max(pane_left) } else { pane_left };
                let col_end = if sel_block {
                    c1.min(pane_right)
                } else if row == r1 { c1.min(pane_right) } else { pane_right };
                if col_start > col_end { continue; }
                for col in col_start..=col_end {
                    if row < buf_area.height && col < buf_area.width {
                        let idx = (row - buf_area.y) as usize * buf_area.width as usize
                            + (col - buf_area.x) as usize;
                        if idx < buf.content.len() {
                            let style = if sel_pwsh {
                                Style::default().fg(Color::Black).bg(Color::White)
                            } else {
                                Style::default().fg(Color::Black).bg(Color::LightCyan)
                            };
                            buf.content[idx].set_style(style);
                        }
                    }
                }
            }
        }
    }
}

/// Render all chooser overlays (session, tree, buffer, keys viewer).
pub(crate) fn render_chooser_overlays(
    f: &mut Frame,
    state: &mut RunRemoteState,
    content_chunk: Rect,
    current_session: &str,
) {
    let mode_style = &state.mode_style_str;

    if state.session_chooser {
        let sel_style = crate::rendering::parse_tmux_style(mode_style);
        let overlay = Block::default().borders(Borders::ALL).title("choose-session (enter=switch, x=kill, esc=close)").border_style(sel_style);
        let sess_h = ((state.session_entries.len() as u16).saturating_add(2))
            .max(5)
            .min(content_chunk.height.saturating_sub(2));
        let oa = centered_rect(70, sess_h, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let visible_h = inner.height as usize;
        if state.session_selected >= state.session_scroll + visible_h {
            state.session_scroll = state.session_selected.saturating_sub(visible_h - 1);
        }
        if state.session_selected < state.session_scroll {
            state.session_scroll = state.session_selected;
        }
        let mut lines: Vec<Line> = Vec::new();
        for (i, (sname, info)) in state.session_entries.iter().enumerate().skip(state.session_scroll).take(visible_h) {
            let marker = if sname == current_session { "*" } else { " " };
            let line = if i == state.session_selected {
                Line::from(Span::styled(format!("{} {}", marker, info), sel_style))
            } else {
                Line::from(format!("{} {}", marker, info))
            };
            lines.push(line);
        }
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
        render_scroll_indicator(f, oa, state.session_entries.len(), visible_h, state.session_scroll);
    }

    if state.tree_chooser {
        let sel_style = crate::rendering::parse_tmux_style(mode_style);
        let overlay = Block::default().borders(Borders::ALL).title("choose-tree").border_style(sel_style);
        let tree_h = ((state.tree_entries.len() as u16).saturating_add(2))
            .max(5)
            .min(content_chunk.height.saturating_sub(2));
        let oa = centered_rect(60, tree_h, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let visible_h = inner.height as usize;
        if state.tree_selected >= state.tree_scroll + visible_h {
            state.tree_scroll = state.tree_selected.saturating_sub(visible_h - 1);
        }
        if state.tree_selected < state.tree_scroll {
            state.tree_scroll = state.tree_selected;
        }
        let mut lines: Vec<Line> = Vec::new();
        for (i, (is_win, wid, _pid, label, _sess)) in state.tree_entries.iter().enumerate().skip(state.tree_scroll).take(visible_h) {
            let line = if i == state.tree_selected {
                Line::from(Span::styled(label.clone(), sel_style))
            } else if *is_win && *wid == usize::MAX {
                Line::from(Span::styled(label.clone(), Style::default().add_modifier(Modifier::BOLD)))
            } else {
                Line::from(label.clone())
            };
            lines.push(line);
        }
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
        render_scroll_indicator(f, oa, state.tree_entries.len(), visible_h, state.tree_scroll);
    }

    if state.buffer_chooser {
        let sel_style = crate::rendering::parse_tmux_style(mode_style);
        let overlay = Block::default().borders(Borders::ALL)
            .title(" choose-buffer (Enter=paste, d=delete, q/Esc=close) ")
            .border_style(sel_style);
        let buf_h = ((state.buffer_entries.len() as u16).saturating_add(2))
            .max(5)
            .min(content_chunk.height.saturating_sub(2));
        let oa = centered_rect(70, buf_h, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let visible_h = inner.height as usize;
        if state.buffer_selected >= state.buffer_scroll + visible_h {
            state.buffer_scroll = state.buffer_selected.saturating_sub(visible_h - 1);
        }
        if state.buffer_selected < state.buffer_scroll {
            state.buffer_scroll = state.buffer_selected;
        }
        let mut lines: Vec<Line> = Vec::new();
        for (i, (idx, byte_len, preview)) in state.buffer_entries.iter().enumerate().skip(state.buffer_scroll).take(visible_h) {
            let label = format!("buffer{}: {} bytes: \"{}\"", idx, byte_len, preview);
            let line = if i == state.buffer_selected {
                Line::from(Span::styled(label, sel_style))
            } else {
                Line::from(label)
            };
            lines.push(line);
        }
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
        render_scroll_indicator(f, oa, state.buffer_entries.len(), visible_h, state.buffer_scroll);
    }

    if state.keys_viewer {
        let avail_h = content_chunk.height;
        let overlay_h = (avail_h * 80 / 100).max(5).min(avail_h.saturating_sub(2));
        let overlay = Block::default().borders(Borders::ALL)
            .title(" list-keys (q/Esc=close, Up/Down/PgUp/PgDn=scroll) ");
        let oa = centered_rect(90, overlay_h, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let visible_h = inner.height as usize;
        let max_scroll = state.keys_viewer_lines.len().saturating_sub(visible_h);
        if state.keys_viewer_scroll > max_scroll { state.keys_viewer_scroll = max_scroll; }
        let mut lines: Vec<Line> = Vec::new();
        for (_i, entry) in state.keys_viewer_lines.iter().enumerate().skip(state.keys_viewer_scroll).take(visible_h) {
            if entry.starts_with("\u{2500}\u{2500}") || entry.starts_with("\u{2500}\u{2500} ") {
                lines.push(Line::from(Span::styled(entry.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
            } else if let Some(rest) = entry.strip_prefix("bind-key") {
                lines.push(Line::from(vec![
                    Span::styled("bind-key", Style::default().fg(Color::Green)),
                    Span::raw(rest.to_string()),
                ]));
            } else {
                lines.push(Line::from(entry.clone()));
            }
        }
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
        render_scroll_indicator(f, oa, state.keys_viewer_lines.len(), visible_h, state.keys_viewer_scroll);
    }
}
/// Render client-side input overlays (rename, command, confirm, window index).
pub(crate) fn render_input_overlays(
    f: &mut Frame,
    state: &RunRemoteState,
    content_chunk: Rect,
) {
    if state.renaming {
        let title = if state.session_renaming { "rename session" } else { "rename window" };
        let overlay = Block::default().borders(Borders::ALL).title(title);
        let oa = centered_rect(60, 3, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let para = Paragraph::new(format!("name: {}", state.rename_buf));
        f.render_widget(para, overlay.inner(oa));
    }
    if state.pane_renaming {
        let overlay = Block::default().borders(Borders::ALL).title("set pane title");
        let oa = centered_rect(60, 3, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let para = Paragraph::new(format!("title: {}", state.pane_title_buf));
        f.render_widget(para, overlay.inner(oa));
    }
    if state.command_input {
        let overlay = Block::default().borders(Borders::ALL).title("command");
        let oa = centered_rect(60, 3, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let para = Paragraph::new(format!(": {}", state.command_buf));
        f.render_widget(para, inner);
        let cx = inner.x + 2 + state.command_cursor as u16;
        f.set_cursor_position((cx, inner.y));
    }
    if state.window_idx_input {
        let overlay = Block::default().borders(Borders::ALL).title("select window");
        let oa = centered_rect(50, 3, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let para = Paragraph::new(format!("index: {}", state.window_idx_buf));
        f.render_widget(para, inner);
        let cx = inner.x + 7 + state.window_idx_buf.len() as u16;
        f.set_cursor_position((cx, inner.y));
    }
    if let Some(ref cmd) = state.confirm_cmd {
        let overlay = Block::default().borders(Borders::ALL).title("confirm");
        let oa = centered_rect(50, 3, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let para = Paragraph::new(format!("{}? (y/n)", cmd));
        f.render_widget(para, overlay.inner(oa));
    }
}


/// Render a scroll position indicator in the bottom-right of an overlay area.
fn render_scroll_indicator(f: &mut Frame, oa: Rect, total: usize, visible_h: usize, scroll: usize) {
    if total > visible_h {
        let max_scroll = total.saturating_sub(visible_h);
        let pct = if max_scroll > 0 { scroll * 100 / max_scroll } else { 0 };
        let indicator = if scroll == 0 {
            "Top".to_string()
        } else if scroll >= max_scroll {
            "Bot".to_string()
        } else {
            format!("{}%", pct)
        };
        let ind_len = indicator.len() as u16;
        if oa.width > ind_len + 2 {
            let ind_x = oa.x + oa.width - ind_len - 2;
            let ind_y = oa.y + oa.height - 1;
            let ind_rect = Rect::new(ind_x, ind_y, ind_len, 1);
            let ind_para = Paragraph::new(Span::styled(indicator, Style::default().fg(Color::DarkGray)));
            f.render_widget(ind_para, ind_rect);
        }
    }
}
