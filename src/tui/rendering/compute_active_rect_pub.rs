#[allow(unused_imports)]

use std::io::{self, Write};
use std::env;
use ratatui::prelude::*;
use ratatui::widgets::*;
use ratatui::style::{Style, Modifier};
use unicode_width::UnicodeWidthStr;
use crossterm::style::Print;
use crossterm::execute;
use portable_pty::PtySize;

use crate::types::{AppState, Mode, Node, LayoutKind};
use crate::tree::split_with_gaps;

// Re-export style utilities so existing `use crate::rendering::*` still works.
pub use crate::style::{
    map_color, parse_tmux_style, parse_inline_styles,
};

// ─── VT color helpers ───────────────────────────────────────────────────────

use super::*;

/// Public version of `compute_active_rect` for use outside the rendering module
/// (e.g. accessibility caret updates).
pub fn compute_active_rect_pub(node: &Node, active_path: &[usize], area: Rect) -> Option<Rect> {
    match node {
        Node::Leaf(_) => Some(area),
        Node::Split { kind, sizes, children } => {
            if active_path.is_empty() || children.is_empty() { return None; }
            let idx = active_path[0];
            if idx >= children.len() { return None; }
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else {
                vec![100 / children.len().max(1) as u16; children.len()]
            };
            let is_horizontal = *kind == LayoutKind::Horizontal;
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
            if idx < rects.len() {
                compute_active_rect(&children[idx], &active_path[1..], rects[idx])
            } else {
                None
            }
        }
    }
}

// ─── Status bar convenience wrappers (delegate to style.rs) ─────────────────

/// Expand simple status variables using AppState context.
pub fn expand_status(fmt: &str, app: &AppState, time_str: &str) -> String {
    let window = &app.windows[app.active_idx];
    let win_idx = app.active_idx + app.window_base_index;
    crate::style::expand_status(fmt, &app.session_name, &window.name, win_idx, time_str)
}

/// Parse a status format string with AppState context into styled spans.
pub fn parse_status(fmt: &str, app: &AppState, time_str: &str) -> Vec<Span<'static>> {
    let window = &app.windows[app.active_idx];
    let win_idx = app.active_idx + app.window_base_index;
    crate::style::parse_status(fmt, &app.session_name, &window.name, win_idx, time_str)
}

// ─── UI layout helpers ──────────────────────────────────────────────────────

pub fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    // Clamp requested height to the available area so we never
    // produce a Rect that extends beyond the buffer.
    let clamped_h = height.min(r.height);
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Length(clamped_h),
            Constraint::Percentage(50),
        ])
        .split(r);
    let middle = popup_layout[1];
    let width = (middle.width * percent_x) / 100;
    let x = middle.x + (middle.width - width) / 2;
    // Use the Layout-allocated height, not the raw parameter,
    // to guarantee the rect stays within the parent area.
    let final_h = middle.height.min(clamped_h);
    Rect { x, y: middle.y, width, height: final_h }
}
