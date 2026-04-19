#[allow(unused_imports)]
use std::io::{self, Write};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use portable_pty::native_pty_system;
use ratatui::prelude::*;

use crate::types::{AppState, Mode, FocusDir, LayoutKind, DragState, Node, Pane};
use crate::tree::{active_pane, active_pane_mut, compute_rects, compute_split_borders,
    split_sizes_at, adjust_split_sizes, path_exists, resize_all_panes};
use crate::pane::{create_window, split_active};
use crate::commands::{execute_action, execute_command_prompt, execute_command_string};
use crate::config::normalize_key_for_binding;
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, switch_with_copy_save, move_copy_cursor,
    scroll_copy_up, scroll_copy_down, scroll_pane_scrollback, paste_latest, yank_selection,
    search_copy_mode, search_next, search_prev, scroll_to_top, scroll_to_bottom};
use crate::layout::{cycle_top_layout, apply_layout};
use crate::window_ops::{toggle_zoom, swap_pane, break_pane_to_window};

/// Write a mouse event to the child PTY using the encoding the child requested.
use super::*;

pub fn move_focus(app: &mut AppState, dir: FocusDir) {
    let win = &mut app.windows[app.active_idx];
    let mut rects: Vec<(Vec<usize>, Rect)> = Vec::new();
    compute_rects(&win.root, app.last_window_area, &mut rects);
    let mut active_idx = None;
    for (i, (path, _)) in rects.iter().enumerate() { if *path == win.active_path { active_idx = Some(i); break; } }
    let Some(ai) = active_idx else { return; };
    let (_, arect) = &rects[ai];
    // Collect pane IDs for MRU-based tie-breaking (issue #70)
    let pane_ids: Vec<usize> = rects.iter().map(|(path, _)| {
        crate::tree::get_active_pane_id(&win.root, path).unwrap_or(usize::MAX)
    }).collect();
    // Try direct neighbour first, then wrap to opposite edge (tmux parity #61)
    let target = find_best_pane_in_direction(&rects, ai, arect, dir, &pane_ids, &win.pane_mru)
        .or_else(|| find_wrap_target(&rects, ai, arect, dir, &pane_ids, &win.pane_mru));
    if let Some(ni) = target {
        // Update MRU: push the newly focused pane to front
        if let Some(new_pane_id) = pane_ids.get(ni) {
            crate::tree::touch_mru(&mut win.pane_mru, *new_pane_id);
        }
        win.active_path = rects[ni].0.clone();
    }
}

