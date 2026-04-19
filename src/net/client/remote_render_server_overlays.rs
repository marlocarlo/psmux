use super::*;
use super::run_remote_state::RunRemoteState;
use super::run_remote_types::*;

/// Render server-side overlays (popup, confirm, menu, customize, display-panes).
pub(crate) fn render_server_overlays(
    f: &mut Frame,
    state: &RunRemoteState,
    content_chunk: Rect,
    root: &LayoutJson,
) {
    if state.srv_popup_active {
        let w = state.srv_popup_width.min(content_chunk.width.saturating_sub(2));
        let h = state.srv_popup_height.min(content_chunk.height.saturating_sub(2));
        let popup_area = Rect {
            x: content_chunk.x + (content_chunk.width.saturating_sub(w)) / 2,
            y: content_chunk.y + (content_chunk.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };
        let title = if state.srv_popup_command.is_empty() { "Popup".to_string() } else {
            let max_title = (w as usize).saturating_sub(4);
            if state.srv_popup_command.len() > max_title { format!("{}...", &state.srv_popup_command[..max_title.saturating_sub(3)]) }
            else { state.srv_popup_command.clone() }
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(title);
        let inner_w = w.saturating_sub(2);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if !state.srv_popup_rows.is_empty() {
            for row_data in &state.srv_popup_rows {
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut col: u16 = 0;
                for run in &row_data.runs {
                    if col >= inner_w { break; }
                    let fg = crate::style::map_color(&run.fg);
                    let bg = crate::style::map_color(&run.bg);
                    let mut style = Style::default().fg(fg).bg(bg);
                    if run.flags & 1  != 0 { style = style.add_modifier(Modifier::DIM); }
                    if run.flags & 2  != 0 { style = style.add_modifier(Modifier::BOLD); }
                    if run.flags & 4  != 0 { style = style.add_modifier(Modifier::ITALIC); }
                    if run.flags & 8  != 0 { style = style.add_modifier(Modifier::UNDERLINED); }
                    if run.flags & 16 != 0 { style = style.add_modifier(Modifier::REVERSED); }
                    if run.flags & 32 != 0 { style = style.add_modifier(Modifier::SLOW_BLINK); }
                    if run.flags & 128 != 0 { style = style.add_modifier(Modifier::CROSSED_OUT); }
                    let text: &str = if run.flags & 64 != 0 { " " } else if run.text.is_empty() { " " } else { &run.text };
                    let run_w = run.width.max(1);
                    if col + run_w > inner_w {
                        let avail = (inner_w - col) as usize;
                        let truncated: String = text.chars().take(avail).collect();
                        if !truncated.is_empty() {
                            spans.push(Span::styled(truncated, style));
                        }
                        col = inner_w;
                    } else {
                        spans.push(Span::styled(text.to_string(), style));
                        col += run_w;
                    }
                }
                lines.push(Line::from(spans));
            }
        } else {
            for line_str in &state.srv_popup_lines {
                lines.push(Line::from(line_str.clone()));
            }
        }
        let para = Paragraph::new(Text::from(lines)).block(block).scroll((state.srv_popup_scroll, 0));
        f.render_widget(Clear, popup_area);
        f.render_widget(para, popup_area);
    }

    if state.srv_confirm_active {
        let overlay = Block::default().borders(Borders::ALL).title("confirm");
        let oa = centered_rect(60, 3, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let para = Paragraph::new(state.srv_confirm_prompt.clone());
        f.render_widget(para, overlay.inner(oa));
    }

    if state.srv_menu_active {
        let sel_style = crate::rendering::parse_tmux_style(&state.mode_style_str);
        let title_str = if state.srv_menu_title.is_empty() { "Menu".to_string() } else { state.srv_menu_title.clone() };
        let overlay = Block::default().borders(Borders::ALL).title(title_str).border_style(sel_style);
        let item_count = state.srv_menu_items.len();
        let menu_h = ((item_count as u16).saturating_add(2)).max(3).min(content_chunk.height.saturating_sub(2));
        let oa = centered_rect(50, menu_h, content_chunk);
        f.render_widget(Clear, oa);
        f.render_widget(&overlay, oa);
        let inner = overlay.inner(oa);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, item) in state.srv_menu_items.iter().enumerate() {
            if item.sep {
                lines.push(Line::from("\u{2500}".repeat(inner.width as usize)));
            } else {
                let name = item.name.clone().unwrap_or_default();
                let key_str = item.key.clone().unwrap_or_default();
                let label = if key_str.is_empty() { name } else { format!("{} ({})", name, key_str) };
                if i == state.srv_menu_selected {
                    lines.push(Line::from(Span::styled(label, sel_style)));
                } else {
                    lines.push(Line::from(label));
                }
            }
        }
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    if state.srv_customize_active {
        render_customize_overlay(f, state, content_chunk);
    }

    if state.srv_display_panes {
        render_display_panes(f, root, content_chunk, state.srv_pane_base_index);
    }
}

fn render_customize_overlay(f: &mut Frame, state: &RunRemoteState, area: Rect) {
    let overlay = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4).min(100),
        height: area.height.saturating_sub(2),
    };
    f.render_widget(Clear, overlay);
    let header_style = Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD);
    let header = if state.srv_customize_filter.is_empty() {
        " Customize Mode  [q:exit  /:filter  Enter:edit  d:reset default] "
    } else {
        " Customize Mode  [q:exit  /:clear filter  Enter:edit  d:reset] "
    };
    if overlay.height > 0 {
        let header_area = Rect { x: overlay.x, y: overlay.y, width: overlay.width, height: 1 };
        let hdr = Paragraph::new(Line::from(Span::styled(
            format!("{:<width$}", header, width = overlay.width as usize),
            header_style,
        )));
        f.render_widget(hdr, header_area);
    }
    let body_start = overlay.y + 1;
    if !state.srv_customize_filter.is_empty() && overlay.height > 1 {
        let filter_area = Rect { x: overlay.x, y: body_start, width: overlay.width, height: 1 };
        let filter_style = Style::default().fg(Color::Yellow).bg(Color::DarkGray);
        let ftxt = format!(" Filter: {} ", state.srv_customize_filter);
        f.render_widget(Paragraph::new(Line::from(Span::styled(
            format!("{:<width$}", ftxt, width = overlay.width as usize), filter_style,
        ))), filter_area);
    }
    let list_start = if state.srv_customize_filter.is_empty() { body_start } else { body_start + 1 };
    let list_height = overlay.y.saturating_add(overlay.height).saturating_sub(list_start) as usize;
    if list_height > 0 {
        let col_hdr_area = Rect { x: overlay.x, y: list_start, width: overlay.width, height: 1 };
        let col_style = Style::default().fg(Color::White).bg(Color::DarkGray).add_modifier(Modifier::BOLD);
        let name_w = (overlay.width as usize / 2).max(20);
        let col_text = format!(" {:<nw$} {}", "Option", "Value", nw = name_w.saturating_sub(2));
        f.render_widget(Paragraph::new(Line::from(Span::styled(
            format!("{:<width$}", col_text, width = overlay.width as usize), col_style,
        ))), col_hdr_area);
    }
    let rows_start = list_start + 1;
    let rows_height = overlay.y.saturating_add(overlay.height).saturating_sub(rows_start) as usize;
    let visible_opts: Vec<&CustomizeOption> = state.srv_customize_options.iter()
        .skip(state.srv_customize_scroll)
        .take(rows_height)
        .collect();
    for (row_idx, opt) in visible_opts.iter().enumerate() {
        if rows_start + row_idx as u16 >= overlay.y + overlay.height { break; }
        let row_area = Rect {
            x: overlay.x,
            y: rows_start + row_idx as u16,
            width: overlay.width,
            height: 1,
        };
        let is_selected = opt.i == state.srv_customize_selected;
        let name_w = (overlay.width as usize / 2).max(20);
        let scope_prefix = match opt.s.as_str() {
            "server" => "[S] ",
            "session" => "[s] ",
            "window" => "[w] ",
            "pane" => "[p] ",
            _ => "    ",
        };
        let name_display = format!("{}{}", scope_prefix, opt.n);
        let value_display = if is_selected && state.srv_customize_editing {
            format!("{}|", state.srv_customize_edit_buf)
        } else {
            opt.v.clone()
        };
        let line_text = format!(" {:<nw$} {}", name_display, value_display, nw = name_w.saturating_sub(2));
        let style = if is_selected {
            if state.srv_customize_editing {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default().fg(Color::Black).bg(Color::White)
            }
        } else {
            Style::default().fg(Color::White).bg(Color::Reset)
        };
        f.render_widget(Paragraph::new(Line::from(Span::styled(
            format!("{:<width$}", line_text, width = overlay.width as usize), style,
        ))), row_area);
    }
}

