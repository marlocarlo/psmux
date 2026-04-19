#[allow(unused_imports)]
use std::io;
use std::time::Instant;
#[cfg(windows)]
use std::path::PathBuf;

use std::io::Write;
use crate::types::{AppState, Mode, Action, FocusDir, LayoutKind, MenuItem, Menu, Node};
use crate::tree::{compute_rects, kill_all_children, get_active_pane_id};
use crate::pane::{create_window, split_active, kill_active_pane};
use crate::copy_mode::{enter_copy_mode, switch_with_copy_save, paste_latest,
    capture_active_pane, save_latest_buffer};
use crate::session::{send_control_to_port, list_all_sessions_tree};
use crate::window_ops::toggle_zoom;

/// Parse a popup dimension spec: "80" (absolute) or "95%" (percentage of term_dim).
use super::*;

pub(crate) fn execute_command_string_single(app: &mut AppState, cmd: &str) -> io::Result<()> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() { return Ok(()); }

    if let Some(r) = super::exec_navigation::handle_navigation(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_window_pane::handle_window_pane(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_layout_zoom::handle_layout_zoom(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_copy_mode::handle_copy_mode(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_display::handle_display(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_list::handle_list(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_options::handle_options(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_session::handle_session(app, cmd, &parts) { return r; }
    if let Some(r) = super::exec_misc::handle_misc(app, cmd, &parts) { return r; }

    // Default: apply config locally and forward unknown commands
    let old_shell = app.default_shell.clone();
    crate::config::parse_config_line(app, cmd);
    if app.default_shell != old_shell {
        if let Some(mut wp) = app.warm_pane.take() {
            wp.child.kill().ok();
        }
    }
    if let Some(port) = app.control_port {
        let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
    }
    Ok(())
}
