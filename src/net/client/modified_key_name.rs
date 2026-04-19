#[allow(unused_imports)]
use std::io::{self, Write, BufRead, BufReader};
use std::time::{Duration, Instant};
use std::env;

use chrono::Local;
use crossterm::event::{Event, KeyCode, KeyModifiers, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::layout::LayoutJson;
use crate::help;
use crate::util::{WinTree, base64_encode, quote_arg};
use crate::session::read_session_key;
use crate::rendering::{dim_predictions_enabled, map_color, dim_color, centered_rect, fix_border_intersections};
use crate::style::parse_tmux_style_components;
use crate::config::{parse_key_string, normalize_key_for_binding};
use crate::copy_mode::{copy_to_system_clipboard, read_from_system_clipboard};
use crate::debug_log::{client_log, client_log_enabled, input_log, input_log_enabled};
use crate::layout::RowRunsJson;
use crate::tree::split_with_gaps;

/// Build a send-key name with modifier prefix (e.g. "C-Left", "S-Right", "C-S-Up").
use super::*;

pub(crate) fn modified_key_name(base: &str, mods: KeyModifiers) -> String {
    let mut prefix = String::new();
    if mods.contains(KeyModifiers::CONTROL) { prefix.push_str("C-"); }
    if mods.contains(KeyModifiers::ALT) { prefix.push_str("M-"); }
    if mods.contains(KeyModifiers::SHIFT) { prefix.push_str("S-"); }
    if prefix.is_empty() {
        base.to_lowercase()
    } else {
        format!("{}{}", prefix, base)
    }
}

/// Extract selected text from the layout tree given absolute terminal coordinates.
/// Computes pane areas via the same Layout splitting render_json uses, then reads
/// characters from the run-length-encoded rows_v2 data.
pub(crate) struct PaneLeaf<'a> {
    pub(crate) inner: Rect,
    pub(crate) rows_v2: &'a [RowRunsJson],
}

pub(crate) fn collect_leaves<'a>(node: &'a LayoutJson, area: Rect, out: &mut Vec<PaneLeaf<'a>>) {
    match node {
        LayoutJson::Leaf { rows_v2, .. } => {
            out.push(PaneLeaf { inner: area, rows_v2 });
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
                if i < rects.len() {
                    collect_leaves(child, rects[i], out);
                }
            }
        }
    }
}

/// Get the character at a column position within a row's runs.
///
/// `run.text` may be shorter than `run.width` (single repeated char) or
/// multi-char (wide chars); pick the nth char if present.
pub(crate) fn char_at_col(runs: &[crate::layout::CellRunJson], local_col: usize) -> char {
    let mut cursor = 0usize;
    for run in runs {
        let run_width = run.width.max(1) as usize;
        if local_col >= cursor && local_col < cursor + run_width {
            let offset = local_col - cursor;
            return run.text.chars().nth(offset).unwrap_or(' ');
        }
        cursor += run_width;
    }
    ' '
}

/// Expand a row's runs into a dense `Vec<char>` indexed by local column.
/// Used by hot paths (word-boundary scan) that would otherwise call
/// `char_at_col` O(width) times and pay O(width²) total.
pub(crate) fn row_chars(runs: &[crate::layout::CellRunJson], width: usize) -> Vec<char> {
    let mut out = vec![' '; width];
    let mut cursor = 0usize;
    for run in runs {
        let run_width = run.width.max(1) as usize;
        let chars: Vec<char> = run.text.chars().collect();
        for i in 0..run_width {
            let col = cursor + i;
            if col >= width { break; }
            out[col] = chars.get(i).copied().unwrap_or(' ');
        }
        cursor += run_width;
        if cursor >= width { break; }
    }
    out
}

/// Normalise a selection (start, end) into reading-order or block-mode bounds.
pub(crate) fn normalize_selection(start: (u16, u16), end: (u16, u16), block: bool) -> (u16, u16, u16, u16) {
    if block {
        (start.1.min(end.1), start.0.min(end.0), start.1.max(end.1), start.0.max(end.0))
    } else if (start.1, start.0) <= (end.1, end.0) {
        (start.1, start.0, end.1, end.0)
    } else {
        (end.1, end.0, start.1, start.0)
    }
}