fn render_display_panes(f: &mut Frame, root: &LayoutJson, content_chunk: Rect, pane_base_index: usize) {
    fn collect_leaf_rects(node: &LayoutJson, area: Rect, out: &mut Vec<Rect>) {
        match node {
            LayoutJson::Leaf { .. } => { out.push(area); }
            LayoutJson::Split { kind, sizes, children } => {
                let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                    sizes.clone()
                } else {
                    vec![(100 / children.len().max(1)) as u16; children.len()]
                };
                let is_horizontal = kind == "Horizontal";
                let rects = crate::tree::split_with_gaps(is_horizontal, &effective_sizes, area);
                for (i, child) in children.iter().enumerate() {
                    if i < rects.len() { collect_leaf_rects(child, rects[i], out); }
                }
            }
        }
    }
    let mut leaf_rects = Vec::new();
    collect_leaf_rects(root, content_chunk, &mut leaf_rects);
    for (idx, prect) in leaf_rects.iter().enumerate() {
        if prect.width >= 7 && prect.height >= 3 {
            let bw = 7u16; let bh = 3u16;
            let bx = prect.x + prect.width.saturating_sub(bw) / 2;
            let by = prect.y + prect.height.saturating_sub(bh) / 2;
            let b = Rect { x: bx, y: by, width: bw, height: bh };
            let pane_sel_style = Style::default().fg(Color::Yellow).bg(Color::Black).add_modifier(Modifier::BOLD);
            let block = Block::default().borders(Borders::ALL).style(pane_sel_style);
            let inner = block.inner(b);
            let disp = ((idx + pane_base_index) % 10).to_string();
            let para = Paragraph::new(Line::from(Span::styled(
                format!(" {} ", disp),
                pane_sel_style,
            ))).alignment(Alignment::Center);
            f.render_widget(Clear, b);
            f.render_widget(block, b);
            f.render_widget(para, inner);
        }
    }
}

