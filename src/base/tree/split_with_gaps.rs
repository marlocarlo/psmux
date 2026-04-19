#[allow(unused_imports)]
use std::io;
use ratatui::prelude::*;

use crate::types::{AppState, Pane, Node, LayoutKind, DragState};
use crate::platform::process_kill;

/// Split an area into sub-rects with 1px gaps between them for separator lines.
/// Matches tmux-style gapless panes with single-character separators.
use super::*;

pub fn split_with_gaps(is_horizontal: bool, sizes: &[u16], area: Rect) -> Vec<Rect> {
    let n = sizes.len();
    if n == 0 { return vec![]; }
    if n == 1 { return vec![area]; }

    let gaps = (n - 1) as u16;
    let total_available = if is_horizontal {
        area.width.saturating_sub(gaps)
    } else {
        area.height.saturating_sub(gaps)
    };

    let total_pct: u32 = sizes.iter().map(|&s| s as u32).sum();
    if total_pct == 0 { return vec![area; n]; }

    let mut rects = Vec::with_capacity(n);
    let mut offset: u16 = 0;

    for (i, &pct) in sizes.iter().enumerate() {
        let size = if i == n - 1 {
            total_available.saturating_sub(offset) // last child gets remainder
        } else {
            ((total_available as u32 * pct as u32) / total_pct) as u16
        };

        let child_rect = if is_horizontal {
            Rect::new(area.x + offset + i as u16, area.y, size, area.height)
        } else {
            Rect::new(area.x, area.y + offset + i as u16, area.width, size)
        };

        rects.push(child_rect);
        offset += size;
    }

    rects
}

pub fn active_pane_mut<'a>(node: &'a mut Node, path: &Vec<usize>) -> Option<&'a mut Pane> {
    let mut cur = node;
    for &idx in path.iter() {
        match cur {
            Node::Split { children, .. } => { cur = children.get_mut(idx)?; }
            Node::Leaf(_) => return None,
        }
    }
    match cur { Node::Leaf(p) => Some(p), _ => None }
}

pub fn replace_leaf_with_split(node: &mut Node, path: &Vec<usize>, kind: LayoutKind, new_leaf: Node) {
    if path.is_empty() {
        let old = std::mem::replace(node, Node::Split { kind, sizes: vec![50,50], children: vec![] });
        if let Node::Split { children, .. } = node { children.push(old); children.push(new_leaf); }
        return;
    }
    let mut cur = node;
    for (depth, &idx) in path.iter().enumerate() {
        match cur {
            Node::Split { children, .. } => {
                if depth == path.len()-1 {
                    let leaf = std::mem::replace(&mut children[idx], Node::Split { kind, sizes: vec![50,50], children: vec![] });
                    if let Node::Split { children: c, .. } = &mut children[idx] { c.push(leaf); c.push(new_leaf); }
                    return;
                } else { cur = &mut children[idx]; }
            }
            Node::Leaf(_) => {
                // Path is invalid (points through a Leaf). Kill the new pane
                // to prevent leaking its ConPTY handle and reader thread.
                kill_node(new_leaf);
                return;
            },
        }
    }
}

pub fn kill_leaf(node: &mut Node, path: &Vec<usize>) {
    *node = remove_node(std::mem::replace(node, Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] }), path);
}

/// Kill a node and all its child processes before dropping it.
/// Uses platform-specific process tree killing to ensure all descendant
/// processes (shells, sub-processes, servers, etc.) are terminated.
pub fn kill_node(mut n: Node) {
    match &mut n {
        Node::Leaf(p) => { process_kill::kill_process_tree(&mut p.child); }
        Node::Split { children, .. } => {
            for child in children.iter_mut() {
                kill_all_children(child);
            }
        }
    }
}

pub fn remove_node(n: Node, path: &Vec<usize>) -> Node {
    match n {
        Node::Leaf(p) => {
            Node::Leaf(p)
        }
        Node::Split { kind, sizes, children } => {
            if path.is_empty() { return Node::Split { kind, sizes, children }; }
            let idx = path[0];
            let mut new_children: Vec<Node> = Vec::new();
            for (i, child) in children.into_iter().enumerate() {
                if i == idx {
                    if path.len() > 1 { new_children.push(remove_node(child, &path[1..].to_vec())); }
                    else {
                        kill_node(child);
                    }
                } else { new_children.push(child); }
            }
            if new_children.len() == 1 { new_children.into_iter().next().unwrap() }
            else {
                let mut eq = vec![100 / new_children.len() as u16; new_children.len()];
                let rem = 100 - eq.iter().sum::<u16>();
                if let Some(last) = eq.last_mut() { *last += rem; }
                Node::Split { kind, sizes: eq, children: new_children }
            }
        }
    }
}

