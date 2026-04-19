//! Shared color and style parsing utilities.
//!
//! This module consolidates ALL tmux-compatible color/style parsing into a
//! single place, eliminating duplication between rendering.rs and client.rs.
//! Both the server-side renderer and the remote client import from here.

use ratatui::prelude::*;
use ratatui::style::{Style, Modifier};

use crate::debug_log::style_log;

// ─── Color mapping ──────────────────────────────────────────────────────────

/// Map a tmux color name/hex/index string to a ratatui `Color`.
///
/// Supports: named colors, `brightX`, `colourN`/`colorN`, `#RRGGBB`,
/// `idx:N`, `rgb:R,G,B`, and `default`/`terminal`.
pub fn map_color(name: &str) -> Color {
    let name = name.trim();
    // idx:N (psmux custom)
    if let Some(idx_str) = name.strip_prefix("idx:") {
        if let Ok(idx) = idx_str.parse::<u8>() {
            return Color::Indexed(idx);
        }
    }
    // rgb:R,G,B (psmux custom)
    if let Some(rgb_str) = name.strip_prefix("rgb:") {
        let parts: Vec<&str> = rgb_str.split(',').collect();
        if parts.len() == 3 {
            if let (Ok(r), Ok(g), Ok(b)) = (parts[0].parse::<u8>(), parts[1].parse::<u8>(), parts[2].parse::<u8>()) {
                return Color::Rgb(r, g, b);
            }
        }
    }
    // #RRGGBB hex
    if let Some(hex_str) = name.strip_prefix('#') {
        if hex_str.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex_str[0..2], 16),
                u8::from_str_radix(&hex_str[2..4], 16),
                u8::from_str_radix(&hex_str[4..6], 16),
            ) {
                return Color::Rgb(r, g, b);
            }
        }
    }
    // colour0-colour255 / color0-color255 (tmux primary indexed color format)
    let lower = name.to_lowercase();
    if let Some(idx_str) = lower.strip_prefix("colour").or_else(|| lower.strip_prefix("color")) {
        if let Ok(idx) = idx_str.parse::<u8>() {
            return Color::Indexed(idx);
        }
    }
    match lower.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "brightblack" | "bright-black" => Color::DarkGray,
        "brightred" | "bright-red" => Color::LightRed,
        "brightgreen" | "bright-green" => Color::LightGreen,
        "brightyellow" | "bright-yellow" => Color::LightYellow,
        "brightblue" | "bright-blue" => Color::LightBlue,
        "brightmagenta" | "bright-magenta" => Color::LightMagenta,
        "brightcyan" | "bright-cyan" => Color::LightCyan,
        "brightwhite" | "bright-white" => Color::White,
        "default" | "terminal" => Color::Reset,
        _ => Color::Reset,
    }
}

/// Parse a tmux color name to an `Option<Color>`.
///
/// Returns `Some(Color::Reset)` for "default" (meaning "terminal default").
/// Returns `None` for empty strings (meaning "not specified / inherit").
/// This is the variant used by the remote client where `None` means "keep
/// the existing color" and `Some(Color::Reset)` means "explicitly reset to
/// terminal default".
pub fn parse_tmux_color(s: &str) -> Option<Color> {
    match s.trim().to_lowercase().as_str() {
        "" => None,
        "default" | "terminal" => Some(Color::Reset),
        _ => {
            let c = map_color(s);
            if c == Color::Reset { None } else { Some(c) }
        }
    }
}

// ─── Style parsing ──────────────────────────────────────────────────────────

/// Parse a tmux style string (e.g. `"bg=green,fg=black,bold"`) into a ratatui `Style`.
///
/// Used for status-style, pane-border-style, message-style, mode-style, etc.
pub fn parse_tmux_style(style_str: &str) -> Style {
    let mut style = Style::default();
    if style_str.is_empty() { return style; }
    for part in style_str.split(',') {
        let p = part.trim();
        if p.starts_with("fg=") { style = style.fg(map_color(&p[3..])); }
        else if p.starts_with("bg=") { style = style.bg(map_color(&p[3..])); }
        else { apply_modifier(p, &mut style); }
    }
    style
}

