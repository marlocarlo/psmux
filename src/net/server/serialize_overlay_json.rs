#[allow(unused_imports)]

use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::env;
use std::net::TcpListener;

use portable_pty::native_pty_system;
use ratatui::prelude::Rect;

use crate::types::{AppState, CtrlReq, Mode, FocusDir, LayoutKind, PipePaneState, VERSION,
    WaitChannel, WaitForOp, Node, Action, Bind};
use crate::platform::install_console_ctrl_handler;
use crate::pane::{create_window, create_window_raw, split_active_with_command, kill_active_pane, kill_pane_by_id, spawn_warm_pane};
use crate::tree::{self, active_pane, active_pane_mut, resize_all_panes, kill_all_children,
    find_window_index_by_id, focus_pane_by_id, focus_pane_by_id_no_mru, focus_pane_by_index, get_active_pane_id,
    get_split_mut, path_exists};

use super::helpers::{collect_pane_paths_server, serialize_bindings_json, json_escape_string,
    list_windows_json_with_tabs, combined_data_version, TMUX_COMMANDS};
use super::options::{get_option_value, apply_set_option};
use super::window_options::{get_window_option_value, render_window_options};

use crate::input::{send_text_to_active, send_key_to_active, send_paste_to_active, move_focus, find_best_pane_in_direction, find_wrap_target};
use crate::copy_mode::{enter_copy_mode, exit_copy_mode, move_copy_cursor, current_prompt_pos,
    yank_selection, scroll_copy_up, scroll_copy_down, switch_with_copy_save,
    capture_active_pane_text, capture_active_pane_range, capture_active_pane_styled};
use crate::layout::{dump_layout_json, dump_layout_json_fast, apply_layout, cycle_layout,
    cycle_layout_reverse};
use crate::window_ops::{toggle_zoom, remote_mouse_down, remote_mouse_drag, remote_mouse_up,
    remote_mouse_button, remote_mouse_motion, remote_scroll_up, remote_scroll_down,
    swap_pane, break_pane_to_window, unzoom_if_zoomed, resize_pane_vertical,
    resize_pane_horizontal, resize_pane_absolute, rotate_panes, respawn_active_pane,
    handle_pane_mouse, handle_pane_scroll, handle_split_set_sizes, handle_split_resize_done};
use crate::config::{load_config, parse_key_string, format_key_binding, normalize_key_for_binding,
    parse_config_content};
use crate::commands::{parse_command_to_action, format_action, parse_menu_definition, execute_command_string};
use crate::util::{list_windows_json, list_tree_json, list_windows_tmux, base64_encode};
use crate::control;
use crate::format::{expand_format, format_list_windows, format_list_panes, set_buffer_idx_override};
use crate::help;

/// Build a JSON fragment with overlay state (popup, menu, confirm, display_panes).
/// Delegates popup-specific serialization to the popup module.
use super::*;

pub(crate) fn serialize_overlay_json(app: &AppState) -> String {
    use crate::server::helpers::json_escape_string;

    // Popup overlay handles PopupMode, MenuMode, ConfirmMode, PaneChooser, and default
    let mut out = crate::popup::serialize_popup_overlay(app);

    // Include status_message for display-message without -p (#110)
    if let Some((ref msg, since, per_msg_duration)) = app.status_message {
        let elapsed = since.elapsed().as_millis() as u64;
        let display_time = per_msg_duration.unwrap_or(app.display_time_ms);
        if elapsed < display_time {
            out.push_str(",\"status_message\":\"");
            out.push_str(&json_escape_string(msg));
            out.push('"');
        }
    }
    out
}

pub(crate) fn should_spawn_warm_server(app: &AppState) -> bool {
    app.warm_enabled && app.session_name != "__warm__" && !app.destroy_unattached
}

/// Check if the active pane is currently squelched (hiding injected cd+cls).
/// Uses the non-consuming `squelch_cleared()` so the layout serialiser can
/// still properly consume the sentinel via `take_squelch_cleared()`.
pub(crate) fn is_active_pane_squelched(app: &AppState) -> bool {
    if app.windows.is_empty() { return false; }
    let win = &app.windows[app.active_idx];
    if let Some(p) = active_pane(&win.root, &win.active_path) {
        if let Some(deadline) = p.squelch_until {
            let sentinel = p.term.lock()
                .map(|parser| parser.screen().squelch_cleared())
                .unwrap_or(false);
            !sentinel && Instant::now() < deadline
        } else { false }
    } else { false }
}

