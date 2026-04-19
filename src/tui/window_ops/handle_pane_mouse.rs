#[allow(unused_imports)]
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{PtySize, native_pty_system};
use ratatui::prelude::*;

use crate::types::{AppState, Mode, Pane, Node, LayoutKind, DragState, Window, FocusDir};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, get_split_mut, resize_all_panes};
use crate::pane::{detect_shell, build_default_shell, set_tmux_env};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, yank_selection};
use crate::platform::mouse_inject;

/// Mouse debug logger — writes to ~/.psmux/mouse_debug.log when
/// PSMUX_MOUSE_DEBUG=1 is set.
use super::*;

/// Handle a semantic mouse event from the client.
/// The client has already determined the target pane and computed pane-relative
/// coordinates, so no coordinate translation is needed.
pub fn handle_pane_mouse(app: &mut AppState, pane_id: usize, button: u8, col: i16, row: i16, press: bool) {
    // Find the pane by ID and focus it
    let win = &mut app.windows[app.active_idx];
    let mut found_path: Option<Vec<usize>> = None;
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    for (path, _area) in &rects {
        if let Some(pid) = crate::tree::get_active_pane_id(&win.root, path) {
            if pid == pane_id {
                found_path = Some(path.clone());
                break;
            }
        }
    }

    let Some(path) = found_path else { return; };

    // Focus the target pane only on actual clicks (not drag/hover).
    // tmux behavior: click-to-focus, not focus-follows-mouse.
    let is_click = matches!(button, 0 | 1 | 2) && press;
    if is_click && win.active_path != path {
        win.active_path = path.clone();
        if let Some(pid) = crate::tree::get_active_pane_id(&win.root, &path) {
            crate::tree::touch_mru(&mut win.pane_mru, pid);
        }
    }

    // Handle copy mode: position cursor with pane-relative coordinates
    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        let r = row.max(0) as u16;
        let c = col.max(0) as u16;
        if button == 0 && press {
            // Left press: position cursor, clear selection
            app.copy_anchor = None;
            app.copy_pos = Some((r, c));
            app.copy_mouse_down_cell = Some((r, c));
        } else if button == 32 {
            // Left drag: extend selection, but ignore same-cell micro-jitter (#199)
            if app.copy_anchor.is_none() {
                if app.copy_pos == Some((r, c)) {
                    return; // same cell as click, ignore jitter
                }
                app.copy_anchor = Some(app.copy_pos.unwrap_or((r, c)));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
            app.copy_pos = Some((r, c));
        } else if button == 0 && !press {
            // Left release: finalize position
            app.copy_pos = Some((r, c));
            // If close to the original click, treat as click (no selection) (#199)
            if let Some((dr, dc)) = app.copy_mouse_down_cell.take() {
                if (dr as i32 - r as i32).unsigned_abs() <= 1
                    && (dc as i32 - c as i32).unsigned_abs() <= 1
                {
                    app.copy_anchor = None;
                    app.copy_pos = Some((dr, dc));
                    return;
                }
            }
            // Auto-yank if real selection exists (anchor != pos)
            if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                if a != p { let _ = yank_selection(app); }
            }
        }
        return;
    }

    // Forward mouse event to PTY if pane wants it
    let win = &mut app.windows[app.active_idx];
    let win_name = win.name.clone();
    if let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) {
        if pane_wants_mouse(pane) {
            let button_state = match (button, press) {
                (0, true) => mouse_inject::FROM_LEFT_1ST_BUTTON_PRESSED,
                (1, true) => mouse_inject::FROM_LEFT_2ND_BUTTON_PRESSED,
                (2, true) => mouse_inject::RIGHTMOST_BUTTON_PRESSED,
                _ => 0,
            };
            let event_flags = if button == 32 || button == 35 { mouse_inject::MOUSE_MOVED } else { 0 };
            inject_mouse_combined(pane, col, row, button, press, button_state, event_flags, &win_name);
        }
    }
}

