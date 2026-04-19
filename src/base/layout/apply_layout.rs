#[allow(unused_imports)]
use std::io;

use serde::{Serialize, Deserialize};
use unicode_width::UnicodeWidthStr;

use crate::types::{AppState, Node, LayoutKind, Mode};
use crate::tree::get_split_mut;

/// Serialize a vt100 screen region into run-length-encoded rows (rows_v2 format).
///
/// This is the shared serialization used by both pane layout rendering and popup
/// overlay rendering.  Extracts cells from [0..rows) x [0..cols), merges
/// adjacent cells with identical styling into runs, and returns the result
/// as a `Vec<RowRunsJson>`.
use super::*;

/// Apply a named layout to the current window.
/// Collects ALL leaf panes and rebuilds the tree structure from scratch.
pub fn apply_layout(app: &mut AppState, layout: &str) {
    let win = &mut app.windows[app.active_idx];
    
    // Collect all leaf panes from the current tree
    let old_root = std::mem::replace(&mut win.root, Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
    let mut leaves = crate::tree::collect_leaves(old_root);
    let pane_count = leaves.len();
    if pane_count < 2 {
        // Put back the single leaf (or empty)
        if let Some(leaf) = leaves.into_iter().next() {
            win.root = leaf;
        }
        return;
    }

    // Helper: compute equal sizes summing to 100
    fn equal_sizes(n: usize) -> Vec<u16> {
        if n == 0 { return vec![]; }
        let base = 100 / n as u16;
        let mut sizes = vec![base; n];
        let rem = 100 - base * n as u16;
        if let Some(last) = sizes.last_mut() { *last += rem; }
        sizes
    }

    // Determine main-pane percentage
    let main_h_pct = if app.main_pane_height > 0 { app.main_pane_height.min(95) } else { 60 };
    let main_v_pct = if app.main_pane_width > 0 { app.main_pane_width.min(95) } else { 60 };

    match layout.to_lowercase().as_str() {
        "even-horizontal" | "even-h" => {
            // Single horizontal split with N equal children
            let sizes = equal_sizes(pane_count);
            win.root = Node::Split { kind: LayoutKind::Horizontal, sizes, children: leaves };
        }
        "even-vertical" | "even-v" => {
            // Single vertical split with N equal children
            let sizes = equal_sizes(pane_count);
            win.root = Node::Split { kind: LayoutKind::Vertical, sizes, children: leaves };
        }
        "main-horizontal" | "main-h" => {
            // Vertical split: top pane (main) + bottom horizontal split of remaining
            let main_pane = leaves.remove(0);
            if leaves.len() == 1 {
                let other = leaves.remove(0);
                win.root = Node::Split {
                    kind: LayoutKind::Vertical,
                    sizes: vec![main_h_pct, 100 - main_h_pct],
                    children: vec![main_pane, other],
                };
            } else {
                let bottom_sizes = equal_sizes(leaves.len());
                let bottom = Node::Split { kind: LayoutKind::Horizontal, sizes: bottom_sizes, children: leaves };
                win.root = Node::Split {
                    kind: LayoutKind::Vertical,
                    sizes: vec![main_h_pct, 100 - main_h_pct],
                    children: vec![main_pane, bottom],
                };
            }
        }
        "main-vertical" | "main-v" => {
            // Horizontal split: left pane (main) + right vertical split of remaining
            let main_pane = leaves.remove(0);
            if leaves.len() == 1 {
                let other = leaves.remove(0);
                win.root = Node::Split {
                    kind: LayoutKind::Horizontal,
                    sizes: vec![main_v_pct, 100 - main_v_pct],
                    children: vec![main_pane, other],
                };
            } else {
                let right_sizes = equal_sizes(leaves.len());
                let right = Node::Split { kind: LayoutKind::Vertical, sizes: right_sizes, children: leaves };
                win.root = Node::Split {
                    kind: LayoutKind::Horizontal,
                    sizes: vec![main_v_pct, 100 - main_v_pct],
                    children: vec![main_pane, right],
                };
            }
        }
        "tiled" => {
            // Balanced binary tree of splits
            fn build_tiled(mut panes: Vec<Node>) -> Node {
                if panes.len() == 1 { return panes.remove(0); }
                if panes.len() == 2 {
                    return Node::Split {
                        kind: LayoutKind::Horizontal,
                        sizes: vec![50, 50],
                        children: panes,
                    };
                }
                let mid = panes.len() / 2;
                let right_panes = panes.split_off(mid);
                let left = build_tiled(panes);
                let right = build_tiled(right_panes);
                // Alternate between vertical and horizontal at each level
                Node::Split {
                    kind: LayoutKind::Vertical,
                    sizes: vec![50, 50],
                    children: vec![left, right],
                }
            }
            win.root = build_tiled(leaves);
        }
        _ => {
            // Unknown layout name — try to parse as tmux layout string
            let new_root = parse_tmux_layout_string(layout, &mut leaves);
            if let Some(root) = new_root {
                win.root = root;
            } else {
                // Parsing failed; put panes back as even-horizontal fallback
                let sizes = equal_sizes(pane_count);
                win.root = Node::Split { kind: LayoutKind::Horizontal, sizes, children: leaves };
            }
        }
    }
    // Reset active_path to first leaf
    win.active_path = crate::tree::first_leaf_path(&win.root);
}

pub(crate) const LAYOUT_NAMES: [&str; 5] = ["even-horizontal", "even-vertical", "main-horizontal", "main-vertical", "tiled"];

/// Cycle through available layouts (forward)
pub fn cycle_layout(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    if matches!(win.root, Node::Leaf(_)) { return; }
    let next_idx = (win.layout_index + 1) % LAYOUT_NAMES.len();
    win.layout_index = next_idx;
    apply_layout(app, LAYOUT_NAMES[next_idx]);
}

/// Cycle through available layouts (reverse)
pub fn cycle_layout_reverse(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    if matches!(win.root, Node::Leaf(_)) { return; }
    let prev_idx = (win.layout_index + LAYOUT_NAMES.len() - 1) % LAYOUT_NAMES.len();
    win.layout_index = prev_idx;
    apply_layout(app, LAYOUT_NAMES[prev_idx]);
}

/// Parse a tmux layout string into a Node tree.
///
/// Format: `checksum,WxH,X,Y{child1,child2,...}` or `checksum,WxH,X,Y[child1,child2,...]`
/// - `{...}` = horizontal split (children side-by-side)
/// - `[...]` = vertical split (children stacked)
/// - Each child is either a leaf `WxH,X,Y,pane_id` or a nested split `WxH,X,Y{...}` / `WxH,X,Y[...]`
///
/// The `panes` vec provides existing pane nodes to fill the tree leaves.
/// Returns `None` if parsing fails.
/// Parsed layout node from a tmux layout string.
/// This is a layout descriptor that can be inspected, counted, and applied
/// to existing panes without requiring pane objects during parsing.
#[derive(Debug, Clone)]
pub enum LayoutNode {
    Leaf { width: u16, height: u16, x: u16, y: u16, pane_id: Option<usize> },
    Split { kind: LayoutKind, width: u16, height: u16, x: u16, y: u16, children: Vec<LayoutNode> },
}

impl LayoutNode {
    /// Count the number of leaf panes in this layout tree.
    pub fn count_leaves(&self) -> usize {
        match self {
            LayoutNode::Leaf { .. } => 1,
            LayoutNode::Split { children, .. } => children.iter().map(|c| c.count_leaves()).sum(),
        }
    }

    pub(crate) fn width(&self) -> u16 {
        match self { LayoutNode::Leaf { width, .. } | LayoutNode::Split { width, .. } => *width }
    }

    pub(crate) fn height(&self) -> u16 {
        match self { LayoutNode::Leaf { height, .. } | LayoutNode::Split { height, .. } => *height }
    }
}

/// Parse a tmux layout string into a `LayoutNode` descriptor tree.
///
/// Layout string format: `CHECKSUM,WxH,X,Y{...}` or `[...]` or `,PANE_ID`
/// The 4-hex-digit checksum prefix is skipped.
pub fn parse_layout_string(layout_str: &str) -> Option<LayoutNode> {
    let s = layout_str.trim();
    if s.len() < 5 { return None; }
    // Validate and skip the 4-hex-char checksum prefix followed by comma.
    // tmux checksums are exactly 4 hex digits (e.g. "5e08,").
    let bytes = s.as_bytes();
    if bytes.len() < 5 || bytes[4] != b',' { return None; }
    for &b in &bytes[..4] {
        if !b.is_ascii_hexdigit() { return None; }
    }
    let body = &s[5..];
    let (node, _) = parse_layout_node(body)?;
    Some(node)
}

/// Parse a tmux layout string into a Node tree using existing panes.
///
/// Parses the layout string into a LayoutNode descriptor, then converts
/// it to a Node tree by assigning panes from the provided vec in leaf order.
/// Returns `None` if parsing fails or there aren't enough panes.
pub fn parse_tmux_layout_string(layout_str: &str, panes: &mut Vec<Node>) -> Option<Node> {
    let layout = parse_layout_string(layout_str)?;
    layout_node_to_node(&layout, panes)
}

/// Convert a LayoutNode descriptor tree into a Node tree,
/// consuming panes from the vec in left-to-right leaf order.
pub(crate) fn layout_node_to_node(layout: &LayoutNode, panes: &mut Vec<Node>) -> Option<Node> {
    match layout {
        LayoutNode::Leaf { .. } => {
            if panes.is_empty() { return None; }
            Some(panes.remove(0))
        }
        LayoutNode::Split { kind, children, .. } => {
            let total_size: u32 = match kind {
                LayoutKind::Horizontal => children.iter().map(|c| c.width() as u32).sum(),
                LayoutKind::Vertical => children.iter().map(|c| c.height() as u32).sum(),
            };
            let sizes: Vec<u16> = if total_size == 0 {
                let n = children.len().max(1) as u16;
                vec![100 / n; children.len()]
            } else {
                let mut szs: Vec<u16> = children.iter().map(|c| {
                    let dim = match kind {
                        LayoutKind::Horizontal => c.width() as u32,
                        LayoutKind::Vertical => c.height() as u32,
                    };
                    (dim * 100 / total_size) as u16
                }).collect();
                let sum: u16 = szs.iter().sum();
                if sum < 100 { if let Some(last) = szs.last_mut() { *last += 100 - sum; } }
                szs
            };
            let mut nodes = Vec::with_capacity(children.len());
            for child in children {
                nodes.push(layout_node_to_node(child, panes)?);
            }
            Some(Node::Split { kind: *kind, sizes, children: nodes })
        }
    }
}

/// Parse a single layout node from position in the string, returns (LayoutNode, chars_consumed).
pub(crate) fn parse_layout_node(s: &str) -> Option<(LayoutNode, usize)> {
    let (w, h, x, y, consumed_dims) = parse_dimensions(s)?;
    let rest = &s[consumed_dims..];

    if rest.starts_with('{') {
        // Horizontal split (children side-by-side)
        let (children, consumed_bracket) = parse_layout_children(&rest[1..], '}')?;
        Some((
            LayoutNode::Split { kind: LayoutKind::Horizontal, width: w, height: h, x, y, children },
            consumed_dims + 1 + consumed_bracket,
        ))
    } else if rest.starts_with('[') {
        // Vertical split (children stacked top/bottom)
        let (children, consumed_bracket) = parse_layout_children(&rest[1..], ']')?;
        Some((
            LayoutNode::Split { kind: LayoutKind::Vertical, width: w, height: h, x, y, children },
            consumed_dims + 1 + consumed_bracket,
        ))
    } else {
        // Leaf node; may have ,pane_id suffix
        let mut extra = 0;
        let mut pane_id = None;
        if rest.starts_with(',') {
            let id_str = &rest[1..];
            let end = id_str.find(|c: char| c == ',' || c == '{' || c == '[' || c == '}' || c == ']')
                .unwrap_or(id_str.len());
            pane_id = id_str[..end].parse::<usize>().ok();
            extra = 1 + end;
        }
        Some((
            LayoutNode::Leaf { width: w, height: h, x, y, pane_id },
            consumed_dims + extra,
        ))
    }
}

/// Parse WxH,X,Y returning (width, height, x, y, chars_consumed).
pub(crate) fn parse_dimensions(s: &str) -> Option<(u16, u16, u16, u16, usize)> {
    let x_pos = s.find('x')?;
    let w: u16 = s[..x_pos].parse().ok()?;
    let after_x = &s[x_pos + 1..];
    let comma1 = after_x.find(',')?;
    let h: u16 = after_x[..comma1].parse().ok()?;
    let after_h = &after_x[comma1 + 1..];
    let comma2 = after_h.find(',')?;
    let xc: u16 = after_h[..comma2].parse().ok()?;
    let after_xcoord = &after_h[comma2 + 1..];
    let y_end = after_xcoord.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_xcoord.len());
    let yc: u16 = after_xcoord[..y_end].parse().ok()?;
    let total = x_pos + 1 + comma1 + 1 + comma2 + 1 + y_end;
    Some((w, h, xc, yc, total))
}

/// Parse comma-separated layout children inside brackets.
/// Returns vec of LayoutNode and total chars consumed including closing bracket.
pub(crate) fn parse_layout_children(s: &str, closing: char) -> Option<(Vec<LayoutNode>, usize)> {
    let mut children = Vec::new();
    let mut pos = 0;

    loop {
        if pos >= s.len() { return None; }
        if s.as_bytes()[pos] == closing as u8 {
            pos += 1;
            break;
        }
        if !children.is_empty() {
            if s.as_bytes().get(pos).copied() == Some(b',') {
                pos += 1;
            }
        }
        let child_str = &s[pos..];
        let (node, consumed) = parse_layout_node(child_str)?;
        children.push(node);
        pos += consumed;
    }

    Some((children, pos))
}

#[cfg(test)]
#[path = "../../../tests-rs/test_layout.rs"]
mod test_layout;
