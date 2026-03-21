use std::io;
use ratatui::prelude::*;

use crate::types::{AppState, Pane, Node, LayoutKind, DragState};
use crate::platform::process_kill;

/// Split an area into sub-rects with 1px gaps between them for separator lines.
/// Matches tmux-style gapless panes with single-character separators.
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

/// Extract (detach) a node from the tree at the given path WITHOUT killing it.
/// Returns (remaining_tree, extracted_node).
/// If the path points to the root, returns (None, root).
pub fn extract_node(root: Node, path: &[usize]) -> (Option<Node>, Option<Node>) {
    if path.is_empty() {
        return (None, Some(root));
    }
    match root {
        Node::Leaf(p) => (Some(Node::Leaf(p)), None), // path doesn't exist
        Node::Split { kind, sizes, children } => {
            let idx = path[0];
            if idx >= children.len() {
                return (Some(Node::Split { kind, sizes, children }), None);
            }
            if path.len() == 1 {
                // Extract child at idx
                let mut remaining: Vec<Node> = Vec::new();
                let mut extracted: Option<Node> = None;
                for (i, child) in children.into_iter().enumerate() {
                    if i == idx { extracted = Some(child); }
                    else { remaining.push(child); }
                }
                let tree = if remaining.is_empty() {
                    None
                } else if remaining.len() == 1 {
                    Some(remaining.into_iter().next().unwrap())
                } else {
                    let mut eq = vec![100 / remaining.len() as u16; remaining.len()];
                    let rem = 100 - eq.iter().sum::<u16>();
                    if let Some(last) = eq.last_mut() { *last += rem; }
                    Some(Node::Split { kind, sizes: eq, children: remaining })
                };
                (tree, extracted)
            } else {
                // Recurse into the child at idx
                let mut new_children: Vec<Node> = Vec::new();
                let mut extracted: Option<Node> = None;
                for (i, child) in children.into_iter().enumerate() {
                    if i == idx {
                        let (rem, ext) = extract_node(child, &path[1..]);
                        extracted = ext;
                        if let Some(r) = rem { new_children.push(r); }
                    } else {
                        new_children.push(child);
                    }
                }
                let tree = if new_children.is_empty() {
                    None
                } else if new_children.len() == 1 {
                    Some(new_children.into_iter().next().unwrap())
                } else {
                    let mut eq = vec![100 / new_children.len() as u16; new_children.len()];
                    let rem = 100 - eq.iter().sum::<u16>();
                    if let Some(last) = eq.last_mut() { *last += rem; }
                    Some(Node::Split { kind, sizes: eq, children: new_children })
                };
                (tree, extracted)
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
fn collect_child_refs<'a>(node: &'a mut Node, out: &mut Vec<&'a mut Box<dyn portable_pty::Child>>) {
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

pub fn prune_exited(n: Node, remain_on_exit: bool) -> Option<Node> {
    match n {
        Node::Leaf(mut p) => {
            if p.dead { return Some(Node::Leaf(p)); }
            match p.child.try_wait() {
                Ok(Some(_)) => {
                    if remain_on_exit {
                        p.dead = true;
                        Some(Node::Leaf(p))
                    } else {
                        None
                    }
                }
                _ => Some(Node::Leaf(p)),
            }
        }
        Node::Split { kind, sizes, children } => {
            let mut new_children: Vec<Node> = Vec::new();
            let mut new_sizes: Vec<u16> = Vec::new();
            for (i, child) in children.into_iter().enumerate() {
                if let Some(c) = prune_exited(child, remain_on_exit) {
                    new_children.push(c);
                    new_sizes.push(sizes.get(i).copied().unwrap_or(0));
                }
            }
            if new_children.is_empty() { None }
            else if new_children.len() == 1 { Some(new_children.remove(0)) }
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
                Some(Node::Split { kind, sizes: new_sizes, children: new_children })
            }
        }
    }
}

pub fn path_exists(node: &Node, path: &Vec<usize>) -> bool {
    let mut cur = node;
    for &idx in path.iter() {
        match cur {
            Node::Split { children, .. } => {
                if let Some(next) = children.get(idx) { cur = next; } else { return false; }
            }
            Node::Leaf(_) => return false,
        }
    }
    matches!(cur, Node::Leaf(_) | Node::Split { .. })
}

pub fn first_leaf_path(node: &Node) -> Vec<usize> {
    fn rec(n: &Node, path: &mut Vec<usize>) -> Option<Vec<usize>> {
        match n {
            Node::Leaf(_) => Some(path.clone()),
            Node::Split { children, .. } => {
                for (i, child) in children.iter().enumerate() {
                    path.push(i);
                    if let Some(p) = rec(child, path) { return Some(p); }
                    path.pop();
                }
                None
            }
        }
    }
    rec(node, &mut Vec::new()).unwrap_or_default()
}

/// Find the tree path to a pane by its ID.  Returns None if not found.
pub fn find_path_by_id(node: &Node, id: usize) -> Option<Vec<usize>> {
    fn rec(n: &Node, id: usize, path: &mut Vec<usize>) -> Option<Vec<usize>> {
        match n {
            Node::Leaf(p) => if p.id == id { Some(path.clone()) } else { None },
            Node::Split { children, .. } => {
                for (i, c) in children.iter().enumerate() {
                    path.push(i);
                    if let Some(p) = rec(c, id, path) { return Some(p); }
                    path.pop();
                }
                None
            }
        }
    }
    rec(node, id, &mut Vec::new())
}

/// Collect all leaf pane paths in DFS order.
fn collect_leaf_paths(node: &Node, path: &mut Vec<usize>, out: &mut Vec<(usize, Vec<usize>)>) {
    match node {
        Node::Leaf(p) => out.push((p.id, path.clone())),
        Node::Split { children, .. } => {
            for (i, c) in children.iter().enumerate() {
                path.push(i);
                collect_leaf_paths(c, path, out);
                path.pop();
            }
        }
    }
}

/// Move `pane_id` to the front of the MRU list.
/// If not present, inserts at front.
pub fn touch_mru(mru: &mut Vec<usize>, pane_id: usize) {
    if let Some(pos) = mru.iter().position(|&id| id == pane_id) {
        mru.remove(pos);
    }
    mru.insert(0, pane_id);
}

/// Remove a pane ID from the MRU list.
pub fn remove_from_mru(mru: &mut Vec<usize>, pane_id: usize) {
    mru.retain(|&id| id != pane_id);
}

/// Get the MRU rank of a pane ID (0 = most recent). Returns usize::MAX if not found.
pub fn mru_rank(mru: &[usize], pane_id: usize) -> usize {
    mru.iter().position(|&id| id == pane_id).unwrap_or(usize::MAX)
}

/// Collect all pane IDs from a tree node (DFS order).
pub fn collect_pane_ids(node: &Node) -> Vec<usize> {
    let mut ids = Vec::new();
    fn rec(node: &Node, ids: &mut Vec<usize>) {
        match node {
            Node::Leaf(p) => ids.push(p.id),
            Node::Split { children, .. } => {
                for c in children { rec(c, ids); }
            }
        }
    }
    rec(node, &mut ids);
    ids
}

/// Find the next pane path after `active_path` in DFS order (wraps around).
/// Returns the path of the next pane, or None if there's only one pane.
pub fn next_leaf_path(node: &Node, active_path: &[usize]) -> Option<Vec<usize>> {
    let mut leaves = Vec::new();
    collect_leaf_paths(node, &mut Vec::new(), &mut leaves);
    if leaves.len() <= 1 { return None; }
    let pos = leaves.iter().position(|(_, p)| p.as_slice() == active_path).unwrap_or(0);
    let next = if pos + 1 < leaves.len() { pos + 1 } else { pos.saturating_sub(1) };
    Some(leaves[next].1.clone())
}

/// Get the pane ID of the active pane
pub fn get_active_pane_id(node: &Node, path: &[usize]) -> Option<usize> {
    match node {
        Node::Leaf(p) => Some(p.id),
        Node::Split { children, .. } => {
            if let Some(&idx) = path.first() {
                if let Some(child) = children.get(idx) {
                    return get_active_pane_id(child, &path[1..]);
                }
            }
            children.first().and_then(|c| get_active_pane_id(c, &[]))
        }
    }
}

/// Get the pane ID at a specific path (used by format vars for pane position lookup).
pub fn get_active_pane_id_at_path(node: &Node, path: &[usize]) -> Option<usize> {
    get_active_pane_id(node, path)
}

/// Get the positional index (0-based) of a pane within its window, by pane ID.
/// Panes are enumerated in tree traversal order (left-to-right, top-to-bottom).
pub fn get_pane_position_in_window(node: &Node, target_id: usize) -> Option<usize> {
    fn collect_ids(node: &Node, ids: &mut Vec<usize>) {
        match node {
            Node::Leaf(p) => ids.push(p.id),
            Node::Split { children, .. } => {
                for c in children { collect_ids(c, ids); }
            }
        }
    }
    let mut ids = Vec::new();
    collect_ids(node, &mut ids);
    ids.iter().position(|&id| id == target_id)
}

/// Get the Nth leaf pane (0-based positional index) from the tree.
pub fn get_nth_pane(node: &Node, n: usize) -> Option<&Pane> {
    fn collect_panes<'a>(node: &'a Node, panes: &mut Vec<&'a Pane>) {
        match node {
            Node::Leaf(p) => panes.push(p),
            Node::Split { children, .. } => {
                for c in children { collect_panes(c, panes); }
            }
        }
    }
    let mut panes = Vec::new();
    collect_panes(node, &mut panes);
    panes.get(n).copied()
}

pub fn find_window_index_by_id(app: &AppState, wid: usize) -> Option<usize> {
    app.windows.iter().position(|w| w.id == wid)
}

pub fn focus_pane_by_id(app: &mut AppState, pid: usize) {
    fn rec(node: &Node, path: &mut Vec<usize>, found: &mut Option<Vec<usize>>, pid: usize) {
        match node {
            Node::Leaf(p) => { if p.id == pid { *found = Some(path.clone()); } }
            Node::Split { children, .. } => {
                for (i, c) in children.iter().enumerate() { path.push(i); rec(c, path, found, pid); path.pop(); if found.is_some() { return; } }
            }
        }
    }
    for (wi, w) in app.windows.iter().enumerate() {
        let mut path = Vec::new();
        let mut found = None;
        rec(&w.root, &mut path, &mut found, pid);
        if let Some(p) = found { app.active_idx = wi; let win = &mut app.windows[wi]; win.active_path = p; touch_mru(&mut win.pane_mru, pid); return; }
    }
}

pub fn focus_pane_by_index(app: &mut AppState, idx: usize) {
    fn collect_pane_paths(node: &Node, path: &mut Vec<usize>, panes: &mut Vec<Vec<usize>>) {
        match node {
            Node::Leaf(_) => { panes.push(path.clone()); }
            Node::Split { children, .. } => {
                for (i, c) in children.iter().enumerate() {
                    path.push(i);
                    collect_pane_paths(c, path, panes);
                    path.pop();
                }
            }
        }
    }
    let win = &mut app.windows[app.active_idx];
    let mut pane_paths = Vec::new();
    let mut path = Vec::new();
    collect_pane_paths(&win.root, &mut path, &mut pane_paths);
    if let Some(path) = pane_paths.get(idx) {
        win.active_path = path.clone();
    }
}

/// Count the number of leaf (pane) nodes in a tree.
pub fn count_panes(node: &Node) -> usize {
    match node {
        Node::Leaf(_) => 1,
        Node::Split { children, .. } => children.iter().map(count_panes).sum(),
    }
}

/// Immutable reference to the active pane (follows path through splits).
pub fn active_pane<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Pane> {
    match node {
        Node::Leaf(p) => Some(p),
        Node::Split { children, .. } => {
            if path.is_empty() { return None; }
            let idx = path[0].min(children.len().saturating_sub(1));
            active_pane(&children[idx], &path[1..])
        }
    }
}

/// Get the index of the pane at `path` among all leaf panes in the window tree (DFS order).
pub fn pane_index_in_window(node: &Node, path: &[usize]) -> Option<usize> {
    // Find the pane ID at the path, then count its position
    let target = active_pane(node, path)?;
    let target_id = target.id;
    let mut idx = 0usize;
    fn walk(n: &Node, target_id: usize, idx: &mut usize) -> bool {
        match n {
            Node::Leaf(p) => {
                if p.id == target_id { return true; }
                *idx += 1;
                false
            }
            Node::Split { children, .. } => {
                for c in children {
                    if walk(c, target_id, idx) { return true; }
                }
                false
            }
        }
    }
    if walk(node, target_id, &mut idx) { Some(idx) } else { None }
}

/// Reap exited children from the app. Returns (all_empty, any_pruned).
/// Fast check: does any pane in this node tree have an exited child?
/// Uses try_wait() but avoids the full tree rebuild if nothing has exited.
fn has_any_exited(node: &mut Node) -> bool {
    match node {
        Node::Leaf(p) => {
            if p.dead { return false; } // Already dead, handled
            matches!(p.child.try_wait(), Ok(Some(_)))
        }
        Node::Split { children, .. } => {
            children.iter_mut().any(|c| has_any_exited(c))
        }
    }
}

pub fn reap_children(app: &mut AppState) -> io::Result<(bool, bool)> {
    let remain = app.remain_on_exit;
    let mut any_pruned = false;
    for i in (0..app.windows.len()).rev() {
        // Fast path: skip full tree rebuild if no panes have exited
        if !has_any_exited(&mut app.windows[i].root) {
            continue;
        }
        let leaves_before = count_panes(&app.windows[i].root);
        let active_pane_id = get_active_pane_id(&app.windows[i].root, &app.windows[i].active_path);
        let root = std::mem::replace(&mut app.windows[i].root, Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
        match prune_exited(root, remain) {
            Some(new_root) => {
                let leaves_after = count_panes(&new_root);
                if leaves_after < leaves_before {
                    any_pruned = true;
                    // Clean up MRU: remove IDs of panes that no longer exist
                    let surviving_ids = collect_pane_ids(&new_root);
                    app.windows[i].pane_mru.retain(|id| surviving_ids.contains(id));
                }
                app.windows[i].root = new_root;
                // After tree restructuring, the old active_path indices may
                // still be in-range but point to a different pane (issue #140).
                // Always verify by pane ID, not just path validity.
                let current_id = get_active_pane_id(&app.windows[i].root, &app.windows[i].active_path);
                if current_id != active_pane_id || !path_exists(&app.windows[i].root, &app.windows[i].active_path) {
                    // The active pane's path shifted due to tree restructuring.
                    // Try to find it by ID first, then by MRU order (issue #71).
                    let found = active_pane_id.and_then(|id| find_path_by_id(&app.windows[i].root, id))
                        .or_else(|| {
                            app.windows[i].pane_mru.iter()
                                .find_map(|&id| find_path_by_id(&app.windows[i].root, id))
                        });
                    app.windows[i].active_path = found.unwrap_or_else(|| first_leaf_path(&app.windows[i].root));
                }
            }
            None => {
                app.windows.remove(i);
                any_pruned = true;
                // Adjust active_idx after removing a window
                let _old = app.active_idx;
                if !app.windows.is_empty() {
                    if i < app.active_idx {
                        app.active_idx -= 1;
                    } else if app.active_idx >= app.windows.len() {
                        app.active_idx = app.windows.len() - 1;
                    }
                }
                if app.active_idx != _old {
                    crate::debug_log::server_log("switch", &format!(
                        "REAP: active_idx {} -> {} after removing window at index {}", _old, app.active_idx, i));
                }
            }
        }
    }
    Ok((app.windows.is_empty(), any_pruned))
}

/// Collect all leaf (Pane) nodes from the tree, consuming it.
/// Returns them in DFS (left-to-right) order.
pub fn collect_leaves(node: Node) -> Vec<Node> {
    match node {
        Node::Leaf(_) => vec![node],
        Node::Split { children, .. } => {
            let mut leaves = Vec::new();
            for child in children {
                leaves.extend(collect_leaves(child));
            }
            leaves
        }
    }
}
