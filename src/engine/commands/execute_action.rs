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

/// Execute an Action (from key bindings)
pub fn execute_action(app: &mut AppState, action: &Action) -> io::Result<bool> {
    match action {
        Action::DisplayPanes => {
            let win = &app.windows[app.active_idx];
            let mut rects: Vec<(Vec<usize>, ratatui::prelude::Rect)> = Vec::new();
            compute_rects(&win.root, app.last_window_area, &mut rects);
            app.display_map.clear();
            for (i, (path, _)) in rects.into_iter().enumerate() {
                if i >= 10 { break; }
                let digit = (i + app.pane_base_index) % 10;
                app.display_map.push((digit, path));
            }
            app.mode = Mode::PaneChooser { opened_at: Instant::now() };
        }
        Action::MoveFocus(dir) => {
            let d = *dir;
            switch_with_copy_save(app, |app| { crate::input::move_focus(app, d); });
        }
        Action::NewWindow => {
            let pty_system = portable_pty::native_pty_system();
            create_window(&*pty_system, app, None, None)?;
        }
        Action::SplitHorizontal => {
            split_active(app, LayoutKind::Horizontal)?;
        }
        Action::SplitVertical => {
            split_active(app, LayoutKind::Vertical)?;
        }
        Action::KillPane => {
            kill_active_pane(app)?;
        }
        Action::NextWindow => {
            if !app.windows.is_empty() {
                switch_with_copy_save(app, |app| {
                    app.last_window_idx = app.active_idx;
                    app.active_idx = (app.active_idx + 1) % app.windows.len();
                });
            }
        }
        Action::PrevWindow => {
            if !app.windows.is_empty() {
                switch_with_copy_save(app, |app| {
                    app.last_window_idx = app.active_idx;
                    app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len();
                });
            }
        }
        Action::CopyMode => {
            enter_copy_mode(app);
        }
        Action::Paste => {
            paste_latest(app)?;
        }
        Action::Detach => {
            return Ok(true);
        }
        Action::RenameWindow => {
            app.mode = Mode::RenamePrompt { input: String::new() };
        }
        Action::WindowChooser | Action::SessionChooser => {
            let tree = build_choose_tree(app);
            let selected = tree.iter().position(|e| e.is_current_session && e.is_active_window && !e.is_session_header).unwrap_or(0);
            app.mode = Mode::WindowChooser { selected, tree };
        }
        Action::ZoomPane => {
            toggle_zoom(app);
        }
        Action::Command(cmd) => {
            execute_command_string(app, cmd)?;
        }
        Action::CommandChain(cmds) => {
            for cmd in cmds {
                execute_command_string(app, cmd)?;
            }
        }
        Action::SwitchTable(table) => {
            app.current_key_table = Some(table.clone());
        }
    }
    Ok(false)
}

pub fn execute_command_prompt(app: &mut AppState) -> io::Result<()> {
    let cmdline = match &app.mode { Mode::CommandPrompt { input, .. } => input.clone(), _ => String::new() };
    app.mode = Mode::Passthrough;

    // Split on \; or ; to support command chaining (issue #192)
    let sub_commands = crate::config::split_chained_commands_pub(&cmdline);
    if sub_commands.len() > 1 {
        for sub in &sub_commands {
            execute_command_string(app, sub)?;
        }
        return Ok(());
    }

    let parts: Vec<&str> = cmdline.split_whitespace().collect();
    if parts.is_empty() { return Ok(()); }
    match parts[0] {
        // Commands that need local (embedded-mode) handling.
        // In server mode the client sends these via TCP directly, so
        // execute_command_prompt() is only reached in embedded mode.
        "new-window" | "neww" => {
            let pty_system = portable_pty::native_pty_system();
            create_window(&*pty_system, app, None, None)?;
        }
        "split-window" | "splitw" => {
            let kind = if parts.iter().any(|p| *p == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
            split_active(app, kind)?;
        }
        "kill-pane" | "killp" => { kill_active_pane(app)?; }
        "capture-pane" | "capturep" => { capture_active_pane(app)?; }
        "save-buffer" | "saveb" => { if let Some(file) = parts.get(1) { save_latest_buffer(app, file)?; } }
        "list-sessions" | "ls" => { println!("default"); }
        "attach-session" | "attach" | "a" | "at" => { }
        // Everything else delegates to execute_command_string() which
        // handles 80+ commands (list-*, show-*, kill-*, display-*,
        // select-*, rename-*, set-*, bind-*, etc.) and forwards
        // anything it doesn't recognise to the server via TCP.
        _ => {
            execute_command_string(app, &cmdline)?;
        }
    }
    Ok(())
}

/// Execute a command string (used by menus, hooks, confirm dialogs, etc.)
pub fn execute_command_string(app: &mut AppState, cmd: &str) -> io::Result<()> {
    // Split on \; or ; to support command chaining (issue #192)
    let sub_commands = crate::config::split_chained_commands_pub(cmd);
    if sub_commands.len() > 1 {
        for sub in &sub_commands {
            execute_command_string_single(app, sub)?;
        }
        return Ok(());
    }
    execute_command_string_single(app, cmd)
}