/// Handle a semantic scroll event targeted at a specific pane.
pub fn handle_pane_scroll(app: &mut AppState, pane_id: usize, up: bool) {
    // Ignore scroll in popup mode (#110)
    if matches!(app.mode, Mode::PopupMode { .. }) { return; }

    // Handle scroll while already in copy mode (coordinates irrelevant)
    if matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. }) {
        if up {
            scroll_copy_up(app, 3);
        } else {
            scroll_copy_down(app, 3);
            if app.copy_scroll_offset == 0 && app.copy_anchor.is_none() {
                exit_copy_mode(app);
            }
        }
        return;
    }

    // Focus the target pane
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    for (path, _area) in &rects {
        if let Some(pid) = crate::tree::get_active_pane_id(&win.root, path) {
            if pid == pane_id {
                win.active_path = path.clone();
                break;
            }
        }
    }

    // Check if target pane is in alternate screen (TUI app)
    let alt = active_pane(&win.root, &win.active_path)
        .map_or(false, |p| {
            p.term.lock().ok().map_or(false, |t| t.screen().alternate_screen())
        });

    if alt {
        // Forward scroll to TUI app
        let win = &mut app.windows[app.active_idx];
        let win_name = win.name.clone();
        let sgr_btn: u8 = if up { 64 } else { 65 };
        let wheel_delta: i16 = if up { 120 } else { -120 };
        let button_state = ((wheel_delta as i32) << 16) as u32;
        if let Some(pane) = active_pane_mut(&mut win.root, &win.active_path) {
            inject_mouse_combined(pane, 0, 0, sgr_btn, true,
                button_state, mouse_inject::MOUSE_WHEELED, &win_name);
        }
    } else if up && app.scroll_enter_copy_mode {
        // Shell prompt — enter copy mode and scroll
        enter_copy_mode(app);
        scroll_copy_up(app, 3);
    } else if !app.scroll_enter_copy_mode {
        // scroll-enter-copy-mode off: scroll scrollback directly (#193)
        scroll_pane_scrollback(app, 3, up);
    }
}

/// Set split sizes at a given tree path during border drag.
pub fn handle_split_set_sizes(app: &mut AppState, path: &[usize], sizes: &[u16]) {
    let win = &mut app.windows[app.active_idx];
    let mut cur: &mut Node = &mut win.root;
    for &idx in path.iter() {
        match cur {
            Node::Split { children, .. } => {
                if idx < children.len() {
                    cur = &mut children[idx];
                } else {
                    return;
                }
            }
            Node::Leaf(_) => return,
        }
    }
    if let Node::Split { sizes: node_sizes, children, .. } = cur {
        if sizes.len() == children.len() && sizes.len() == node_sizes.len() {
            *node_sizes = sizes.to_vec();
        }
    }
}

/// Finalize a border resize: apply PTY resizes to match the new layout.
pub fn handle_split_resize_done(app: &mut AppState) {
    resize_all_panes(app);
}

pub fn swap_pane(app: &mut AppState, dir: FocusDir) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    
    let mut active_idx = None;
    for (i, (path, _)) in rects.iter().enumerate() { 
        if *path == win.active_path { active_idx = Some(i); break; } 
    }
    let Some(ai) = active_idx else { return; };
    let (_, arect) = &rects[ai];
    
    // Collect pane IDs for MRU-based tie-breaking (issue #70)
    let pane_ids: Vec<usize> = rects.iter().map(|(path, _)| {
        crate::tree::get_active_pane_id(&win.root, path).unwrap_or(usize::MAX)
    }).collect();
    // Try direct neighbour first, then wrap to opposite edge (tmux parity #61)
    let target = crate::input::find_best_pane_in_direction(&rects, ai, arect, dir, &pane_ids, &win.pane_mru)
        .or_else(|| crate::input::find_wrap_target(&rects, ai, arect, dir, &pane_ids, &win.pane_mru));
    if let Some(ni) = target {
        if let Some(new_pane_id) = pane_ids.get(ni) {
            crate::tree::touch_mru(&mut win.pane_mru, *new_pane_id);
        }
        win.active_path = rects[ni].0.clone();
    }
}

