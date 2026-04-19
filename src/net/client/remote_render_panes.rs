use super::*;
/// Recursively render the pane layout tree (Leaf cells + Split separators).
pub(crate) fn render_json(f: &mut Frame, node: &LayoutJson, area: Rect, dim_preds: bool, border_fg: Color, active_border_fg: Color, clock_mode: bool, clock_colour: Color, active_rect: Option<Rect>, mode_style_str: &str, zoomed: bool, border_status: &str, border_format: &str) {
    match node {
        LayoutJson::Leaf {
            id,
            rows: _,
            cols: _,
            cursor_row,
            cursor_col,
            alternate_screen,
            hide_cursor: _,
            cursor_shape: _,
            active,
            copy_mode,
            scroll_offset,
            sel_start_row,
            sel_start_col,
            sel_end_row,
            sel_end_col,
            sel_mode,
            copy_cursor_row,
            copy_cursor_col,
            content,
            rows_v2,
            title,
        } => {
            let inner = area;
            let mut lines: Vec<Line> = Vec::new();
            let use_full_cells = *copy_mode && *active && !content.is_empty();
            if use_full_cells || rows_v2.is_empty() {
                for r in 0..inner.height.min(content.len() as u16) {
                    let mut spans: Vec<Span> = Vec::new();
                    let row = &content[r as usize];
                    let max_c = inner.width.min(row.len() as u16);
                    let mut c: u16 = 0;
                    while c < max_c {
                        let cell = &row[c as usize];
                        let mut fg = map_color(&cell.fg);
                        let bg = map_color(&cell.bg);
                        let in_selection = if *copy_mode && *active {
                            if let (Some(sr), Some(sc), Some(er), Some(ec)) = (sel_start_row, sel_start_col, sel_end_row, sel_end_col) {
                                let mode = sel_mode.as_deref().unwrap_or("char");
                                match mode {
                                    "rect" => r >= *sr && r <= *er && c >= (*sc).min(*ec) && c <= (*sc).max(*ec),
                                    "line" => r >= *sr && r <= *er,
                                    _ => {
                                        if *sr == *er {
                                            r == *sr && c >= (*sc).min(*ec) && c <= (*sc).max(*ec)
                                        } else if r == *sr {
                                            c >= *sc
                                        } else if r == *er {
                                            c <= *ec
                                        } else {
                                            r > *sr && r < *er
                                        }
                                    }
                                }
                            } else { false }
                        } else { false };
                        if *active && dim_preds && !*alternate_screen
                            && (r > *cursor_row || (r == *cursor_row && c >= *cursor_col))
                        {
                            fg = dim_color(fg);
                        }
                        let mut style = Style::default().fg(fg).bg(bg);
                        if in_selection {
                            let ms = crate::rendering::parse_tmux_style(mode_style_str);
                            style = ms;
                        }
                        if cell.inverse { style = style.add_modifier(Modifier::REVERSED); }
                        if cell.dim { style = style.add_modifier(Modifier::DIM); }
                        if cell.bold { style = style.add_modifier(Modifier::BOLD); }
                        if cell.italic { style = style.add_modifier(Modifier::ITALIC); }
                        if cell.underline { style = style.add_modifier(Modifier::UNDERLINED); }
                        if cell.blink { style = style.add_modifier(Modifier::SLOW_BLINK); }
                        if cell.strikethrough { style = style.add_modifier(Modifier::CROSSED_OUT); }
                        let text: &str = if cell.hidden { " " } else if cell.text.is_empty() { " " } else { &cell.text };
                        let char_width = unicode_width::UnicodeWidthStr::width(text) as u16;
                        if char_width >= 2 && c + char_width > max_c {
                            spans.push(Span::styled(" ", style));
                            c += 1;
                        } else {
                            spans.push(Span::styled(text, style));
                            if char_width >= 2 { c += 2; } else { c += 1; }
                        }
                    }
                    if c < inner.width {
                        let last_bg = if !spans.is_empty() {
                            spans.last().unwrap().style.bg.unwrap_or(Color::Reset)
                        } else { Color::Reset };
                        let pad = " ".repeat((inner.width - c) as usize);
                        spans.push(Span::styled(pad, Style::default().bg(last_bg)));
                    }
                    lines.push(Line::from(spans));
                }
            } else {
                for r in 0..inner.height.min(rows_v2.len() as u16) {
                    let mut spans: Vec<Span> = Vec::new();
                    let mut c: u16 = 0;
                    let mut last_bg = Color::Reset;
                    for run in &rows_v2[r as usize].runs {
                        if c >= inner.width { break; }
                        let mut fg = map_color(&run.fg);
                        let bg = map_color(&run.bg);
                        last_bg = bg;
                        if *active && dim_preds && !*alternate_screen
                            && (r > *cursor_row || (r == *cursor_row && c >= *cursor_col))
                        {
                            fg = dim_color(fg);
                        }
                        let mut style = Style::default().fg(fg).bg(bg);
                        if run.flags & 16 != 0 { style = style.add_modifier(Modifier::REVERSED); }
                        if run.flags & 1 != 0 { style = style.add_modifier(Modifier::DIM); }
                        if run.flags & 2 != 0 { style = style.add_modifier(Modifier::BOLD); }
                        if run.flags & 4 != 0 { style = style.add_modifier(Modifier::ITALIC); }
                        if run.flags & 8 != 0 { style = style.add_modifier(Modifier::UNDERLINED); }
                        if run.flags & 32 != 0 { style = style.add_modifier(Modifier::SLOW_BLINK); }
                        if run.flags & 128 != 0 { style = style.add_modifier(Modifier::CROSSED_OUT); }
                        let text: &str = if run.flags & 64 != 0 { " " } else if run.text.is_empty() { " " } else { &run.text };
                        let run_w = run.width.max(1);
                        if c + run_w > inner.width {
                            let avail = (inner.width - c) as usize;
                            let mut truncated = String::new();
                            let mut used = 0usize;
                            for ch in text.chars() {
                                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                                if used + cw > avail { break; }
                                used += cw;
                                truncated.push(ch);
                            }
                            if !truncated.is_empty() {
                                spans.push(Span::styled(truncated, style));
                            }
                            c = inner.width;
                        } else {
                            spans.push(Span::styled(text, style));
                            c = c.saturating_add(run_w);
                        }
                    }
                    if c < inner.width {
                        let pad = " ".repeat((inner.width - c) as usize);
                        spans.push(Span::styled(pad, Style::default().bg(last_bg)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            f.render_widget(Clear, inner);
            let para = Paragraph::new(Text::from(lines));
            f.render_widget(para, inner);

            // Copy mode indicator
            if *copy_mode && *active {
                let label = "[copy mode]";
                let lw = label.len() as u16;
                if area.width >= lw {
                    let lx = area.x + area.width.saturating_sub(lw);
                    let la = Rect::new(lx, area.y, lw, 1);
                    let ls = Span::styled(label, Style::default().fg(Color::Black).bg(Color::Yellow));
                    f.render_widget(Paragraph::new(Line::from(ls)), la);
                }
            }

            if *copy_mode && *active && *scroll_offset > 0 {
                let indicator = format!("[{}/{}]", scroll_offset, scroll_offset);
                let indicator_width = indicator.len() as u16;
                if area.width > indicator_width + 2 {
                    let indicator_x = area.x + area.width - indicator_width - 1;
                    let indicator_y = if *copy_mode { area.y + 1 } else { area.y };
                    let indicator_area = Rect::new(indicator_x, indicator_y, indicator_width, 1);
                    let indicator_span = Span::styled(indicator, Style::default().fg(Color::Black).bg(Color::Yellow));
                    f.render_widget(Paragraph::new(Line::from(indicator_span)), indicator_area);
                }
            }

            if *active && !*copy_mode {
                if clock_mode {
                    super::remote_render_clock::render_clock_overlay(f, inner, clock_colour);
                }
            }

            // Copy mode cursor with reverse video
            if *copy_mode && *active {
                if let (Some(cr), Some(cc)) = (copy_cursor_row, copy_cursor_col) {
                    let cr = (*cr).min(inner.height.saturating_sub(1));
                    let cc = (*cc).min(inner.width.saturating_sub(1));
                    let cy = inner.y + cr;
                    let cx = inner.x + cc;
                    f.set_cursor_position((cx, cy));
                    let buf = f.buffer_mut();
                    let buf_area = buf.area;
                    if cy >= buf_area.y && cy < buf_area.y + buf_area.height
                        && cx >= buf_area.x && cx < buf_area.x + buf_area.width
                    {
                        let idx = (cy - buf_area.y) as usize * buf_area.width as usize
                            + (cx - buf_area.x) as usize;
                        if idx < buf.content.len() {
                            let cell = &mut buf.content[idx];
                            cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
                        }
                    }
                }
            }
            // Pane border format/status overlay
            if border_status != "off" && !border_format.is_empty() && area.height > 1 {
                let pane_title_str = title.as_deref().unwrap_or("");
                let pane_label = border_format
                    .replace("#{pane_title}", pane_title_str)
                    .replace("#{pane_index}", &id.to_string())
                    .replace("#P", &id.to_string());
                let label_width = unicode_width::UnicodeWidthStr::width(pane_label.as_str()) as u16;
                if label_width > 0 && area.width >= label_width {
                    let label_y = if border_status == "bottom" { area.y + area.height.saturating_sub(1) } else { area.y };
                    let label_area = Rect::new(area.x, label_y, label_width.min(area.width), 1);
                    let label_style = Style::default().fg(if *active { active_border_fg } else { border_fg });
                    f.render_widget(Paragraph::new(Line::from(Span::styled(pane_label, label_style))), label_area);
                }
            }
        }
        LayoutJson::Split { kind, sizes, children } => {
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else {
                vec![(100 / children.len().max(1)) as u16; children.len()]
            };
            let is_horizontal = kind == "Horizontal";
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);

            for (i, child) in children.iter().enumerate() {
                if i < rects.len() { render_json(f, child, rects[i], dim_preds, border_fg, active_border_fg, clock_mode, clock_colour, active_rect, mode_style_str, zoomed, border_status, border_format); }
            }

            if zoomed { return; }
            let border_style = Style::default().fg(border_fg);
            let active_border_style = Style::default().fg(active_border_fg);
            let buf = f.buffer_mut();
            for i in 0..children.len().saturating_sub(1) {
                if i >= rects.len() { break; }
                let both_leaves = matches!(&children[i], LayoutJson::Leaf { .. })
                    && matches!(children.get(i + 1), Some(LayoutJson::Leaf { .. }));

                if is_horizontal {
                    let sep_x = rects[i].x + rects[i].width;
                    if sep_x < buf.area.x + buf.area.width {
                        if both_leaves {
                            let left_active = matches!(&children[i], LayoutJson::Leaf { active, .. } if *active);
                            let right_active = matches!(children.get(i + 1), Some(LayoutJson::Leaf { active, .. }) if *active);
                            let left_sty = if left_active { active_border_style } else { border_style };
                            let right_sty = if right_active { active_border_style } else { border_style };
                            let mid_y = area.y + area.height / 2;
                            for y in area.y..area.y + area.height {
                                let sty = if y < mid_y { left_sty } else { right_sty };
                                let idx = (y - buf.area.y) as usize * buf.area.width as usize
                                    + (sep_x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('\u{2502}');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        } else {
                            for y in area.y..area.y + area.height {
                                let active = active_rect.map_or(false, |ar| {
                                    y >= ar.y && y < ar.y + ar.height
                                    && (sep_x == ar.x + ar.width || sep_x + 1 == ar.x)
                                });
                                let sty = if active { active_border_style } else { border_style };
                                let idx = (y - buf.area.y) as usize * buf.area.width as usize
                                    + (sep_x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('\u{2502}');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        }
                    }
                } else {
                    let sep_y = rects[i].y + rects[i].height;
                    if sep_y < buf.area.y + buf.area.height {
                        if both_leaves {
                            let top_active = matches!(&children[i], LayoutJson::Leaf { active, .. } if *active);
                            let bot_active = matches!(children.get(i + 1), Some(LayoutJson::Leaf { active, .. }) if *active);
                            let top_sty = if top_active { active_border_style } else { border_style };
                            let bot_sty = if bot_active { active_border_style } else { border_style };
                            let mid_x = area.x + area.width / 2;
                            for x in area.x..area.x + area.width {
                                let sty = if x < mid_x { top_sty } else { bot_sty };
                                let idx = (sep_y - buf.area.y) as usize * buf.area.width as usize
                                    + (x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('\u{2500}');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        } else {
                            for x in area.x..area.x + area.width {
                                let active = active_rect.map_or(false, |ar| {
                                    x >= ar.x && x < ar.x + ar.width
                                    && (sep_y == ar.y + ar.height || sep_y + 1 == ar.y)
                                });
                                let sty = if active { active_border_style } else { border_style };
                                let idx = (sep_y - buf.area.y) as usize * buf.area.width as usize
                                    + (x - buf.area.x) as usize;
                                if idx < buf.content.len() {
                                    buf.content[idx].set_char('\u{2500}');
                                    buf.content[idx].set_style(sty);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Re-color all border characters based on adjacency to the active pane rect.
pub(crate) fn recolor_borders(buf: &mut ratatui::buffer::Buffer, active_rect: Option<Rect>, border_fg: Color, active_border_fg: Color) {
    if let Some(ar) = active_rect {
        let w = buf.area.width as usize;
        let h = buf.area.height as usize;
        let border_style = Style::default().fg(border_fg);
        let active_style = Style::default().fg(active_border_fg);
        for row in 0..h {
            for col in 0..w {
                let idx = row * w + col;
                if idx >= buf.content.len() { continue; }
                let ch = buf.content[idx].symbol().chars().next().unwrap_or(' ');
                if !matches!(ch, '\u{2502}' | '\u{2500}' | '\u{253c}' | '\u{251c}' | '\u{2524}' | '\u{252c}' | '\u{2534}') { continue; }
                let x = buf.area.x + col as u16;
                let y = buf.area.y + row as u16;
                let adj = (x + 1 == ar.x && y >= ar.y && y < ar.y + ar.height)
                    || (x == ar.x + ar.width && y >= ar.y && y < ar.y + ar.height)
                    || (y + 1 == ar.y && x >= ar.x && x < ar.x + ar.width)
                    || (y == ar.y + ar.height && x >= ar.x && x < ar.x + ar.width)
                    || ((x + 1 == ar.x || x == ar.x + ar.width) && (y + 1 == ar.y || y == ar.y + ar.height));
                buf.content[idx].set_style(if adj { active_style } else { border_style });
            }
        }
    }
}

/// Highlight the border segment under the cursor for drag preview.
pub(crate) fn render_hover_border(buf: &mut ratatui::buffer::Buffer, hovered_border: &Option<(u16, String, Rect)>, hover_fg: Color) {
    if let Some((hpos, ref hkind, harea)) = *hovered_border {
        let w = buf.area.width as usize;
        let hover_style = Style::default().fg(hover_fg);
        if hkind == "Horizontal" {
            let col = hpos as usize;
            if col >= buf.area.x as usize && col < (buf.area.x + buf.area.width) as usize {
                for y in harea.y..harea.y + harea.height {
                    let idx = (y - buf.area.y) as usize * w + (col - buf.area.x as usize);
                    if idx < buf.content.len() {
                        buf.content[idx].set_style(hover_style);
                    }
                }
            }
        } else {
            let row = hpos as usize;
            if row >= buf.area.y as usize && row < (buf.area.y + buf.area.height) as usize {
                for x in harea.x..harea.x + harea.width {
                    let idx = (row - buf.area.y as usize) * w + (x - buf.area.x) as usize;
                    if idx < buf.content.len() {
                        buf.content[idx].set_style(hover_style);
                    }
                }
            }
        }
    }
}