/// Parse a tmux style string into `(Option<fg>, Option<bg>, bold)` tuple.
///
/// This is the decomposed variant used by the remote client where it needs
/// individual components to merge into existing styles.
pub fn parse_tmux_style_components(style: &str) -> (Option<Color>, Option<Color>, bool) {
    let mut fg = None;
    let mut bg = None;
    let mut bold = false;
    for part in style.split(',') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix("fg=") {
            fg = parse_tmux_color(val);
        } else if let Some(val) = part.strip_prefix("bg=") {
            bg = parse_tmux_color(val);
        } else if part == "bold" {
            bold = true;
        } else if part == "nobold" {
            bold = false;
        }
    }
    (fg, bg, bold)
}

/// Apply a modifier token (e.g. "bold", "nobold", "italic") to a `Style`.
fn apply_modifier(token: &str, style: &mut Style) {
    match token {
        "bold" => { *style = style.add_modifier(Modifier::BOLD); }
        "dim" => { *style = style.add_modifier(Modifier::DIM); }
        "italic" | "italics" => { *style = style.add_modifier(Modifier::ITALIC); }
        "underline" | "underscore" => { *style = style.add_modifier(Modifier::UNDERLINED); }
        "blink" => { *style = style.add_modifier(Modifier::SLOW_BLINK); }
        "reverse" => { *style = style.add_modifier(Modifier::REVERSED); }
        "hidden" => { *style = style.add_modifier(Modifier::HIDDEN); }
        "strikethrough" => { *style = style.add_modifier(Modifier::CROSSED_OUT); }
        "overline" => { /* ratatui doesn't support overline natively */ }
        "double-underscore" | "curly-underscore" | "dotted-underscore" | "dashed-underscore" => {
            *style = style.add_modifier(Modifier::UNDERLINED);
        }
        "default" | "none" => { *style = Style::default(); }
        "nobold" => { *style = style.remove_modifier(Modifier::BOLD); }
        "nodim" => { *style = style.remove_modifier(Modifier::DIM); }
        "noitalics" | "noitalic" => { *style = style.remove_modifier(Modifier::ITALIC); }
        "nounderline" | "nounderscore" => { *style = style.remove_modifier(Modifier::UNDERLINED); }
        "noblink" => { *style = style.remove_modifier(Modifier::SLOW_BLINK); }
        "noreverse" => { *style = style.remove_modifier(Modifier::REVERSED); }
        "nohidden" => { *style = style.remove_modifier(Modifier::HIDDEN); }
        "nostrikethrough" => { *style = style.remove_modifier(Modifier::CROSSED_OUT); }
        _ => {}
    }
}

// ─── Inline style parsing ───────────────────────────────────────────────────

