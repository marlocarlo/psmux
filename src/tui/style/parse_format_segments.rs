#[allow(unused_imports)]

use ratatui::prelude::*;
use ratatui::style::{Style, Modifier};

use crate::debug_log::style_log;

// ─── Color mapping ──────────────────────────────────────────────────────────

/// Map a tmux color name/hex/index string to a ratatui `Color`.
///
/// Supports: named colors, `brightX`, `colourN`/`colorN`, `#RRGGBB`,
/// `idx:N`, `rgb:R,G,B`, and `default`/`terminal`.
use super::*;

/// Parse a format string into tokens that preserve layout directives.
///
/// Like `parse_inline_styles` but emits `FormatToken` variants for
/// `#[align=...]`, `#[fill]`, `#[list=...]`, `#[range=...]` instead of
/// silently discarding them.
pub fn parse_format_segments(text: &str, base_style: Style) -> Vec<FormatToken> {
    let mut tokens: Vec<FormatToken> = Vec::new();
    let mut cur_style = base_style;
    let mut style_stack: Vec<Style> = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some(end) = text[i + 2..].find(']') {
                let token_str = &text[i + 2..i + 2 + end];
                // Split on both comma and whitespace to handle
                // `#[range=window|0,list=focus]` and `#[range=window|0 list=focus]`
                for part in token_str.split(|c: char| c == ',' || c.is_whitespace()) {
                    let p = part.trim();
                    if p.is_empty() { continue; }

                    if p.starts_with("fg=") {
                        cur_style = cur_style.fg(map_color(&p[3..]));
                    } else if p.starts_with("bg=") {
                        cur_style = cur_style.bg(map_color(&p[3..]));
                    } else if p == "default" || p == "none" {
                        cur_style = base_style;
                    } else if p == "push-default" {
                        style_stack.push(cur_style);
                    } else if p == "pop-default" {
                        cur_style = style_stack.pop().unwrap_or(base_style);
                    }
                    // ── Layout directives ──
                    else if p == "fill" || p.starts_with("fill=") {
                        let mut fill_style = cur_style;
                        if let Some(color_str) = p.strip_prefix("fill=") {
                            fill_style = fill_style.bg(map_color(color_str));
                        }
                        tokens.push(FormatToken::Fill(fill_style));
                    } else if p.starts_with("align=") {
                        let align = match &p[6..] {
                            "left" => StatusAlignment::Left,
                            "centre" | "center" => StatusAlignment::Centre,
                            "right" => StatusAlignment::Right,
                            _ => StatusAlignment::Left,
                        };
                        tokens.push(FormatToken::Align(align));
                    } else if p == "list=on" {
                        tokens.push(FormatToken::ListOn);
                    } else if p == "list=focus" {
                        tokens.push(FormatToken::ListFocus);
                    } else if p == "list=left-marker" {
                        tokens.push(FormatToken::ListLeftMarker);
                    } else if p == "list=right-marker" {
                        tokens.push(FormatToken::ListRightMarker);
                    } else if p == "nolist" || p == "list=off" {
                        tokens.push(FormatToken::NoList);
                    } else if p.starts_with("range=") {
                        let val = &p[6..];
                        if let Some(rest) = val.strip_prefix("window|") {
                            if let Ok(n) = rest.parse::<usize>() {
                                tokens.push(FormatToken::Range(StatusRangeType::Window(n)));
                            }
                        }
                    } else if p == "norange" {
                        tokens.push(FormatToken::NoRange);
                    } else {
                        apply_modifier(p, &mut cur_style);
                    }
                }
                i += 2 + end + 1;
                continue;
            }
            // No closing ']' — treat remaining text as literal
            let chunk = &text[i..];
            if !chunk.is_empty() {
                tokens.push(FormatToken::Text(Span::styled(chunk.to_string(), cur_style)));
            }
            break;
        }
        // Literal text until next #[
        let mut j = i;
        while j < bytes.len() && !(bytes[j] == b'#' && j + 1 < bytes.len() && bytes[j + 1] == b'[') {
            j += 1;
        }
        let chunk = &text[i..j];
        if !chunk.is_empty() {
            tokens.push(FormatToken::Text(Span::styled(chunk.to_string(), cur_style)));
        }
        i = j;
    }
    tokens
}