/// Spawn a standby "warm server" process that pre-loads config + shell.
/// When `psmux new-session` is run later, the CLI claims this warm server
/// via `claim-session` instead of cold-spawning, making session creation
/// nearly instant.  The warm server uses session name `__warm__`.
pub(crate) fn spawn_warm_server(app: &AppState) {
    // destroy-unattached means the user expects the session to be torn down
    // when the last client leaves; keeping a hidden warm server alive breaks
    // that expectation and makes exit-empty appear ineffective.
    if !should_spawn_warm_server(app) {
        return;
    }
    // Skip if a warm server already exists
    let home = env::var("USERPROFILE").or_else(|_| env::var("HOME")).unwrap_or_default();
    let warm_base = if let Some(ref sn) = app.socket_name {
        format!("{}____warm__", sn)
    } else {
        "__warm__".to_string()
    };
    let warm_port_path = format!("{}\\.psmux\\{}.port", home, warm_base);
    if std::path::Path::new(&warm_port_path).exists() {
        // Check if it's actually alive
        if let Ok(port_str) = std::fs::read_to_string(&warm_port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                let addr = format!("127.0.0.1:{}", port);
                if std::net::TcpStream::connect_timeout(
                    &addr.parse().unwrap(),
                    Duration::from_millis(100),
                ).is_ok() {
                    return; // warm server already running
                }
            }
        }
        // Stale port file — remove it (and matching key file)
        let _ = std::fs::remove_file(&warm_port_path);
        let warm_key_path = format!("{}\\.psmux\\{}.key", home, warm_base);
        let _ = std::fs::remove_file(&warm_key_path);
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("psmux"));
    let mut args: Vec<String> = vec!["server".into(), "-s".into(), "__warm__".into()];
    if let Some(ref sn) = app.socket_name {
        args.push("-L".into());
        args.push(sn.clone());
    }
    // Pass current terminal dimensions so the warm server's first window
    // and warm pane are spawned at the right size.
    let area = app.last_window_area;
    if area.width > 1 && area.height > 1 {
        args.push("-x".into());
        args.push(area.width.to_string());
        args.push("-y".into());
        args.push(area.height.to_string());
    }
    #[cfg(windows)]
    { let _ = crate::platform::spawn_server_hidden(&exe, &args); }
    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new(&exe);
        for a in &args { cmd.arg(a); }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let _ = cmd.spawn();
    }
}

/// Parse a popup dimension spec: "80" (absolute) or "95%" (percentage of term_dim).
pub(crate) fn parse_popup_dim(spec: &str, term_dim: u16, default: u16) -> u16 {
    if let Some(pct_str) = spec.strip_suffix('%') {
        if let Ok(pct) = pct_str.parse::<u16>() {
            let pct = pct.min(100);
            (term_dim as u32 * pct as u32 / 100) as u16
        } else {
            default
        }
    } else {
        spec.parse().unwrap_or(default)
    }
}

/// Compute the effective display size from all connected clients' terminal sizes.
/// Returns None if no clients have reported sizes.
pub(crate) fn compute_effective_client_size(app: &AppState) -> Option<(u16, u16)> {
    if app.client_sizes.is_empty() { return None; }
    match app.window_size.as_str() {
        "smallest" => Some((
            app.client_sizes.values().map(|s| s.0).min().unwrap(),
            app.client_sizes.values().map(|s| s.1).min().unwrap(),
        )),
        "largest" => Some((
            app.client_sizes.values().map(|s| s.0).max().unwrap(),
            app.client_sizes.values().map(|s| s.1).max().unwrap(),
        )),
        _ => {
            // "latest" — use latest client's size, fall back to smallest
            if let Some(cid) = app.latest_client_id {
                if let Some(&size) = app.client_sizes.get(&cid) {
                    return Some(size);
                }
            }
            Some((
                app.client_sizes.values().map(|s| s.0).min().unwrap(),
                app.client_sizes.values().map(|s| s.1).min().unwrap(),
            ))
        }
    }
}

