use super::*;
use super::run_remote_state::RunRemoteState;
use super::run_remote_types::*;

/// Render the status bar (line 0 and additional status lines).
pub(crate) fn render_status_bar(
    f: &mut Frame,
    state: &mut RunRemoteState,
    status_chunk: Rect,
    windows: &[WinStatus],
    base_index: usize,
    name: &str,
    status_lines: usize,
    status_format: &[String],
    status_message: &Option<String>,
) {
    let sb_fg = state.status_fg;
    let sb_bg = state.status_bg;
    let sb_base = if state.status_bold {
        Style::default().fg(sb_fg).bg(sb_bg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(sb_fg).bg(sb_bg)
    };
    use unicode_width::UnicodeWidthStr;

    let use_status_format_0 = !status_format.is_empty() && !status_format[0].is_empty();

    let left_prefix = match state.custom_status_left {
        Some(ref sl) => sl.clone(),
        None => format!("[{}] ", name),
    };
    let mut left_spans: Vec<Span> = crate::rendering::parse_inline_styles(&left_prefix, sb_base);

    let mut tab_spans_all: Vec<Span> = Vec::new();
    let mut tab_rel_positions: Vec<(usize, u16, u16)> = Vec::new();
    let mut tab_cursor: u16 = 0;
    for (i, w) in windows.iter().enumerate() {
        let tab_text = if !w.tab_text.is_empty() {
            w.tab_text.clone()
        } else {
            let display_idx = i + base_index;
            let fmt = if w.active { &state.win_status_current_fmt } else { &state.win_status_fmt };
            fmt.replace("#I", &display_idx.to_string())
               .replace("#W", &w.name)
               .replace("#F", if w.active { "*" } else { "" })
        };
        if i > 0 {
            let sep_spans = crate::rendering::parse_inline_styles(&state.win_status_sep, sb_base);
            let sep_w: u16 = sep_spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16).sum();
            tab_spans_all.extend(sep_spans);
            tab_cursor += sep_w;
        }
        let fallback_style = if w.active {
            if let Some((fg, bg, bold)) = state.win_status_current_style {
                let mut s = Style::default();
                if let Some(c) = fg { s = s.fg(c); }
                if let Some(c) = bg { s = s.bg(c); }
                if bold { s = s.add_modifier(Modifier::BOLD); }
                s
            } else { sb_base }
        } else if w.activity {
            Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            if let Some((fg, bg, bold)) = state.win_status_style {
                let mut s = Style::default();
                if let Some(c) = fg { s = s.fg(c); }
                if let Some(c) = bg { s = s.bg(c); }
                if bold { s = s.add_modifier(Modifier::BOLD); }
                s
            } else { sb_base }
        };
        let parsed = crate::rendering::parse_inline_styles(&tab_text, fallback_style);
        let tab_start = tab_cursor;
        let tab_w: u16 = parsed.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16).sum();
        tab_cursor += tab_w;
        tab_rel_positions.push((i, tab_start, tab_cursor));
        tab_spans_all.extend(parsed);
    }

    let right_text = state.custom_status_right.as_deref().unwrap_or("").to_string();
    let mut right_spans = crate::rendering::parse_inline_styles(&right_text, sb_base);

    crate::style::truncate_spans_to_width(&mut left_spans, state.last_sent_size.0 as usize); // use status_left_length if available
    crate::style::truncate_spans_to_width(&mut right_spans, state.last_sent_size.0 as usize);

    let left_w: usize = left_spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
    let tabs_w: usize = tab_spans_all.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
    let right_w: usize = right_spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
    let total_width = status_chunk.width as usize;

    let mut status_spans: Vec<Span> = Vec::new();
    match state.status_justify_str.as_str() {
        "centre" | "center" => {
            let avail = total_width.saturating_sub(left_w).saturating_sub(right_w);
            let pad_before = avail.saturating_sub(tabs_w) / 2;
            let pad_after = avail.saturating_sub(tabs_w).saturating_sub(pad_before);
            status_spans.extend(left_spans);
            if pad_before > 0 { status_spans.push(Span::styled(" ".repeat(pad_before), sb_base)); }
            status_spans.extend(tab_spans_all);
            if pad_after > 0 { status_spans.push(Span::styled(" ".repeat(pad_after), sb_base)); }
            status_spans.extend(right_spans);
        }
        "absolute-centre" | "absolute-center" => {
            let tabs_start = total_width.saturating_sub(tabs_w) / 2;
            status_spans.extend(left_spans);
            let pad_before = tabs_start.saturating_sub(left_w);
            if pad_before > 0 { status_spans.push(Span::styled(" ".repeat(pad_before), sb_base)); }
            status_spans.extend(tab_spans_all);
            let used = left_w + pad_before + tabs_w;
            let pad_after = total_width.saturating_sub(used).saturating_sub(right_w);
            if pad_after > 0 { status_spans.push(Span::styled(" ".repeat(pad_after), sb_base)); }
            status_spans.extend(right_spans);
        }
        "right" => {
            status_spans.extend(left_spans);
            let used = left_w + tabs_w + right_w;
            let pad = total_width.saturating_sub(used);
            if pad > 0 { status_spans.push(Span::styled(" ".repeat(pad), sb_base)); }
            status_spans.extend(tab_spans_all);
            status_spans.extend(right_spans);
        }
        _ => {
            status_spans.extend(left_spans);
            status_spans.extend(tab_spans_all);
            let used = left_w + tabs_w + right_w;
            let pad = total_width.saturating_sub(used);
            if pad > 0 { status_spans.push(Span::styled(" ".repeat(pad), sb_base)); }
            status_spans.extend(right_spans);
        }
    }

    let tabs_x_offset: u16 = status_chunk.x + match state.status_justify_str.as_str() {
        "centre" | "center" => {
            let avail = total_width.saturating_sub(left_w).saturating_sub(right_w);
            let pad_before = avail.saturating_sub(tabs_w) / 2;
            (left_w + pad_before) as u16
        }
        "absolute-centre" | "absolute-center" => {
            let tabs_start = total_width.saturating_sub(tabs_w) / 2;
            tabs_start as u16
        }
        "right" => {
            let used = left_w + tabs_w + right_w;
            let pad = total_width.saturating_sub(used);
            (left_w + pad) as u16
        }
        _ => left_w as u16,
    };
    state.client_tab_positions = tab_rel_positions.iter().map(|&(idx, s, e)| (idx, s + tabs_x_offset, e + tabs_x_offset)).collect();
    state.client_status_row = status_chunk.y;

    crate::style::truncate_spans_to_width(&mut status_spans, total_width);

    let status_bar = if let Some(ref msg) = status_message {
        let msg_style = crate::rendering::parse_tmux_style("bg=yellow,fg=black");
        let padded = if msg.len() < status_chunk.width as usize {
            format!("{}{}", msg, " ".repeat(status_chunk.width as usize - msg.len()))
        } else {
            msg.chars().take(status_chunk.width as usize).collect()
        };
        Paragraph::new(Line::from(Span::styled(padded, msg_style))).style(msg_style)
    } else {
        Paragraph::new(Line::from(status_spans)).style(sb_base)
    };

    f.render_widget(Clear, status_chunk);
    let line0_area = Rect { x: status_chunk.x, y: status_chunk.y, width: status_chunk.width, height: 1.min(status_chunk.height) };
    if use_status_format_0 && status_message.is_none() {
        let layout = crate::style::layout_format_line(
            &status_format[0], total_width, sb_base,
        );
        state.client_tab_positions = layout.ranges.iter().filter_map(|(rt, s, e)| {
            match rt {
                crate::style::StatusRangeType::Window(idx) => {
                    Some((*idx, *s + status_chunk.x, *e + status_chunk.x))
                }
            }
        }).collect();
        let fmt0_widget = Paragraph::new(Line::from(layout.spans)).style(sb_base);
        f.render_widget(fmt0_widget, line0_area);
    } else {
        f.render_widget(status_bar, line0_area);
    }

    for line_idx in 1..status_lines {
        let line_y = status_chunk.y + line_idx as u16;
        if line_y >= status_chunk.y + status_chunk.height { break; }
        let line_area = Rect { x: status_chunk.x, y: line_y, width: status_chunk.width, height: 1 };
        let text = if line_idx < status_format.len() && !status_format[line_idx].is_empty() {
            status_format[line_idx].clone()
        } else {
            String::new()
        };
        let layout = crate::style::layout_format_line(&text, line_area.width as usize, sb_base);
        let line_widget = Paragraph::new(Line::from(layout.spans)).style(sb_base);
        f.render_widget(line_widget, line_area);
    }
}