pub fn compute_rects(node: &Node, area: Rect, out: &mut Vec<(Vec<usize>, Rect)>) {
    fn rec(node: &Node, area: Rect, path: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, Rect)>) {
        match node {
            Node::Leaf(_) => { out.push((path.clone(), area)); }
            Node::Split { kind, sizes, children } => {
                let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                    sizes.clone()
                } else { vec![(100 / children.len().max(1)) as u16; children.len()] };
                let is_horizontal = matches!(*kind, LayoutKind::Horizontal);
                let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
                for (i, child) in children.iter().enumerate() {
                    if i < rects.len() { path.push(i); rec(child, rects[i], path, out); path.pop(); }
                }
            }
        }
    }
    let mut path = Vec::new();
    rec(node, area, &mut path, out);
}

/// Resize all panes in the current window to match their computed areas
pub fn resize_all_panes(app: &mut AppState) {
    if app.windows.is_empty() { return; }
    let area = app.last_window_area;
    if area.width == 0 || area.height == 0 { return; }
    
    fn resize_node(node: &mut Node, rects: &[(Vec<usize>, Rect)], path: &mut Vec<usize>) {
        match node {
            Node::Leaf(pane) => {
                if let Some((_, rect)) = rects.iter().find(|(p, _)| p == path) {
                    // Skip resize for panes hidden by zoom (size 0 in either
                    // dimension).  Resizing a hidden pane to 1x1 corrupts its
                    // terminal buffer — lines get reflowed to 1-column width
                    // and the cursor position is lost.  (fixes #44, #45)
                    if rect.width == 0 || rect.height == 0 {
                        return;
                    }
                    // Clamp to MIN_PANE_DIM so ConPTY never receives a
                    // dimension small enough to crash the child process.
                    let inner_height = rect.height.max(crate::pane::MIN_PANE_DIM);
                    let inner_width = rect.width.max(crate::pane::MIN_PANE_DIM);
                    
                    if pane.last_rows != inner_height || pane.last_cols != inner_width {
                        let _ = pane.master.resize(portable_pty::PtySize { 
                            rows: inner_height, 
                            cols: inner_width, 
                            pixel_width: 0, 
                            pixel_height: 0 
                        });
                        if let Ok(mut parser) = pane.term.lock() {
                            parser.screen_mut().set_size(inner_height, inner_width);
                        }
                        pane.last_rows = inner_height;
                        pane.last_cols = inner_width;
                    }
                }
            }
            Node::Split { children, .. } => {
                for (i, child) in children.iter_mut().enumerate() {
                    path.push(i);
                    resize_node(child, rects, path);
                    path.pop();
                }
            }
        }
    }
    
    // Only resize the active window immediately — background windows will be
    // resized lazily when switched to.  This avoids O(total_panes) ConPTY
    // resize syscalls on every structural change.
    if app.active_idx < app.windows.len() {
        let win = &mut app.windows[app.active_idx];
        let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
        compute_rects(&win.root, area, &mut rects);
        let mut path = Vec::new();
        resize_node(&mut win.root, &rects, &mut path);
    }
}

pub fn kill_all_children(node: &mut Node) {
    match node {
        Node::Leaf(p) => { process_kill::kill_process_tree(&mut p.child); }
        Node::Split { children, .. } => { for child in children.iter_mut() { kill_all_children(child); } }
    }
}

/// Collect mutable references to all child processes in a tree node.
pub(crate) fn collect_child_refs<'a>(node: &'a mut Node, out: &mut Vec<&'a mut Box<dyn portable_pty::Child>>) {
    match node {
        Node::Leaf(p) => { out.push(&mut p.child); }
        Node::Split { children, .. } => { for child in children.iter_mut() { collect_child_refs(child, out); } }
    }
}

/// Kill all children across multiple windows using a single process snapshot.
/// Much faster than per-window `kill_all_children` when killing an entire session.
pub fn kill_all_children_batch(windows: &mut [crate::types::Window]) {
    let mut all_children: Vec<&mut Box<dyn portable_pty::Child>> = Vec::new();
    for win in windows.iter_mut() {
        collect_child_refs(&mut win.root, &mut all_children);
    }
    if !all_children.is_empty() {
        process_kill::kill_process_trees_batch(&mut all_children);
    }
}

/// Returns borders as (path, kind, idx, pixel_pos, total_pixels_along_axis).
pub fn compute_split_borders(node: &Node, area: Rect, out: &mut Vec<(Vec<usize>, LayoutKind, usize, u16, u16)>) {
    fn rec(node: &Node, area: Rect, path: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, LayoutKind, usize, u16, u16)>) {
        match node {
            Node::Leaf(_) => {}
            Node::Split { kind, sizes, children } => {
                let effective_sizes: Vec<u16> = if sizes.len() == children.len() {
                    sizes.clone()
                } else { vec![(100 / children.len().max(1)) as u16; children.len()] };
                let is_horizontal = matches!(*kind, LayoutKind::Horizontal);
                let rects = split_with_gaps(is_horizontal, &effective_sizes, area);
                let total_px = if is_horizontal { area.width } else { area.height };
                for i in 0..children.len().saturating_sub(1) {
                    if i < rects.len() {
                        let pos = if is_horizontal {
                            rects[i].x + rects[i].width
                        } else {
                            rects[i].y + rects[i].height
                        };
                        out.push((path.clone(), *kind, i, pos, total_px));
                    }
                }
                for (i, child) in children.iter().enumerate() {
                    if i < rects.len() { path.push(i); rec(child, rects[i], path, out); path.pop(); }
                }
            }
        }
    }
    let mut path = Vec::new();
    rec(node, area, &mut path, out);
}