pub(crate) fn extract_selection_text(
    layout: &LayoutJson,
    term_width: u16,
    content_height: u16,
    start: (u16, u16),
    end: (u16, u16),
    block: bool,
) -> String {
    let (r0, c0, r1, c1) = normalize_selection(start, end, block);

    let content_area = Rect { x: 0, y: 0, width: term_width, height: content_height };
    let mut leaves: Vec<PaneLeaf> = Vec::new();
    collect_leaves(layout, content_area, &mut leaves);

    let mut result = String::new();
    for row in r0..=r1 {
        let col_start = if block || row == r0 { c0 } else { 0 };
        let col_end = if block || row == r1 { c1 } else { term_width.saturating_sub(1) };

        let mut line = String::new();
        for col in col_start..=col_end {
            let mut ch = ' ';
            for leaf in &leaves {
                let inner = &leaf.inner;
                if col >= inner.x && col < inner.x + inner.width
                    && row >= inner.y && row < inner.y + inner.height
                {
                    let local_row = (row - inner.y) as usize;
                    let local_col = (col - inner.x) as usize;
                    if local_row < leaf.rows_v2.len() {
                        ch = char_at_col(&leaf.rows_v2[local_row].runs, local_col);
                    }
                    break;
                }
            }
            line.push(ch);
        }
        let trimmed = line.trim_end();
        result.push_str(trimmed);
        if row < r1 {
            result.push('\n');
        }
    }

    result
}

/// Check if the active pane is running a fullscreen TUI app (alternate screen).
/// Used to decide whether right-click should paste (shell prompt) or forward
/// as a mouse event to the child (TUI app like htop, Claude Code, etc.).
pub(crate) fn active_pane_in_alt_screen(layout: &LayoutJson) -> bool {
    match layout {
        LayoutJson::Leaf { active, alternate_screen, .. } => *active && *alternate_screen,
        LayoutJson::Split { children, .. } => children.iter().any(|c| active_pane_in_alt_screen(c)),
    }
}

/// Check if the active pane is in server-side copy mode.
/// When true, the client should NOT start its own text selection —
/// the server handles cursor positioning and selection in copy mode.
pub(crate) fn active_pane_in_copy_mode(layout: &LayoutJson) -> bool {
    match layout {
        LayoutJson::Leaf { active, copy_mode, .. } => *active && *copy_mode,
        LayoutJson::Split { children, .. } => children.iter().any(|c| active_pane_in_copy_mode(c)),
    }
}

pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Find the (start_col, end_col) of the word at `(col, row)` inside the
/// given pane. Returns None when the cell is not a word character.
///
/// `layout` is walked to resolve the clicked leaf's `rows_v2` — the caller
/// already knows `pane_rect`, but it does not have a handle to the raw
/// content, so we do a single targeted descent.
pub(crate) fn word_bounds_at(
    layout: &LayoutJson,
    term_width: u16,
    content_height: u16,
    pane_rect: Rect,
    col: u16,
    row: u16,
) -> Option<(u16, u16)> {
    let content_area = Rect { x: 0, y: 0, width: term_width, height: content_height };
    let mut leaves: Vec<PaneLeaf> = Vec::new();
    collect_leaves(layout, content_area, &mut leaves);

    let leaf = leaves.iter().find(|l| l.inner == pane_rect)?;

    let local_row = row.checked_sub(leaf.inner.y)? as usize;
    if local_row >= leaf.rows_v2.len() { return None; }
    let width = leaf.inner.width as usize;
    let chars = row_chars(&leaf.rows_v2[local_row].runs, width);

    let local_col = col.checked_sub(leaf.inner.x)? as usize;
    if local_col >= width { return None; }
    if !is_word_char(chars[local_col]) { return None; }

    let mut left = local_col;
    while left > 0 && is_word_char(chars[left - 1]) {
        left -= 1;
    }
    let mut right = local_col;
    while right + 1 < width && is_word_char(chars[right + 1]) {
        right += 1;
    }

    Some((leaf.inner.x + left as u16, leaf.inner.x + right as u16))
}