/// Lay out a status format line with full range tracking.
///
/// This is the primary entry point for rendering `status-format[]` lines.
/// Returns styled spans fitting within `width` plus clickable range regions.
pub fn layout_format_line(text: &str, width: usize, base_style: Style) -> LayoutResult {
    use unicode_width::UnicodeWidthStr;

    let tokens = parse_format_segments(text, base_style);

    // ── Phase 1: distribute tokens into alignment sections ──

    struct Section {
        spans: Vec<Span<'static>>,
        width: usize,
    }
    impl Section {
        fn new() -> Self { Section { spans: Vec::new(), width: 0 } }
        fn push(&mut self, span: Span<'static>) {
            self.width += spans_visual_width(&[span.clone()]);
            self.spans.push(span);
        }
    }

    let mut left = Section::new();
    let mut centre = Section::new();
    let mut right = Section::new();
    let mut current_align = StatusAlignment::Left;
    let mut fill_style: Option<Style> = None;

    // List tracking
    #[derive(PartialEq, Clone, Copy)]
    enum ListSt { Normal, LeftMarker, RightMarker, InList }
    let mut list_state = ListSt::Normal;
    let mut list_left_marker: Vec<Span<'static>> = Vec::new();
    let mut list_right_marker: Vec<Span<'static>> = Vec::new();
    let mut list_spans: Vec<Span<'static>> = Vec::new();
    let mut list_focus_col: Option<usize> = None;
    let mut list_total_w: usize = 0;
    let mut list_align = StatusAlignment::Left;
    let mut has_list = false;

    // Range tracking: (type, section, in_list, start_col_in_context, end_col_in_context)
    struct RangeRecord {
        range_type: StatusRangeType,
        section: StatusAlignment,
        in_list: bool,
        start_col: usize,
        end_col: usize,
    }
    let mut recorded_ranges: Vec<RangeRecord> = Vec::new();
    // Active range: (type, section, in_list, start_col_in_context)
    let mut active_range: Option<(StatusRangeType, StatusAlignment, bool, usize)> = None;

    fn current_col(left: &Section, centre: &Section, right: &Section,
                   align: StatusAlignment, in_list: bool, list_total_w: usize) -> usize {
        if in_list {
            list_total_w
        } else {
            match align {
                StatusAlignment::Left => left.width,
                StatusAlignment::Centre => centre.width,
                StatusAlignment::Right => right.width,
            }
        }
    }

    for token in &tokens {
        match token {
            FormatToken::Text(span) => {
                let w = UnicodeWidthStr::width(span.content.as_ref());
                match list_state {
                    ListSt::LeftMarker => { list_left_marker.push(span.clone()); }
                    ListSt::RightMarker => { list_right_marker.push(span.clone()); }
                    ListSt::InList => {
                        list_total_w += w;
                        list_spans.push(span.clone());
                    }
                    ListSt::Normal => {
                        match current_align {
                            StatusAlignment::Left => left.push(span.clone()),
                            StatusAlignment::Centre => centre.push(span.clone()),
                            StatusAlignment::Right => right.push(span.clone()),
                        }
                    }
                }
            }
            FormatToken::Align(a) => { current_align = *a; }
            FormatToken::Fill(s) => { fill_style = Some(*s); }
            FormatToken::ListLeftMarker => {
                list_state = ListSt::LeftMarker;
                list_align = current_align;
                has_list = true;
            }
            FormatToken::ListRightMarker => { list_state = ListSt::RightMarker; }
            FormatToken::ListOn => {
                list_state = ListSt::InList;
                if !has_list { list_align = current_align; has_list = true; }
            }
            FormatToken::ListFocus => {
                if list_state == ListSt::InList { list_focus_col = Some(list_total_w); }
            }
            FormatToken::NoList => { list_state = ListSt::Normal; }
            FormatToken::Range(rt) => {
                let col = current_col(&left, &centre, &right, current_align,
                                      list_state == ListSt::InList, list_total_w);
                active_range = Some((rt.clone(), current_align, list_state == ListSt::InList, col));
            }
            FormatToken::NoRange => {
                if let Some((rt, sec, in_list, start_col)) = active_range.take() {
                    let end_col = current_col(&left, &centre, &right, sec, in_list, list_total_w);
                    recorded_ranges.push(RangeRecord {
                        range_type: rt, section: sec, in_list, start_col, end_col,
                    });
                }
            }
        }
    }
    // Close any unclosed range
    if let Some((rt, sec, in_list, start_col)) = active_range.take() {
        let end_col = current_col(&left, &centre, &right, sec, in_list, list_total_w);
        recorded_ranges.push(RangeRecord {
            range_type: rt, section: sec, in_list, start_col, end_col,
        });
    }

    // ── Phase 2: Merge list into its alignment section ──

    let lm_w = spans_visual_width(&list_left_marker);
    let rm_w = spans_visual_width(&list_right_marker);

    let other_w = match list_align {
        StatusAlignment::Left => centre.width + right.width,
        StatusAlignment::Centre => left.width + right.width,
        StatusAlignment::Right => left.width + centre.width,
    };
    let list_sec_pre_w = match list_align {
        StatusAlignment::Left => left.width,
        StatusAlignment::Centre => centre.width,
        StatusAlignment::Right => right.width,
    };
    let avail = width.saturating_sub(other_w).saturating_sub(list_sec_pre_w);
    let list_insert_offset = list_sec_pre_w; // column within section where list starts

    // Variables to track viewport offset for range adjustment
    let mut list_viewport_start: usize = 0;
    let mut list_rendered_offset: usize = list_insert_offset;

    let list_section = match list_align {
        StatusAlignment::Left => &mut left,
        StatusAlignment::Centre => &mut centre,
        StatusAlignment::Right => &mut right,
    };

    if has_list {
        if list_total_w <= avail {
            // List fits
            for s in &list_spans {
                list_section.push(s.clone());
            }
        } else {
            // Overflow
            let focus = list_focus_col.unwrap_or(0);
            let (vp_start, show_left, show_right) = {
                let a_rm = avail.saturating_sub(rm_w);
                let a_lm = avail.saturating_sub(lm_w);
                let a_both = avail.saturating_sub(lm_w + rm_w);
                if focus <= a_rm / 2 {
                    (0usize, false, true)
                } else if focus >= list_total_w.saturating_sub(a_lm / 2) {
                    (list_total_w.saturating_sub(a_lm), true, false)
                } else {
                    (focus.saturating_sub(a_both / 2), true, true)
                }
            };
            list_viewport_start = vp_start;

            let vp_w = avail
                .saturating_sub(if show_left { lm_w } else { 0 })
                .saturating_sub(if show_right { rm_w } else { 0 });

            if show_left {
                for s in &list_left_marker { list_section.push(s.clone()); }
                list_rendered_offset = list_insert_offset + lm_w;
            }
            let visible = extract_span_range(&list_spans, vp_start, vp_w);
            for s in &visible { list_section.push(s.clone()); }
            if show_right {
                for s in &list_right_marker { list_section.push(s.clone()); }
            }
        }
    }

    // ── Phase 3: Position and assemble ──

    let fill_s = fill_style.unwrap_or(base_style);
    let mut result_spans: Vec<Span<'static>> = Vec::new();

    let left_w = left.width;
    let centre_w = centre.width;
    let right_w = right.width;

    let right_start = width.saturating_sub(right_w);
    let centre_start = if centre_w > 0 {
        let gap = right_start.saturating_sub(left_w);
        left_w + gap.saturating_sub(centre_w) / 2
    } else { 0 };

    // Left
    result_spans.extend(left.spans);
    if centre_w > 0 {
        let gap1 = centre_start.saturating_sub(left_w);
        if gap1 > 0 { result_spans.push(Span::styled(" ".repeat(gap1), fill_s)); }
        result_spans.extend(centre.spans);
        let gap2 = right_start.saturating_sub(centre_start + centre_w);
        if gap2 > 0 { result_spans.push(Span::styled(" ".repeat(gap2), fill_s)); }
    } else {
        let gap = right_start.saturating_sub(left_w);
        if gap > 0 { result_spans.push(Span::styled(" ".repeat(gap), fill_s)); }
    }
    result_spans.extend(right.spans);

    // Pad remainder
    let total = spans_visual_width(&result_spans);
    if total < width {
        result_spans.push(Span::styled(" ".repeat(width - total), fill_s));
    }
    truncate_spans_to_width(&mut result_spans, width);

    // ── Phase 4: Resolve ranges to absolute columns ──

    let section_start = |align: StatusAlignment| -> usize {
        match align {
            StatusAlignment::Left => 0,
            StatusAlignment::Centre => centre_start,
            StatusAlignment::Right => right_start,
        }
    };

    let mut ranges: Vec<(StatusRangeType, u16, u16)> = Vec::new();
    for rr in &recorded_ranges {
        if rr.in_list {
            // Range is within the list. Adjust for viewport offset.
            let base = section_start(rr.section) + list_rendered_offset;
            let start = base + rr.start_col.saturating_sub(list_viewport_start);
            let end = base + rr.end_col.saturating_sub(list_viewport_start);
            // Clamp to visible area
            let vis_end = section_start(rr.section) + match rr.section {
                StatusAlignment::Left => left_w,
                StatusAlignment::Centre => centre_w,
                StatusAlignment::Right => right_w,
            };
            let clamped_start = start.min(vis_end);
            let clamped_end = end.min(vis_end);
            if clamped_start < clamped_end {
                ranges.push((rr.range_type.clone(), clamped_start as u16, clamped_end as u16));
            }
        } else {
            let base = section_start(rr.section);
            let start = base + rr.start_col;
            let end = base + rr.end_col;
            if start < end {
                ranges.push((rr.range_type.clone(), start as u16, end as u16));
            }
        }
    }

    LayoutResult { spans: result_spans, ranges }
}