/// Spatial pane navigation: find the best pane in the given direction.
/// Prefers panes that overlap on the perpendicular axis (visually adjacent),
/// then picks the closest by primary-axis gap, tie-broken by MRU recency
/// when multiple candidates have the same geometry (tmux parity #70).
pub fn find_best_pane_in_direction(
    rects: &[(Vec<usize>, Rect)],
    ai: usize,
    arect: &Rect,
    dir: FocusDir,
    pane_ids: &[usize],
    pane_mru: &[usize],
) -> Option<usize> {
    // Center of the active pane (scaled by 2 to avoid fractional math)
    let acx = arect.x as i32 * 2 + arect.width as i32;
    let acy = arect.y as i32 * 2 + arect.height as i32;

    // Check whether two 1-D ranges [a_start, a_start+a_len) and [b_start, b_start+b_len) overlap
    let ranges_overlap = |a_start: u16, a_len: u16, b_start: u16, b_len: u16| -> bool {
        let a_end = a_start + a_len;
        let b_end = b_start + b_len;
        a_start < b_end && b_start < a_end
    };

    // (index, primary_gap, perp_center_dist, has_perp_overlap, mru_rank)
    let mut best: Option<(usize, u32, i32, bool, usize)> = None;

    for (i, (_, r)) in rects.iter().enumerate() {
        if i == ai { continue; }
        // Primary-axis gap: the pane must be in the correct direction
        let (primary_gap, perp_overlap) = match dir {
            FocusDir::Left => {
                if r.x + r.width > arect.x { continue; }
                let gap = (arect.x - (r.x + r.width)) as u32;
                let overlap = ranges_overlap(r.y, r.height, arect.y, arect.height);
                (gap, overlap)
            }
            FocusDir::Right => {
                if r.x < arect.x + arect.width { continue; }
                let gap = (r.x - (arect.x + arect.width)) as u32;
                let overlap = ranges_overlap(r.y, r.height, arect.y, arect.height);
                (gap, overlap)
            }
            FocusDir::Up => {
                if r.y + r.height > arect.y { continue; }
                let gap = (arect.y - (r.y + r.height)) as u32;
                let overlap = ranges_overlap(r.x, r.width, arect.x, arect.width);
                (gap, overlap)
            }
            FocusDir::Down => {
                if r.y < arect.y + arect.height { continue; }
                let gap = (r.y - (arect.y + arect.height)) as u32;
                let overlap = ranges_overlap(r.x, r.width, arect.x, arect.width);
                (gap, overlap)
            }
        };

        // Perpendicular center distance (how far off-center the candidate is)
        let rcx = r.x as i32 * 2 + r.width as i32;
        let rcy = r.y as i32 * 2 + r.height as i32;
        let perp_dist = match dir {
            FocusDir::Left | FocusDir::Right => (rcy - acy).abs(),
            FocusDir::Up | FocusDir::Down => (rcx - acx).abs(),
        };

        // MRU rank: lower = more recently used (tmux parity #70)
        let rank = pane_ids.get(i)
            .map(|id| crate::tree::mru_rank(pane_mru, *id))
            .unwrap_or(usize::MAX);

        let dominated = if let Some((_, bg, bd, bo, br)) = best {
            // Prefer: (1) perp-overlapping over non-overlapping,
            //         (2) smaller primary gap,
            //         (3) among overlapping candidates with same gap → MRU (tmux parity #70),
            //         (4) among non-overlapping candidates → perpendicular center distance,
            //         (5) final fallback → MRU rank
            if perp_overlap && !bo {
                false  // new candidate has overlap, current best doesn't → new wins
            } else if !perp_overlap && bo {
                true   // current best has overlap, new doesn't → new loses
            } else if primary_gap < bg {
                false  // closer on primary axis
            } else if primary_gap > bg {
                true   // farther on primary axis
            } else if perp_overlap && bo {
                // Both candidates overlap the active pane's perpendicular
                // range with the same primary gap — use MRU directly.
                // tmux does NOT compare center distance for overlapping
                // candidates; it picks the most recently focused one.
                rank >= br
            } else if perp_dist < bd {
                false  // neither overlaps → closer perpendicular center
            } else if perp_dist > bd {
                true   // farther perpendicular center
            } else {
                rank >= br  // same geometry → MRU tie-break
            }
        } else {
            false  // no best yet
        };

        if !dominated {
            best = Some((i, primary_gap, perp_dist, perp_overlap, rank));
        }
    }

    best.map(|(idx, _, _, _, _)| idx)
}

