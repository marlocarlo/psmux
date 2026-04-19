#[allow(unused_imports)]
use std::io;
use ratatui::prelude::*;

use crate::types::{AppState, Pane, Node, LayoutKind, DragState};
use crate::platform::process_kill;

/// Split an area into sub-rects with 1px gaps between them for separator lines.
/// Matches tmux-style gapless panes with single-character separators.
use super::*;

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
pub(crate) fn collect_leaf_paths(node: &Node, path: &mut Vec<usize>, out: &mut Vec<(usize, Vec<usize>)>) {
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

/// Public wrapper for collect_leaf_paths (used by join-pane to resolve pane index to path).
pub fn collect_leaf_paths_pub(node: &Node, path: &mut Vec<usize>, out: &mut Vec<(usize, Vec<usize>)>) {
    collect_leaf_paths(node, path, out);
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

/// Visit every pane in a tree node (DFS order), calling `f` on each.
pub fn for_each_pane(node: &Node, f: &mut dyn FnMut(&Pane)) {
    match node {
        Node::Leaf(p) => f(p),
        Node::Split { children, .. } => {
            for c in children { for_each_pane(c, f); }
        }
    }
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
    focus_pane_by_id_inner(app, pid, true);
}

/// Like `focus_pane_by_id` but does NOT update MRU.
/// Used for temporary -t targeting where the focus change is transient
/// and should not pollute the recency list (#71).
pub fn focus_pane_by_id_no_mru(app: &mut AppState, pid: usize) {
    focus_pane_by_id_inner(app, pid, false);
}

pub(crate) fn focus_pane_by_id_inner(app: &mut AppState, pid: usize, update_mru: bool) {
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
        if let Some(p) = found { app.active_idx = wi; let win = &mut app.windows[wi]; win.active_path = p; if update_mru { touch_mru(&mut win.pane_mru, pid); } return; }
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

/// Reap exited children from the app.
/// Returns `(all_empty, any_pruned, any_newly_dead)`:
/// - `any_pruned`: at least one pane was removed from the tree (remain-on-exit off)
/// - `any_newly_dead`: at least one pane transitioned alive→dead (remain-on-exit on)
///
/// Callers should fire pane-died/pane-exited hooks when either flag is true,
/// and only resize the layout when `any_pruned` is true.
///
/// Fast check: does any pane in this node tree have an exited child?
/// Uses try_wait() but avoids the full tree rebuild if nothing has exited.
pub(crate) fn has_any_exited(node: &mut Node) -> bool {
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

pub fn reap_children(app: &mut AppState) -> io::Result<(bool, bool, bool)> {
    let remain = app.remain_on_exit;
    let mut any_pruned = false;
    let mut any_newly_dead = false;
    for i in (0..app.windows.len()).rev() {
        // Fast path: skip full tree rebuild if no panes have exited
        if !has_any_exited(&mut app.windows[i].root) {
            continue;
        }
        let leaves_before = count_panes(&app.windows[i].root);
        let active_pane_id = get_active_pane_id(&app.windows[i].root, &app.windows[i].active_path);
        let root = std::mem::replace(&mut app.windows[i].root, Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
        let (pruned_result, newly_dead_count) = prune_exited(root, remain);
        if newly_dead_count > 0 {
            any_newly_dead = true;
        }
        match pruned_result {
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
    Ok((app.windows.is_empty(), any_pruned, any_newly_dead))
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

#[cfg(test)]
#[path = "../../../tests-rs/test_issue171_layout_bugs.rs"]
mod test_issue171_layout_bugs;