/// Parse inline `#[fg=...,bg=...,bold]` style directives from pre-expanded text.
///
/// Unlike `parse_status()`, this does NOT re-expand status variables.
/// Use for text already expanded by the format engine (e.g. window tab labels).
///
/// Supports tmux-compatible tokens:
/// - `fg=color`, `bg=color` — set foreground/background
/// - `bold`, `dim`, `italic`, `underline`, `blink`, `reverse`, `strikethrough`
/// - `nobold`, `nodim`, etc. — remove modifiers
/// - `default`, `none` — reset to base style
/// - `push-default` — push current style onto stack
/// - `pop-default` — pop style from stack
/// - `fill` — recognised but handled by caller (ignored here)
/// - `list=on`, `list=left`, `list=right`, `nolist` — window list markers (ignored here)
/// - `range=...`, `norange` — mouse range markers (ignored here)
/// - `align=left`, `align=centre`, `align=right` — alignment markers (ignored here)
pub fn parse_inline_styles(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur_style = base_style;
    let mut style_stack: Vec<Style> = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some(end) = text[i + 2..].find(']') {
                let token = &text[i + 2..i + 2 + end];
                for part in token.split(',') {
                    let p = part.trim();
                    if p.starts_with("fg=") { cur_style = cur_style.fg(map_color(&p[3..])); }
                    else if p.starts_with("bg=") { cur_style = cur_style.bg(map_color(&p[3..])); }
                    else if p == "default" || p == "none" { cur_style = base_style; }
                    else if p == "push-default" { style_stack.push(cur_style); }
                    else if p == "pop-default" {
                        if let Some(s) = style_stack.pop() { cur_style = s; }
                        else { cur_style = base_style; }
                    }
                    // Recognised but handled at a higher level — silently skip
                    else if p == "fill" || p.starts_with("list") || p == "nolist"
                         || p.starts_with("range") || p == "norange"
                         || p.starts_with("align") {}
                    else { apply_modifier(p, &mut cur_style); }
                }
                i += 2 + end + 1;
                continue;
            }
            // No closing ']' found — treat remaining text as literal
            style_log("parse_inline", &format!("WARN: unclosed #[ at pos {} in: [{}]",
                i, text.chars().take(120).collect::<String>()));
            let chunk = &text[i..];
            if !chunk.is_empty() {
                spans.push(Span::styled(chunk.to_string(), cur_style));
            }
            break;
        }
        let mut j = i;
        while j < bytes.len() && !(bytes[j] == b'#' && j + 1 < bytes.len() && bytes[j + 1] == b'[') {
            j += 1;
        }
        let chunk = &text[i..j];
        if !chunk.is_empty() {
            spans.push(Span::styled(chunk.to_string(), cur_style));
        }
        i = j;
    }
    spans
}

/// Calculate the visual display width of styled spans.
pub fn spans_visual_width(spans: &[Span]) -> usize {
    use unicode_width::UnicodeWidthStr;
    spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum()
}

/// Truncate a list of styled spans so their total visual width fits within
/// `max_width` columns.  If the content exceeds `max_width`, spans are
/// trimmed character by character and a trailing ellipsis is NOT added (to
/// match tmux behaviour).  Returns the mutated vector in place.
pub fn truncate_spans_to_width(spans: &mut Vec<Span<'static>>, max_width: usize) {
    use unicode_width::UnicodeWidthChar;
    let mut remaining = max_width;
    let mut keep = 0;
    for (i, span) in spans.iter().enumerate() {
        let sw = spans_visual_width(&[span.clone()]);
        if sw <= remaining {
            remaining -= sw;
            keep = i + 1;
        } else {
            // Partially truncate this span
            let mut truncated = String::new();
            for ch in span.content.chars() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                if cw > remaining {
                    break;
                }
                remaining -= cw;
                truncated.push(ch);
            }
            if !truncated.is_empty() {
                spans[i] = Span::styled(truncated, span.style);
                keep = i + 1;
            }
            break;
        }
    }
    spans.truncate(keep);
}

// ─── Status bar parsing ─────────────────────────────────────────────────────

/// Expand simple status variables (`#I`, `#W`, `#S`, `%H:%M`) in a fragment.
pub fn expand_status(fmt: &str, session_name: &str, win_name: &str, win_idx: usize, time_str: &str) -> String {
    let mut s = fmt.to_string();
    s = s.replace("#I", &win_idx.to_string());
    s = s.replace("#W", win_name);
    s = s.replace("#S", session_name);
    s = s.replace("%H:%M", time_str);
    s
}