/// Wrap-around pane navigation (tmux parity #61): when no pane exists in the
/// requested direction, wrap to the pane on the opposite edge.
/// For Right → leftmost pane, Left → rightmost, Down → topmost, Up → bottommost.
/// Prefers panes with perpendicular overlap, then closest to center.
pub fn find_wrap_target(
    rects: &[(Vec<usize>, Rect)],
    ai: usize,
    arect: &Rect,
    dir: FocusDir,
    pane_ids: &[usize],
    pane_mru: &[usize],
) -> Option<usize> {
    let acx = arect.x as i32 * 2 + arect.width as i32;
    let acy = arect.y as i32 * 2 + arect.height as i32;

    let ranges_overlap = |a_start: u16, a_len: u16, b_start: u16, b_len: u16| -> bool {
        let a_end = a_start + a_len;
        let b_end = b_start + b_len;
        a_start < b_end && b_start < a_end
    };

    // (index, edge_score, perp_center_dist, has_perp_overlap, mru_rank)
    // edge_score: lower = better (closer to the target edge after wrapping)
    let mut best: Option<(usize, i32, i32, bool, usize)> = None;

    for (i, (_, r)) in rects.iter().enumerate() {
        if i == ai { continue; }

        let (edge_score, perp_overlap) = match dir {
            // Going right, wrap to leftmost → prefer smallest x
            FocusDir::Right => {
                (r.x as i32, ranges_overlap(r.y, r.height, arect.y, arect.height))
            }
            // Going left, wrap to rightmost → prefer largest x+width (negate)
            FocusDir::Left => {
                (-((r.x + r.width) as i32), ranges_overlap(r.y, r.height, arect.y, arect.height))
            }
            // Going down, wrap to topmost → prefer smallest y
            FocusDir::Down => {
                (r.y as i32, ranges_overlap(r.x, r.width, arect.x, arect.width))
            }
            // Going up, wrap to bottommost → prefer largest y+height (negate)
            FocusDir::Up => {
                (-((r.y + r.height) as i32), ranges_overlap(r.x, r.width, arect.x, arect.width))
            }
        };

        let rcx = r.x as i32 * 2 + r.width as i32;
        let rcy = r.y as i32 * 2 + r.height as i32;
        let perp_dist = match dir {
            FocusDir::Left | FocusDir::Right => (rcy - acy).abs(),
            FocusDir::Up | FocusDir::Down => (rcx - acx).abs(),
        };

        let rank = pane_ids.get(i)
            .map(|id| crate::tree::mru_rank(pane_mru, *id))
            .unwrap_or(usize::MAX);

        let dominated = if let Some((_, be, bd, bo, br)) = best {
            if perp_overlap && !bo {
                false
            } else if !perp_overlap && bo {
                true
            } else if edge_score < be {
                false
            } else if edge_score > be {
                true
            } else if perp_overlap && bo {
                // Both overlap with same edge score → MRU (tmux parity #70)
                rank >= br
            } else if perp_dist < bd {
                false
            } else if perp_dist > bd {
                true
            } else {
                rank >= br  // same geometry → MRU tie-break
            }
        } else {
            false
        };

        if !dominated {
            best = Some((i, edge_score, perp_dist, perp_overlap, rank));
        }
    }

    // Tmux parity (#141): wrapped navigation must stay within the same
    // column (U/D) or row (L/R). If no candidate overlaps on the
    // perpendicular axis, the pane is alone in its row/column and
    // navigation should stay put (no-op) rather than jump sideways.
    best.filter(|(_, _, _, has_overlap, _)| *has_overlap)
        .map(|(idx, _, _, _, _)| idx)
}

/// Encode a crossterm `KeyEvent` into the byte sequence that should be
/// written to the child PTY.  Extracted as a standalone function so it can
/// be unit-tested without needing a full `AppState`.
///
/// Returns `None` for key codes we don't handle (F-keys, etc.).
/// Compute xterm modifier parameter: 1 + Shift*1 + Alt*2 + Ctrl*4.
/// Returns 1 when no modifiers are held (callers use >1 to decide whether to
/// emit the extended `;mod` form).
pub(crate) fn modifier_param(mods: KeyModifiers) -> u8 {
    let mut m: u8 = 1;
    if mods.contains(KeyModifiers::SHIFT) { m += 1; }
    if mods.contains(KeyModifiers::ALT) { m += 2; }
    if mods.contains(KeyModifiers::CONTROL) { m += 4; }
    m
}