pub fn resize_pane_vertical(app: &mut AppState, amount: i16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == LayoutKind::Vertical {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    if idx + 1 < sizes.len() {
                        let new_size = (sizes[idx] as i16 + amount).max(1) as u16;
                        let diff = new_size as i16 - sizes[idx] as i16;
                        sizes[idx] = new_size;
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        // tmux parity (#81): last child has no bottom border.
                        // Resize the previous sibling with the same amount so
                        // the border moves in the arrow direction.
                        let new_size = (sizes[idx - 1] as i16 + amount).max(1) as u16;
                        let diff = new_size as i16 - sizes[idx - 1] as i16;
                        sizes[idx - 1] = new_size;
                        sizes[idx] = (sizes[idx] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

pub fn resize_pane_horizontal(app: &mut AppState, amount: i16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == LayoutKind::Horizontal {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    if idx + 1 < sizes.len() {
                        let new_size = (sizes[idx] as i16 + amount).max(1) as u16;
                        let diff = new_size as i16 - sizes[idx] as i16;
                        sizes[idx] = new_size;
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        // tmux parity (#81): last child has no right border.
                        // Resize the previous sibling with the same amount so
                        // the border moves in the arrow direction.
                        let new_size = (sizes[idx - 1] as i16 + amount).max(1) as u16;
                        let diff = new_size as i16 - sizes[idx - 1] as i16;
                        sizes[idx - 1] = new_size;
                        sizes[idx] = (sizes[idx] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

/// Absolute resize: set the active pane's share to an exact size.
/// axis is "x" (width/horizontal) or "y" (height/vertical).
pub fn resize_pane_absolute(app: &mut AppState, axis: &str, target: u16) {
    let win = &mut app.windows[app.active_idx];
    if win.active_path.is_empty() { return; }
    let target_kind = if axis == "x" { LayoutKind::Horizontal } else { LayoutKind::Vertical };
    for depth in (0..win.active_path.len()).rev() {
        let parent_path = win.active_path[..depth].to_vec();
        if let Some(Node::Split { kind, sizes, .. }) = get_split_mut(&mut win.root, &parent_path) {
            if *kind == target_kind {
                let idx = win.active_path[depth];
                if idx < sizes.len() {
                    let old = sizes[idx];
                    let new = target.max(1);
                    let diff = new as i16 - old as i16;
                    sizes[idx] = new;
                    // Absorb the difference from a neighbour
                    if idx + 1 < sizes.len() {
                        sizes[idx + 1] = (sizes[idx + 1] as i16 - diff).max(1) as u16;
                    } else if idx > 0 {
                        sizes[idx - 1] = (sizes[idx - 1] as i16 - diff).max(1) as u16;
                    }
                }
                return;
            }
        }
    }
}

pub fn rotate_panes(app: &mut AppState, reverse: bool) {
    let win = &mut app.windows[app.active_idx];
    match &mut win.root {
        Node::Split { children, .. } if children.len() >= 2 => {
            if reverse {
                // Rotate counter-clockwise: first element goes to end
                let first = children.remove(0);
                children.push(first);
            } else {
                // Rotate clockwise: last element goes to front
                let last = children.pop().unwrap();
                children.insert(0, last);
            }
        }
        _ => {}
    }
}

pub fn break_pane_to_window(app: &mut AppState) {
    let src_idx = app.active_idx;
    let src_path = app.windows[src_idx].active_path.clone();
    
    // Extract the active pane from the current window using tree operations
    let src_root = std::mem::replace(&mut app.windows[src_idx].root,
        Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
    let (remaining, extracted) = crate::tree::extract_node(src_root, &src_path);
    
    if let Some(pane_node) = extracted {
        let src_empty = remaining.is_none();
        if let Some(rem) = remaining {
            app.windows[src_idx].root = rem;
            app.windows[src_idx].active_path = crate::tree::first_leaf_path(&app.windows[src_idx].root);
        }
        
        // Determine the window name from the pane
        let win_name = match &pane_node {
            Node::Leaf(p) => p.title.clone(),
            _ => format!("win {}", app.windows.len() + 1),
        };
        
        // Create new window containing the extracted pane
        let initial_mru = crate::tree::collect_pane_ids(&pane_node);
        app.windows.push(Window {
            root: pane_node,
            active_path: vec![],
            name: win_name,
            id: app.next_win_id,
            activity_flag: false,
            bell_flag: false,
            silence_flag: false,
            last_output_time: std::time::Instant::now(),
            last_seen_version: 0,
            manual_rename: false,
            layout_index: 0,
            pane_mru: initial_mru,
            zoom_saved: None,
            linked_from: None,
        });
        app.next_win_id += 1;
        
        if src_empty {
            app.windows.remove(src_idx);
        }
        
        // Switch to the new window
        app.active_idx = app.windows.len() - 1;
    } else {
        // Extraction failed — restore
        if let Some(rem) = remaining {
            app.windows[src_idx].root = rem;
        }
    }
}