/// Check if screen coordinates (x, y) fall on a separator line in the layout.
/// Used to distinguish border-drag (resize) from text selection on left-click.
pub(crate) fn is_on_separator(layout: &LayoutJson, area: Rect, x: u16, y: u16) -> bool {
    match layout {
        LayoutJson::Leaf { .. } => false,
        LayoutJson::Split { kind, sizes, children } => {
            let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                sizes.clone()
            } else {
                vec![(100 / children.len().max(1)) as u16; children.len()]
            };
            let is_horizontal = kind == "Horizontal";
            let rects = split_with_gaps(is_horizontal, &effective_sizes, area);

            // Check if (x, y) is on any separator between children
            for i in 0..children.len().saturating_sub(1) {
                if i >= rects.len() { break; }
                if is_horizontal {
                    let sep_x = rects[i].x + rects[i].width;
                    if x == sep_x && y >= area.y && y < area.y + area.height {
                        return true;
                    }
                } else {
                    let sep_y = rects[i].y + rects[i].height;
                    if y == sep_y && x >= area.x && x < area.x + area.width {
                        return true;
                    }
                }
            }

            // Recurse into children
            for (i, child) in children.iter().enumerate() {
                if i < rects.len() && is_on_separator(child, rects[i], x, y) {
                    return true;
                }
            }

            false
        }
    }
}

/// Collect all leaf pane IDs and their absolute rects from a LayoutJson tree.
pub(crate) fn collect_pane_rects(node: &LayoutJson, area: Rect, out: &mut Vec<(usize, Rect)>) {
    match node {
        LayoutJson::Leaf { id, .. } => {
            out.push((*id, area));
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
                if i < rects.len() {
                    collect_pane_rects(child, rects[i], out);
                }
            }
        }
    }
}

/// Collect all split border positions from a LayoutJson tree.
/// Returns: (tree_path_to_parent, kind, child_index, border_pixel_pos, total_pixels, sizes_snapshot)
pub(crate) fn collect_layout_borders(
    node: &LayoutJson,
    area: Rect,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, String, usize, u16, u16, Vec<u16>, Rect)>,
) {
    if let LayoutJson::Split { kind, sizes, children } = node {
        let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
            sizes.clone()
        } else {
            vec![(100 / children.len().max(1)) as u16; children.len()]
        };
        let is_horizontal = kind == "Horizontal";
        let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
        let total_px = if is_horizontal { area.width } else { area.height };
        for i in 0..children.len().saturating_sub(1) {
            if i < rects.len() {
                let pos = if is_horizontal {
                    rects[i].x + rects[i].width
                } else {
                    rects[i].y + rects[i].height
                };
                out.push((path.clone(), kind.clone(), i, pos, total_px, effective_sizes.clone(), area));
            }
        }
        for (i, child) in children.iter().enumerate() {
            if i < rects.len() {
                path.push(i);
                collect_layout_borders(child, rects[i], path, out);
                path.pop();
            }
        }
    }
}

/// Check if any leaf in a LayoutJson subtree is the active pane.
/// Compute the rectangle of the active pane by searching the LayoutJson tree.
pub(crate) fn compute_active_rect_json(node: &LayoutJson, area: Rect) -> Option<Rect> {
    match node {
        LayoutJson::Leaf { active, .. } => {
            if *active { Some(area) } else { None }
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
                if i < rects.len() {
                    if let Some(r) = compute_active_rect_json(child, rects[i]) {
                        return Some(r);
                    }
                }
            }
            None
        }
    }
}

/// Client-side border drag state — tracks an in-progress separator resize.
pub(crate) struct ClientDragState {
    pub(crate) path: Vec<usize>,
    pub(crate) kind: String,
    pub(crate) index: usize,
    pub(crate) start_pos: u16,
    pub(crate) initial_sizes: Vec<u16>,
    pub(crate) total_pixels: u16,
}