/// Parse modifier+special key names like "C-Left", "S-Right", "C-S-Up",
/// "C-M-Home", etc. and return the xterm escape sequence.
/// Returns None if the string isn't a recognized modified special key.
pub fn parse_modified_special_key(s: &str) -> Option<String> {
    let upper = s.to_uppercase();
    // Extract modifier prefixes and base key name
    let mut rest = upper.as_str();
    let mut bits: u8 = 0;
    loop {
        if rest.starts_with("C-") { bits |= 4; rest = &rest[2..]; }
        else if rest.starts_with("M-") { bits |= 2; rest = &rest[2..]; }
        else if rest.starts_with("S-") { bits |= 1; rest = &rest[2..]; }
        else { break; }
    }
    if bits == 0 { return None; } // no modifiers found
    let m = bits + 1; // xterm modifier param = 1 + modifier bits
    // Match the base key name
    match rest {
        "ENTER" | "RETURN" | "CR" => Some(format!("\x1b[13;{}~", m)),
        "TAB" => Some(format!("\x1b[9;{}~", m)),
        "BTAB" | "BACKTAB" => {
            // Shift is implicit in BackTab; ensure Shift bit is set in the bitmask
            let sm = (bits | 1) + 1;
            Some(format!("\x1b[9;{}~", sm))
        }
        "LEFT" => Some(format!("\x1b[1;{}D", m)),
        "RIGHT" => Some(format!("\x1b[1;{}C", m)),
        "UP" => Some(format!("\x1b[1;{}A", m)),
        "DOWN" => Some(format!("\x1b[1;{}B", m)),
        "HOME" => Some(format!("\x1b[1;{}H", m)),
        "END" => Some(format!("\x1b[1;{}F", m)),
        "INSERT" | "IC" => Some(format!("\x1b[2;{}~", m)),
        "DELETE" | "DC" => Some(format!("\x1b[3;{}~", m)),
        "PAGEUP" | "PPAGE" => Some(format!("\x1b[5;{}~", m)),
        "PAGEDOWN" | "NPAGE" => Some(format!("\x1b[6;{}~", m)),
        s if s.starts_with('F') && s.len() >= 2 => {
            if let Ok(n) = s[1..].parse::<u8>() {
                let seq = encode_fkey(n, m);
                if seq.is_empty() { None } else { Some(String::from_utf8_lossy(&seq).into_owned()) }
            } else { None }
        }
        _ => None,
    }
}

/// Encode an F-key with optional xterm modifier parameter.
pub(crate) fn encode_fkey(n: u8, m: u8) -> Vec<u8> {
    // F1-F4 use SS3 when unmodified, CSI with modifier when modified.
    let (prefix, num) = match n {
        1 => if m > 1 { ("", Some((11, 'P'))) } else { return b"\x1bOP".to_vec() },
        2 => if m > 1 { ("", Some((12, 'Q'))) } else { return b"\x1bOQ".to_vec() },
        3 => if m > 1 { ("", Some((13, 'R'))) } else { return b"\x1bOR".to_vec() },
        4 => if m > 1 { ("", Some((14, 'S'))) } else { return b"\x1bOS".to_vec() },
        5 => ("", Some((15, '~'))),
        6 => ("", Some((17, '~'))),
        7 => ("", Some((18, '~'))),
        8 => ("", Some((19, '~'))),
        9 => ("", Some((20, '~'))),
        10 => ("", Some((21, '~'))),
        11 => ("", Some((23, '~'))),
        12 => ("", Some((24, '~'))),
        _ => return Vec::new(),
    };
    let _ = prefix;
    if let Some((code, suffix)) = num {
        if suffix == '~' {
            if m > 1 { format!("\x1b[{};{}~", code, m).into_bytes() }
            else { format!("\x1b[{}~", code).into_bytes() }
        } else {
            // F1-F4 modified: \x1b[1;{mod}P/Q/R/S
            format!("\x1b[1;{}{}", m, suffix).into_bytes()
        }
    } else {
        Vec::new()
    }
}