/// Process a single CtrlReq during the post-config plugin drain loop.
/// Handles the subset of requests that plugin scripts send (set, show, bind,
/// source-file) and silently drops others.
pub(crate) fn drain_plugin_req(
    app: &mut AppState,
    req: CtrlReq,
    shared_aliases: &std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
) {
    match req {
        CtrlReq::SetOption(option, value) => {
            apply_set_option(app, &option, &value, false);
            app.user_set_options.insert(option.clone());
            if option == "command-alias" {
                if let Ok(mut map) = shared_aliases.write() {
                    *map = app.command_aliases.clone();
                }
            }
        }
        CtrlReq::SetOptionQuiet(option, value, quiet) => {
            apply_set_option(app, &option, &value, quiet);
            app.user_set_options.insert(option.clone());
            if option == "command-alias" {
                if let Ok(mut map) = shared_aliases.write() {
                    *map = app.command_aliases.clone();
                }
            }
        }
        CtrlReq::SetOptionAppend(option, value) => {
            if option.starts_with('@') {
                let existing = app.user_options.get(&option).cloned().unwrap_or_default();
                app.user_options.insert(option, format!("{}{}", existing, value));
            } else {
                match option.as_str() {
                    "status-left" => app.status_left.push_str(&value),
                    "status-right" => app.status_right.push_str(&value),
                    "status-style" => app.status_style.push_str(&value),
                    _ => {}
                }
            }
        }
        CtrlReq::SetOptionUnset(option) => {
            if option.starts_with('@') {
                app.user_options.remove(&option);
            }
        }
        CtrlReq::SetOptionOnlyIfUnset(option, value) => {
            // Only set if the option hasn't been explicitly set by user/config.
            // For @-prefixed user options, check if the key exists.
            // For built-in options, check the user_set_options tracker.
            let already_set = if option.starts_with('@') {
                app.user_options.contains_key(&option)
            } else {
                app.user_set_options.contains(&option)
            };
            if !already_set {
                apply_set_option(app, &option, &value, false);
                app.user_set_options.insert(option.clone());
                if option == "command-alias" {
                    if let Ok(mut map) = shared_aliases.write() {
                        *map = app.command_aliases.clone();
                    }
                }
            }
        }
        CtrlReq::ShowOptionValue(resp, name) => {
            let val = get_option_value(app, &name);
            let _ = resp.send(val);
        }
        CtrlReq::ShowWindowOptionValue(resp, name) => {
            let val = get_window_option_value(app, &name);
            let _ = resp.send(val);
        }
        CtrlReq::ShowOptions(resp) => {
            // Minimal: just send empty to unblock the caller
            let _ = resp.send(String::new());
        }
        CtrlReq::ShowWindowOptions(resp) => {
            let _ = resp.send(render_window_options(app));
        }
        CtrlReq::BindKey(table_name, key, command, repeat) => {
            if let Some(kc) = parse_key_string(&key) {
                let kc = normalize_key_for_binding(kc);
                let sub_cmds = crate::config::split_chained_commands_pub(&command);
                let action = if sub_cmds.len() > 1 {
                    Some(Action::CommandChain(sub_cmds))
                } else {
                    parse_command_to_action(&command)
                };
                if let Some(act) = action {
                    let table = app.key_tables.entry(table_name).or_default();
                    table.retain(|b| b.key != kc);
                    table.push(Bind { key: kc, action: act, repeat });
                }
            }
        }
        CtrlReq::SourceFile(path) => {
            app.defaults_suppressed = false;
            app.key_tables.clear();
            crate::config::populate_default_bindings(app);
            crate::config::source_file(app, &path);
        }
        CtrlReq::UnbindAll => {
            app.key_tables.clear();
            app.defaults_suppressed = true;
        }
        CtrlReq::UnbindAllInTable(table) => {
            if let Some(binds) = app.key_tables.get_mut(&table) {
                binds.clear();
            }
        }
        CtrlReq::UnbindKey(key, table) => {
            if let Some(kc) = parse_key_string(&key) {
                let kc = normalize_key_for_binding(kc);
                let target = table.unwrap_or_else(|| "prefix".to_string());
                if let Some(binds) = app.key_tables.get_mut(&target) {
                    binds.retain(|b| b.key != kc);
                }
            }
        }
        // Ignore other request types during plugin drain
        _ => {}
    }
}
