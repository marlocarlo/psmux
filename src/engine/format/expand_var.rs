#[allow(unused_imports)]
// format.rs — tmux-compatible format expansion engine
//
// Supports: variables, #{?cond,t,f}, #{==:a,b}, #{!=:a,b}, #{<:a,b}, etc,
// #{s/pat/rep/flags:var}, #{b:var}, #{d:var}, #{t:var}, #{l:str},
// #{E:var}, #{T:var}, #{q:var}, #{e|op|flags:a,b}, #{m/flags:pat,str},
// #{=N:var}, #{=/N/marker:var}, #{pN:var}, #{||:a,b}, #{&&:a,b},
// #{C/flags:fmt}, chained modifiers with ';',
// -F custom format for list commands.

use std::env;
use std::cell::Cell;

use crate::types::{AppState, Node, LayoutKind, Pane, Mode, VERSION};
use crate::tree::{split_with_gaps, get_active_pane_id, active_pane, count_panes};
use crate::config::format_key_binding;

// Thread-local override for per-pane format expansion in list-panes.
// When set to Some(pos), pane_* variables resolve for the Nth pane (0-based)
// instead of the active pane.
use super::*;

/// Expand a named variable.
pub fn expand_var(var: &str, app: &AppState, win_idx: usize) -> String {
    let win = match app.windows.get(win_idx) {
        Some(w) => w,
        None => {
            // Even without a window, some variables still resolve
            return match var {
                "session_name" => app.session_name.clone(),
                "session_windows" => app.windows.len().to_string(),
                "session_id" => format!("${}", app.session_id),
                "pid" | "server_pid" => std::process::id().to_string(),
                "version" => VERSION.to_string(),
                "host" | "hostname" => hostname_cached(),
                "host_short" => { let h = hostname_cached(); h.split('.').next().unwrap_or(&h).to_string() }
                _ => {
                    if let Some(v) = lookup_option(var, app) { v } else { String::new() }
                }
            };
        }
    };
    // Resolve the target pane for format expansion. When PANE_POS_OVERRIDE is set
    // (during list-panes iteration), use that positional pane instead of the active pane.
    let (fmt_pane_pos, fmt_pane_is_active) = {
        let override_pos = PANE_POS_OVERRIDE.get();
        if let Some(pos) = override_pos {
            let active_id = get_active_pane_id(&win.root, &win.active_path);
            let is_active = crate::tree::get_nth_pane(&win.root, pos)
                .map(|p| Some(p.id) == active_id).unwrap_or(false);
            (pos, is_active)
        } else {
            let active_id = get_active_pane_id(&win.root, &win.active_path).unwrap_or(0);
            let pos = crate::tree::get_pane_position_in_window(&win.root, active_id).unwrap_or(0);
            (pos, true)
        }
    };
    // Helper closure to get the target pane reference
    let target_pane = || -> Option<&Pane> {
        crate::tree::get_nth_pane(&win.root, fmt_pane_pos)
    };
    match var {
        // ── Session ──
        "session_name" => app.session_name.clone(),
        "session_attached" => if app.attached_clients > 0 { "1".into() } else { "0".into() },
        "session_windows" => app.windows.len().to_string(),
        "session_id" => format!("${}", app.session_id),
        "session_created" => app.created_at.timestamp().to_string(),
        "session_created_string" => app.created_at.format("%a %b %e %H:%M:%S %Y").to_string(),
        "session_activity" | "session_last_attached" => app.created_at.timestamp().to_string(),
        "session_activity_string" => app.created_at.format("%a %b %e %H:%M:%S %Y").to_string(),
        "session_group" | "session_group_list" => app.session_group.clone().unwrap_or_default(),
        "session_alerts" | "session_stack" => String::new(),
        "session_group_attached" => {
            if app.session_group.is_some() && app.attached_clients > 0 { "1".into() } else { "0".into() }
        }
        "session_group_size" => {
            if app.session_group.is_some() { "1".into() } else { "0".into() }
        }
        "session_grouped" => if app.session_group.is_some() { "1".into() } else { "0".into() },
        "session_format" | "session_many_attached" => if app.attached_clients > 1 { "1".into() } else { "0".into() },
        "session_path" => env::var("HOME").or_else(|_| env::var("USERPROFILE")).unwrap_or_default(),

        // ── Window ──
        "window_index" => (win_idx + app.window_base_index).to_string(),
        "window_name" => win.name.clone(),
        "window_active" => if win_idx == app.active_idx { "1".into() } else { "0".into() },
        "window_panes" => count_panes(&win.root).to_string(),
        "window_flags" | "window_raw_flags" => {
            let mut f = String::new();
            if win_idx == app.active_idx { f.push('*'); }
            else if win_idx == app.last_window_idx { f.push('-'); }
            if win.zoom_saved.is_some() { f.push('Z'); }
            if win.activity_flag { f.push('#'); }
            if win.bell_flag { f.push('!'); }
            if win.silence_flag { f.push('~'); }
            f
        }
        "window_id" => format!("@{}", win.id),
        "window_activity_flag" => if win.activity_flag { "1".into() } else { "0".into() },
        "window_zoomed_flag" => if win.zoom_saved.is_some() { "1".into() } else { "0".into() },
        "window_layout" | "window_visible_layout" => generate_window_layout(&win.root, app.last_window_area),
        "window_width" => app.last_window_area.width.to_string(),
        "window_height" => app.last_window_area.height.to_string(),
        "window_format" => "1".into(),
        "window_activity" => app.created_at.timestamp().to_string(),
        "window_silence_flag" => if win.silence_flag { "1".into() } else { "0".into() },
        "window_bell_flag" => if win.bell_flag { "1".into() } else { "0".into() },
        "window_linked" => if win.linked_from.is_some() { "1".into() } else { "0".into() },
        "window_linked_sessions" => if win.linked_from.is_some() { "1".into() } else { "0".into() },
        "window_linked_sessions_list" => String::new(),
        "window_last_flag" => if win_idx == app.last_window_idx { "1".into() } else { "0".into() },
        "window_start_flag" => if win_idx == 0 { "1".into() } else { "0".into() },
        "window_end_flag" => if win_idx == app.windows.len().saturating_sub(1) { "1".into() } else { "0".into() },
        "window_bigger" => "0".into(),
        "window_cell_width" => "8".into(),
        "window_cell_height" => "16".into(),
        "window_offset_x" | "window_offset_y" | "window_stack_index" => "0".into(),

        // ── Pane ──
        "pane_index" => {
            (fmt_pane_pos + app.pane_base_index).to_string()
        }
        "pane_id" => {
            if let Some(p) = target_pane() { format!("%{}", p.id) } else { "%0".into() }
        }
        "pane_title" => {
            if let Some(p) = target_pane() {
                if !p.title.is_empty() { p.title.clone() } else { hostname_cached() }
            } else { hostname_cached() }
        }
        "pane_width" => {
            if let Some(p) = target_pane() { p.last_cols.to_string() } else { "80".into() }
        }
        "pane_height" => {
            if let Some(p) = target_pane() { p.last_rows.to_string() } else { "24".into() }
        }
        "pane_active" => if fmt_pane_is_active { "1".into() } else { "0".into() },
        "pane_current_command" => {
            if let Some(p) = target_pane() {
                if let Some(pid) = p.child_pid {
                    crate::platform::process_info::get_foreground_process_name(pid)
                        .unwrap_or_else(|| "shell".into())
                } else if !p.title.is_empty() {
                    p.title.clone()
                } else {
                    "shell".into()
                }
            } else { String::new() }
        }
        "pane_current_path" => {
            if let Some(p) = target_pane() {
                // Layer 1: PEB walk (authoritative for local processes)
                if let Some(pid) = p.child_pid {
                    if let Some(cwd) = crate::platform::process_info::get_foreground_cwd(pid) {
                        return cwd;
                    }
                }
                // Layer 2: OSC 7 path (works over SSH/WSL where PEB fails)
                if let Ok(parser) = p.term.lock() {
                    if let Some(osc_path) = parser.screen().path() {
                        return osc_path.to_string();
                    }
                }
                // Layer 3: fallback to server CWD
                std::env::current_dir()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else { String::new() }
        }
        "pane_path" => {
            // Pure OSC 7 value (tmux-compatible: only what the shell announced)
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    parser.screen().path().unwrap_or_default().to_string()
                } else { String::new() }
            } else { String::new() }
        }
        "pane_pid" => {
            if let Some(p) = target_pane() {
                p.child_pid.map(|pid| pid.to_string()).unwrap_or_default()
            } else { String::new() }
        }
        "pane_tty" => {
            if let Some(p) = target_pane() { format!("/dev/pty{}", p.id) }
            else { String::new() }
        }
        "pane_in_mode" => match app.mode {
            Mode::CopyMode | Mode::CopySearch { .. } | Mode::ClockMode => "1".into(),
            _ => "0".into(),
        },
        "pane_mode" => match app.mode {
            Mode::CopyMode | Mode::CopySearch { .. } => "copy-mode".into(),
            Mode::ClockMode => "clock-mode".into(),
            _ => String::new(),
        },
        "pane_synchronized" => if app.sync_input { "1".into() } else { "0".into() },
        "pane_dead" => {
            if let Some(p) = target_pane() {
                if p.dead { "1".into() } else { "0".into() }
            } else { "0".into() }
        }
        "pane_dead_signal" | "pane_dead_status" | "pane_dead_time" => "0".into(),
        "pane_format" => "1".into(),
        "pane_input_off"
        | "pane_pipe" | "pane_unseen_changes" => "0".into(),
        "pane_last" => {
            if let Some(p) = target_pane() {
                if !app.last_pane_path.is_empty() {
                    if let Some(last_p) = active_pane(&win.root, &app.last_pane_path) {
                        if last_p.id == p.id { return "1".into(); }
                    }
                }
            }
            "0".into()
        }
        "pane_marked" => {
            if let Some(p) = target_pane() {
                if let Some((mw, mp)) = app.marked_pane {
                    if mw == win_idx && mp == p.id { "1".into() } else { "0".into() }
                } else { "0".into() }
            } else { "0".into() }
        }
        "pane_marked_set" => {
            if app.marked_pane.is_some() { "1".into() } else { "0".into() }
        }
        "pane_left" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { rect.x.to_string() } else { "0".into() }
            } else { "0".into() }
        }
        "pane_top" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { rect.y.to_string() } else { "0".into() }
            } else { "0".into() }
        }
        "pane_right" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { (rect.x + rect.width).saturating_sub(1).to_string() } else { "79".into() }
            } else { "79".into() }
        }
        "pane_bottom" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) { (rect.y + rect.height).saturating_sub(1).to_string() } else { "23".into() }
            } else { "23".into() }
        }
        "pane_at_top" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    if rect.y == app.last_window_area.y { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_at_bottom" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    let bottom = rect.y + rect.height;
                    let win_bottom = app.last_window_area.y + app.last_window_area.height;
                    if bottom >= win_bottom { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_at_left" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    if rect.x == app.last_window_area.x { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_at_right" => {
            if let Some(p) = target_pane() {
                let mut rects = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                if let Some((_, rect)) = rects.iter().find(|(path, _)| {
                    crate::tree::get_active_pane_id_at_path(&win.root, path) == Some(p.id)
                }) {
                    let right = rect.x + rect.width;
                    let win_right = app.last_window_area.x + app.last_window_area.width;
                    if right >= win_right { "1".into() } else { "0".into() }
                } else { "1".into() }
            } else { "1".into() }
        }
        "pane_search_string" => app.copy_search_query.clone(),
        "pane_start_command" => app.default_shell.clone(),
        "pane_start_path" | "pane_tabs" => String::new(),
        "pane_fg" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, c) = parser.screen().cursor_position();
                    if let Some(cell) = parser.screen().cell(r, c) {
                        return format_vt100_color(cell.fgcolor());
                    }
                }
            }
            "default".into()
        }
        "pane_bg" => {
            if let Some(p) = target_pane() {
                if let Ok(parser) = p.term.lock() {
                    let (r, c) = parser.screen().cursor_position();
                    if let Some(cell) = parser.screen().cell(r, c) {
                        return format_vt100_color(cell.bgcolor());
                    }
                }
            }
            "default".into()
        }

        // Remaining variables (cursor, mouse, copy mode, buffer, client, server, options, misc)
        _ => expand_var_extra(var, app, win_idx, fmt_pane_pos, fmt_pane_is_active),
    }
}