/// Parse a format string with inline `#[style]` directives into styled spans.
///
/// Handles both style tokens and status variable expansion.
pub fn parse_status(fmt: &str, session_name: &str, win_name: &str, win_idx: usize, time_str: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur_style = Style::default();
    let mut i = 0;
    while i < fmt.len() {
        if fmt.as_bytes()[i] == b'#' && i + 1 < fmt.len() && fmt.as_bytes()[i+1] == b'[' {
            if let Some(end) = fmt[i+2..].find(']') {
                let token = &fmt[i+2..i+2+end];
                for part in token.split(',') {
                    let p = part.trim();
                    if p.starts_with("fg=") { cur_style = cur_style.fg(map_color(&p[3..])); }
                    else if p.starts_with("bg=") { cur_style = cur_style.bg(map_color(&p[3..])); }
                    else if p == "default" || p == "none" { cur_style = Style::default(); }
                    else { apply_modifier(p, &mut cur_style); }
                }
                i += 2 + end + 1;
                continue;
            }
            // No closing ']' found — treat remaining text as literal
            style_log("parse_status", &format!("WARN: unclosed #[ at pos {} in: [{}]",
                i, fmt.chars().take(120).collect::<String>()));
            let chunk = &fmt[i..];
            let text = expand_status(chunk, session_name, win_name, win_idx, time_str);
            spans.push(Span::styled(text, cur_style));
            break;
        }
        let mut j = i;
        while j < fmt.len() && !(fmt.as_bytes()[j] == b'#' && j + 1 < fmt.len() && fmt.as_bytes()[j+1] == b'[') { j += 1; }
        let chunk = &fmt[i..j];
        let text = expand_status(chunk, session_name, win_name, win_idx, time_str);
        spans.push(Span::styled(text, cur_style));
        i = j;
    }
    spans
}

// ─── Layout engine for status-format[] directives ───────────────────────────

/// Alignment section for `#[align=...]` directives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusAlignment {
    Left,
    Centre,
    Right,
}

/// Type of a clickable range defined by `#[range=...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusRangeType {
    Window(usize),
}

