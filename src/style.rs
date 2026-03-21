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
/// Returns `None` for "default" or empty strings (meaning "inherit").
/// This is the variant used by the remote client where `None` means "keep
/// the existing color".
pub fn parse_tmux_color(s: &str) -> Option<Color> {
    match s.trim().to_lowercase().as_str() {
        "default" | "" => None,
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