/// Write post-draw cursor visibility + position + DECSCUSR as one atomic VT write.
pub(crate) fn post_draw_cursor(
    root: &LayoutJson,
    post_draw_cursor_pos: Option<(u16, u16)>,
    terminal: &ratatui::Terminal<ratatui::backend::CrosstermBackend<crate::platform::PsmuxWriter>>,
    status_at_top: bool,
    status_lines: usize,
    state_cursor_style_code: Option<u8>,
    last_cursor_style: &mut u8,
    is_ssh_mode: bool,
) {
    use std::io::Write;
    fn find_active_cursor_shape(node: &LayoutJson) -> Option<u8> {
        match node {
            LayoutJson::Leaf { active, cursor_shape, .. } => {
                if *active && *cursor_shape >= 1 && *cursor_shape <= 6 { Some(*cursor_shape) } else { None }
            }
            LayoutJson::Split { children, .. } => {
                children.iter().find_map(find_active_cursor_shape)
            }
        }
    }
    let effective = find_active_cursor_shape(root)
        .unwrap_or_else(|| state_cursor_style_code.unwrap_or_else(crate::rendering::configured_cursor_code));

    fn find_active_rect(node: &LayoutJson, area: Rect) -> Option<Rect> {
        match node {
            LayoutJson::Leaf { active, .. } => {
                if *active { Some(area) } else { None }
            }
            LayoutJson::Split { kind, sizes, children } => {
                let eff: Vec<u16> = if sizes.len() == children.len() {
                    sizes.clone()
                } else {
                    vec![(100 / children.len().max(1)) as u16; children.len()]
                };
                let rects = crate::tree::split_with_gaps(kind == "Horizontal", &eff, area);
                for (i, child) in children.iter().enumerate() {
                    if i < rects.len() {
                        if let Some(r) = find_active_rect(child, rects[i]) { return Some(r); }
                    }
                }
                None
            }
        }
    }
    let active_pane_area: Option<Rect> = {
        let sz = terminal.size().unwrap_or_default();
        let constraints = if status_at_top {
            vec![Constraint::Length(status_lines as u16), Constraint::Min(1)]
        } else {
            vec![Constraint::Min(1), Constraint::Length(status_lines as u16)]
        };
        let chunks = Layout::default().direction(Direction::Vertical)
            .constraints(constraints).split(sz.into());
        let content_chunk = if status_at_top { chunks[1] } else { chunks[0] };
        find_active_rect(root, content_chunk)
    };
    let cursor_visible = if let (Some((cc, cr)), Some(inner)) = (post_draw_cursor_pos, active_pane_area) {
        let cy = inner.y + cr.min(inner.height.saturating_sub(1));
        let cx = inner.x + cc.min(inner.width.saturating_sub(1));
        Some((cx, cy))
    } else {
        None
    };
    let mut buf = String::with_capacity(32);
    if let Some((cx, cy)) = cursor_visible {
        buf.push_str("\x1b[?25h");
        use std::fmt::Write as FmtWrite;
        let _ = write!(buf, "\x1b[{};{}H", cy + 1, cx + 1);
    }
    if effective != *last_cursor_style {
        *last_cursor_style = effective;
        use std::fmt::Write as FmtWrite;
        let _ = write!(buf, "\x1b[{} q", effective);
    }
    if !buf.is_empty() {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
    }
    if !is_ssh_mode {
        if let Some((cx, cy)) = cursor_visible {
            crate::platform::caret::update(cx, cy);
        }
    }
}