/// A token from parsing a format string with layout awareness.
#[derive(Debug, Clone)]
pub enum FormatToken {
    /// Visible styled text.
    Text(Span<'static>),
    /// Switch alignment section.
    Align(StatusAlignment),
    /// Fill unused space (records the style at the point of the directive).
    Fill(Style),
    /// Enter list content section.
    ListOn,
    /// Mark the focused item position in the list.
    ListFocus,
    /// Following text is the left overflow marker.
    ListLeftMarker,
    /// Following text is the right overflow marker.
    ListRightMarker,
    /// End list section.
    NoList,
    /// Start a clickable range.
    Range(StatusRangeType),
    /// End clickable range.
    NoRange,
}

/// Result of laying out a status format line.
pub struct LayoutResult {
    pub spans: Vec<Span<'static>>,
    /// Clickable ranges: (type, start_column, end_column).
    pub ranges: Vec<(StatusRangeType, u16, u16)>,
}

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

/// Extract spans from a column range within a list of spans.
///
/// Returns spans whose visible content falls within `[col_start, col_start + max_width)`.
fn extract_span_range(spans: &[Span<'static>], col_start: usize, max_width: usize) -> Vec<Span<'static>> {
    use unicode_width::UnicodeWidthChar;
    let mut result = Vec::new();
    let mut col = 0usize;
    let mut remaining = max_width;

    for span in spans {
        let sw = spans_visual_width(&[span.clone()]);
        if col + sw <= col_start {
            col += sw;
            continue;
        }
        // This span overlaps with our range
        let mut text = String::new();
        for ch in span.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col < col_start {
                col += cw;
                continue;
            }
            if cw > remaining { break; }
            remaining -= cw;
            col += cw;
            text.push(ch);
        }
        if !text.is_empty() {
            result.push(Span::styled(text, span.style));
        }
        if remaining == 0 { break; }
    }
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    /// Issue #164: parse_inline_styles must parse #[fg=red] and apply the style,
    /// NOT render it as literal text.
    #[test]
    fn parse_inline_styles_fg_red() {
        let base = Style::default().fg(Color::White).bg(Color::Black);
        let spans = parse_inline_styles("#[fg=red]Custom Line 2", base);

        // Should produce exactly one span with the visible text (no style directive text)
        assert_eq!(spans.len(), 1, "Expected 1 span, got {:?}", spans);
        assert_eq!(spans[0].content.as_ref(), "Custom Line 2");
        // The style should have fg=Red applied
        assert_eq!(spans[0].style.fg, Some(Color::Red),
            "fg should be Red, got {:?}", spans[0].style.fg);
        // bg should remain from base
        assert_eq!(spans[0].style.bg, Some(Color::Black),
            "bg should remain Black from base, got {:?}", spans[0].style.bg);
    }

    /// Issue #164: #[align=left] should be consumed (not rendered as literal text)
    #[test]
    fn parse_inline_styles_align_left() {
        let base = Style::default();
        let spans = parse_inline_styles("#[align=left]Custom Line 1", base);

        assert_eq!(spans.len(), 1, "Expected 1 span, got {:?}", spans);
        assert_eq!(spans[0].content.as_ref(), "Custom Line 1");
        // No literal "#[align=left]" text should appear
    }

    /// Issue #164: Multiple style directives in one format string
    #[test]
    fn parse_inline_styles_multiple_directives() {
        let base = Style::default();
        let spans = parse_inline_styles("#[fg=red]Hello #[fg=green]World", base);

        assert_eq!(spans.len(), 2, "Expected 2 spans, got {:?}", spans);
        assert_eq!(spans[0].content.as_ref(), "Hello ");
        assert_eq!(spans[0].style.fg, Some(Color::Red));
        assert_eq!(spans[1].content.as_ref(), "World");
        assert_eq!(spans[1].style.fg, Some(Color::Green));
    }

    /// Issue #164: fg+bg combined in one directive
    #[test]
    fn parse_inline_styles_fg_and_bg() {
        let base = Style::default();
        let spans = parse_inline_styles("#[fg=yellow,bg=blue]Styled", base);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "Styled");
        assert_eq!(spans[0].style.fg, Some(Color::Yellow));
        assert_eq!(spans[0].style.bg, Some(Color::Blue));
    }

    /// Issue #164: Plain text without directives passes through unchanged
    #[test]
    fn parse_inline_styles_plain_text() {
        let base = Style::default().fg(Color::White);
        let spans = parse_inline_styles("No styles here", base);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "No styles here");
        assert_eq!(spans[0].style.fg, Some(Color::White));
    }

    /// Issue #164: Empty string produces no spans
    #[test]
    fn parse_inline_styles_empty() {
        let base = Style::default();
        let spans = parse_inline_styles("", base);
        assert!(spans.is_empty());
    }

    /// Issue #182: bg=default should map to Color::Reset (terminal default),
    /// not None (which causes fallback to hardcoded green).
    #[test]
    fn parse_tmux_color_default_returns_reset() {
        let c = parse_tmux_color("default");
        assert_eq!(c, Some(Color::Reset),
            "parse_tmux_color(\"default\") should return Some(Color::Reset), got {:?}", c);
    }

    /// Issue #182: parse_tmux_style_components should propagate bg=default as Some(Color::Reset)
    #[test]
    fn parse_tmux_style_components_bg_default() {
        let (fg, bg, bold) = parse_tmux_style_components("fg=white,bg=default");
        assert_eq!(fg, Some(Color::White));
        assert_eq!(bg, Some(Color::Reset),
            "bg=default should yield Some(Color::Reset), got {:?}", bg);
        assert!(!bold);
    }

    /// Issue #182: map_color("default") should return Color::Reset
    #[test]
    fn map_color_default_is_reset() {
        assert_eq!(map_color("default"), Color::Reset);
        assert_eq!(map_color("terminal"), Color::Reset);
    }

    /// Empty color string should remain None (not specified)
    #[test]
    fn parse_tmux_color_empty_returns_none() {
        assert_eq!(parse_tmux_color(""), None);
    }
}

#[cfg(test)]
#[path = "../tests-rs/test_issue185_layout_directives.rs"]
mod test_issue185_layout_directives;