pub fn split_sizes_at<'a>(node: &'a Node, path: Vec<usize>, idx: usize) -> Option<(u16,u16)> {
    let mut cur = node;
    for &i in path.iter() {
        match cur { Node::Split { children, .. } => { cur = children.get(i)?; } _ => return None }
    }
    if let Node::Split { sizes, .. } = cur {
        if idx+1 < sizes.len() { Some((sizes[idx], sizes[idx+1])) } else { None }
    } else { None }
}

pub fn adjust_split_sizes(root: &mut Node, d: &DragState, x: u16, y: u16) {
    if let Some(Node::Split { sizes, .. }) = get_split_mut(root, &d.split_path) {
        let total_pct = sizes[d.index] + sizes[d.index+1];
        let min_pct = 5u16;
        // Convert pixel delta to percentage delta
        let pixel_delta: i32 = match d.kind {
            LayoutKind::Horizontal => x as i32 - d.start_x as i32,
            LayoutKind::Vertical => y as i32 - d.start_y as i32,
        };
        let total_px = d.total_pixels.max(1) as i32;
        let pct_delta = (pixel_delta * total_pct as i32) / total_px;
        let left = (d.left_initial as i32 + pct_delta).clamp(min_pct as i32, (total_pct - min_pct) as i32) as u16;
        let right = total_pct - left;
        sizes[d.index] = left;
        sizes[d.index+1] = right;
    }
}

pub fn get_split_mut<'a>(node: &'a mut Node, path: &Vec<usize>) -> Option<&'a mut Node> {
    let mut cur = node;
    for &idx in path.iter() {
        match cur { Node::Split { children, .. } => { cur = children.get_mut(idx)?; } _ => return None }
    }
    Some(cur)
}

/// Prune exited panes from the tree.  Returns `(Option<Node>, newly_dead_count)`:
/// - `newly_dead_count` tracks panes that transitioned alive→dead in this call
///   (remain-on-exit case), so callers can fire hooks even when the tree shape
///   doesn't change.
pub fn prune_exited(n: Node, remain_on_exit: bool) -> (Option<Node>, usize) {
    match n {
        Node::Leaf(mut p) => {
            if p.dead { return (Some(Node::Leaf(p)), 0); }
            match p.child.try_wait() {
                Ok(Some(_)) => {
                    if remain_on_exit {
                        p.dead = true;
                        (Some(Node::Leaf(p)), 1)
                    } else {
                        (None, 0)
                    }
                }
                _ => (Some(Node::Leaf(p)), 0),
            }
        }
        Node::Split { kind, sizes, children } => {
            let mut new_children: Vec<Node> = Vec::new();
            let mut new_sizes: Vec<u16> = Vec::new();
            let mut newly_dead = 0;
            for (i, child) in children.into_iter().enumerate() {
                let (pruned, dead_count) = prune_exited(child, remain_on_exit);
                newly_dead += dead_count;
                if let Some(c) = pruned {
                    new_children.push(c);
                    new_sizes.push(sizes.get(i).copied().unwrap_or(0));
                }
            }
            if new_children.is_empty() { (None, newly_dead) }
            else if new_children.len() == 1 { (Some(new_children.remove(0)), newly_dead) }
            else {
                // Redistribute removed pane's percentage proportionally among survivors
                let total: u16 = new_sizes.iter().sum();
                if total == 0 || total == 100 {
                    // Already fine or all zero — just normalize
                    if total == 0 {
                        new_sizes = vec![100 / new_children.len() as u16; new_children.len()];
                        let rem = 100 - new_sizes.iter().sum::<u16>();
                        if let Some(last) = new_sizes.last_mut() { *last += rem; }
                    }
                } else {
                    // Scale proportionally to sum to 100
                    let mut scaled: Vec<u16> = new_sizes.iter().map(|&s| (s as u32 * 100 / total as u32) as u16).collect();
                    let rem = 100u16.saturating_sub(scaled.iter().sum::<u16>());
                    if let Some(last) = scaled.last_mut() { *last += rem; }
                    new_sizes = scaled;
                }
                (Some(Node::Split { kind, sizes: new_sizes, children: new_children }), newly_dead)
            }
        }
    }
}
