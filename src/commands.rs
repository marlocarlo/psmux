use std::io;
use std::time::Instant;

use crate::types::{AppState, Mode, Action, FocusDir, LayoutKind, MenuItem, Menu, PopupPty, Node};
use crate::tree::{compute_rects, kill_all_children, get_active_pane_id};
use crate::pane::{create_window, split_active, kill_active_pane};
use crate::copy_mode::{enter_copy_mode, switch_with_copy_save, paste_latest,
    capture_active_pane, save_latest_buffer};
use crate::session::{send_control_to_port, list_all_sessions_tree};
use crate::window_ops::toggle_zoom;

/// Show text output in a popup overlay (used by list-* commands inside a session).
fn show_output_popup(app: &mut AppState, title: &str, output: String) {
    let lines: Vec<&str> = output.lines().collect();
    let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
    let height = (lines.len() as u16 + 2).max(5).min(40);
    app.mode = Mode::PopupMode {
        command: title.to_string(),
        output,
        process: None,
        width: width.min(120),
        height,
        close_on_exit: false,
        popup_pty: None,
    };
}

/// Generate list-windows output from AppState (tmux-compatible format).
fn generate_list_windows(app: &AppState) -> String {
    crate::util::list_windows_tmux(app)
}

/// Generate list-panes output from AppState.
fn generate_list_panes(app: &AppState) -> String {
    let win = &app.windows[app.active_idx];
    fn collect(node: &Node, panes: &mut Vec<(usize, u16, u16)>) {
        match node {
            Node::Leaf(p) => { panes.push((p.id, p.last_cols, p.last_rows)); }
            Node::Split { children, .. } => { for c in children { collect(c, panes); } }
        }
    }
    let mut panes = Vec::new();
    collect(&win.root, &mut panes);
    let active_id = get_active_pane_id(&win.root, &win.active_path);
    let mut output = String::new();
    for (pos, (id, cols, rows)) in panes.iter().enumerate() {
        let idx = pos + app.pane_base_index;
        let marker = if active_id == Some(*id) { " (active)" } else { "" };
        output.push_str(&format!("{}: [{}x{}] [history {}/{}, 0 bytes] %{}{}\n",
            idx, cols, rows, app.history_limit, app.history_limit, id, marker));
    }
    output
}

/// Generate list-clients output from AppState.
fn generate_list_clients(app: &AppState) -> String {
    format!("/dev/pts/0: {}: {} [{}x{}] (utf8)\n",
        app.session_name,
        app.windows[app.active_idx].name,
        app.last_window_area.width,
        app.last_window_area.height)
}

/// Generate show-hooks output from AppState.
fn generate_show_hooks(app: &AppState) -> String {
    let mut output = String::new();
    for (name, commands) in &app.hooks {
        for cmd in commands {
            output.push_str(&format!("{} -> {}\n", name, cmd));
        }
    }
    if output.is_empty() {
        output.push_str("(no hooks)\n");
    }
    output
}

/// Generate list-commands output.
fn generate_list_commands() -> String {
    crate::help::cli_command_lines().join("\n")
}

/// Build the choose-tree data for the WindowChooser mode.
pub fn build_choose_tree(app: &AppState) -> Vec<crate::session::TreeEntry> {
    let current_windows: Vec<(String, usize, String, bool)> = app.windows.iter().enumerate().map(|(i, w)| {
        let panes = crate::tree::count_panes(&w.root);
        let size = format!("{}x{}", app.last_window_area.width, app.last_window_area.height);
        (w.name.clone(), panes, size, i == app.active_idx)
    }).collect();
    list_all_sessions_tree(&app.session_name, &current_windows)
}

/// Extract a window index from a tmux-style target string.
/// Handles formats like "0", ":0", ":=0", "=0", stripping leading ':'/'=' chars.
fn parse_window_target(target: &str) -> Option<usize> {
    let s = target.trim_start_matches(':').trim_start_matches('=');
    s.parse::<usize>().ok()
}

/// Parse a command string to an Action
pub fn parse_command_to_action(cmd: &str) -> Option<Action> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() { return None; }
    
    match parts[0] {
        "display-panes" | "displayp" => Some(Action::DisplayPanes),
        "new-window" | "neww" => Some(Action::NewWindow),
        "split-window" | "splitw" => {
            // If extra flags like -c, -d, -p, -F, or a shell command are present,
            // store as Command to preserve the full argument string.
            let has_extra = parts.iter().any(|p| matches!(*p, "-c" | "-d" | "-p" | "-l" | "-F" | "-P" | "-b" | "-f" | "-I" | "-Z" | "-e"))
                || parts.iter().any(|p| !p.starts_with('-') && *p != "split-window" && *p != "splitw");
            if has_extra {
                Some(Action::Command(cmd.to_string()))
            } else if parts.iter().any(|p| *p == "-h") {
                Some(Action::SplitHorizontal)
            } else {
                Some(Action::SplitVertical)
            }
        }
        "kill-pane" | "killp" => Some(Action::KillPane),
        "next-window" | "next" => Some(Action::NextWindow),
        "previous-window" | "prev" => Some(Action::PrevWindow),
        "copy-mode" => Some(Action::CopyMode),
        "paste-buffer" | "pasteb" => Some(Action::Paste),
        "detach-client" | "detach" => Some(Action::Detach),
        "rename-window" | "renamew" => Some(Action::RenameWindow),
        "choose-window" | "choose-tree" | "choose-session" => Some(Action::WindowChooser),
        "resize-pane" | "resizep" if parts.iter().any(|p| *p == "-Z") => Some(Action::ZoomPane),
        "zoom-pane" => Some(Action::ZoomPane),
        "select-pane" | "selectp" => {
            if parts.iter().any(|p| *p == "-U") {
                Some(Action::MoveFocus(FocusDir::Up))
            } else if parts.iter().any(|p| *p == "-D") {
                Some(Action::MoveFocus(FocusDir::Down))
            } else if parts.iter().any(|p| *p == "-L") {
                Some(Action::MoveFocus(FocusDir::Left))
            } else if parts.iter().any(|p| *p == "-R") {
                Some(Action::MoveFocus(FocusDir::Right))
            } else {
                Some(Action::Command(cmd.to_string()))
            }
        }
        "last-window" | "last" => Some(Action::Command("last-window".to_string())),
        "last-pane" | "lastp" => Some(Action::Command("last-pane".to_string())),
        "swap-pane" | "swapp" => Some(Action::Command(cmd.to_string())),
        "resize-pane" | "resizep" => Some(Action::Command(cmd.to_string())),
        "rotate-window" | "rotatew" => Some(Action::Command(cmd.to_string())),
        "break-pane" | "breakp" => Some(Action::Command(cmd.to_string())),
        "respawn-pane" | "respawnp" => Some(Action::Command(cmd.to_string())),
        "kill-window" | "killw" => Some(Action::Command(cmd.to_string())),
        "kill-session" => Some(Action::Command(cmd.to_string())),
        "select-window" | "selectw" => Some(Action::Command(cmd.to_string())),
        "toggle-sync" => Some(Action::Command("toggle-sync".to_string())),
        "send-keys" => Some(Action::Command(cmd.to_string())),
        "set-option" | "set" | "setw" | "set-window-option" => Some(Action::Command(cmd.to_string())),
        "source-file" | "source" => Some(Action::Command(cmd.to_string())),
        "select-layout" | "selectl" => Some(Action::Command(cmd.to_string())),
        "next-layout" => Some(Action::Command("next-layout".to_string())),
        "confirm-before" | "confirm" => Some(Action::Command(cmd.to_string())),
        "display-menu" | "menu" => Some(Action::Command(cmd.to_string())),
        "display-popup" | "popup" => Some(Action::Command(cmd.to_string())),
        "pipe-pane" | "pipep" => Some(Action::Command(cmd.to_string())),
        "rename-session" | "rename" => Some(Action::Command(cmd.to_string())),
        "clear-history" => Some(Action::Command("clear-history".to_string())),
        "set-buffer" | "setb" => Some(Action::Command(cmd.to_string())),
        "delete-buffer" | "deleteb" => Some(Action::Command("delete-buffer".to_string())),
        "display-message" | "display" => Some(Action::Command(cmd.to_string())),
        "switch-client" | "switchc" => {
            // Check for -T flag to switch key table
            if let Some(pos) = parts.iter().position(|p| *p == "-T") {
                if let Some(table) = parts.get(pos + 1) {
                    Some(Action::SwitchTable(table.to_string()))
                } else {
                    Some(Action::Command(cmd.to_string()))
                }
            } else {
                Some(Action::Command(cmd.to_string()))
            }
        }
        _ => Some(Action::Command(cmd.to_string()))
    }
}

/// Format an Action back to a command string
pub fn format_action(action: &Action) -> String {
    match action {
        Action::DisplayPanes => "display-panes".to_string(),
        Action::NewWindow => "new-window".to_string(),
        Action::SplitHorizontal => "split-window -h".to_string(),
        Action::SplitVertical => "split-window -v".to_string(),
        Action::KillPane => "kill-pane".to_string(),
        Action::NextWindow => "next-window".to_string(),
        Action::PrevWindow => "previous-window".to_string(),
        Action::CopyMode => "copy-mode".to_string(),
        Action::Paste => "paste-buffer".to_string(),
        Action::Detach => "detach-client".to_string(),
        Action::RenameWindow => "rename-window".to_string(),
        Action::WindowChooser => "choose-window".to_string(),
        Action::ZoomPane => "resize-pane -Z".to_string(),
        Action::MoveFocus(dir) => {
            let flag = match dir {
                FocusDir::Up => "-U",
                FocusDir::Down => "-D",
                FocusDir::Left => "-L",
                FocusDir::Right => "-R",
            };
            format!("select-pane {}", flag)
        }
        Action::Command(cmd) => cmd.clone(),
        Action::CommandChain(cmds) => cmds.join(" \\; "),
        Action::SwitchTable(table) => format!("switch-client -T {}", table),
    }
}

/// Parse a command line string, respecting quoted arguments
pub fn parse_command_line(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_double_quotes = false;
    let mut in_single_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if in_single_quotes {
            // Inside single quotes: everything is literal (no escape processing)
            if c == '\'' {
                in_single_quotes = false;
            } else {
                current.push(c);
            }
        } else if c == '\\' && in_double_quotes {
            // Inside double quotes, recognise two escape sequences:
            //   \"  → literal double-quote
            //   \\  → literal backslash
            // All other backslashes are kept literal because psmux is a
            // Windows-native tool where backslash is the normal path
            // separator (e.g. "C:\Program Files\Git\bin\bash.exe").
            if i + 1 < chars.len() && chars[i + 1] == '"' {
                current.push('"');
                i += 1; // skip the quote
            } else if i + 1 < chars.len() && chars[i + 1] == '\\' {
                current.push('\\');
                i += 1; // skip the second backslash
            } else {
                current.push(c); // literal backslash
            }
        } else if c == '"' {
            in_double_quotes = !in_double_quotes;
        } else if c == '\'' && !in_double_quotes {
            in_single_quotes = true;
        } else if c.is_whitespace() && !in_double_quotes {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
        } else {
            current.push(c);
        }
        i += 1;
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

/// Parse a menu definition string into a Menu structure
pub fn parse_menu_definition(def: &str, x: Option<i16>, y: Option<i16>) -> Menu {
    let mut menu = Menu {
        title: String::new(),
        items: Vec::new(),
        selected: 0,
        x,
        y,
    };
    
    let parts: Vec<&str> = def.split_whitespace().collect();
    if parts.is_empty() {
        return menu;
    }
    
    let mut i = 0;
    while i < parts.len() {
        if parts[i] == "-T" {
            if let Some(title) = parts.get(i + 1) {
                menu.title = title.trim_matches('"').to_string();
                i += 2;
                continue;
            }
        }
        
        if let Some(name) = parts.get(i) {
            let name = name.trim_matches('"').to_string();
            if name.is_empty() || name == "-" {
                menu.items.push(MenuItem {
                    name: String::new(),
                    key: None,
                    command: String::new(),
                    is_separator: true,
                });
                i += 1;
            } else {
                let key = parts.get(i + 1).map(|k| k.trim_matches('"').chars().next()).flatten();
                let command = parts.get(i + 2).map(|c| c.trim_matches('"').to_string()).unwrap_or_default();
                menu.items.push(MenuItem {
                    name,
                    key,
                    command,
                    is_separator: false,
                });
                i += 3;
            }
        } else {
            break;
        }
    }
    
    if menu.items.is_empty() && !def.is_empty() {
        menu.title = "Menu".to_string();
        menu.items.push(MenuItem {
            name: def.to_string(),
            key: Some('1'),
            command: def.to_string(),
            is_separator: false,
        });
    }
    
    menu
}

/// Fire hooks for a given event
pub fn fire_hooks(app: &mut AppState, event: &str) {
    if let Some(commands) = app.hooks.get(event).cloned() {
        for cmd in commands {
            let _ = execute_command_string(app, &cmd);
        }
    }
}

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
        Action::WindowChooser => {
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
    let parts: Vec<&str> = cmdline.split_whitespace().collect();
    if parts.is_empty() { return Ok(()); }
    match parts[0] {
        "new-window" => {
            let pty_system = portable_pty::native_pty_system();
            create_window(&*pty_system, app, None, None)?;
        }
        "split-window" => {
            let kind = if parts.iter().any(|p| *p == "-h") { LayoutKind::Horizontal } else { LayoutKind::Vertical };
            split_active(app, kind)?;
        }
        "kill-pane" => { kill_active_pane(app)?; }
        "capture-pane" => { capture_active_pane(app)?; }
        "save-buffer" => { if let Some(file) = parts.get(1) { save_latest_buffer(app, file)?; } }
        "list-sessions" => { println!("default"); }
        "attach-session" => { }
        "list-windows" | "lsw" => {
            let output = generate_list_windows(app);
            show_output_popup(app, "list-windows", output);
        }
        "list-panes" | "lsp" => {
            let output = generate_list_panes(app);
            show_output_popup(app, "list-panes", output);
        }
        "list-clients" | "lsc" => {
            let output = generate_list_clients(app);
            show_output_popup(app, "list-clients", output);
        }
        "list-commands" | "lscm" => {
            let output = generate_list_commands();
            show_output_popup(app, "list-commands", output);
        }
        "show-hooks" => {
            let output = generate_show_hooks(app);
            show_output_popup(app, "show-hooks", output);
        }
        "next-window" => {
            app.last_window_idx = app.active_idx;
            app.active_idx = (app.active_idx + 1) % app.windows.len();
        }
        "previous-window" => {
            app.last_window_idx = app.active_idx;
            app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len();
        }
        "select-window" => {
            if let Some(tidx) = parts.iter().position(|p| *p == "-t").and_then(|i| parts.get(i+1)) {
                if let Some(n) = parse_window_target(tidx) {
                    if n >= app.window_base_index {
                        let internal_idx = n - app.window_base_index;
                        if internal_idx < app.windows.len() {
                            app.last_window_idx = app.active_idx;
                            app.active_idx = internal_idx;
                        }
                    }
                }
            }
        }
        _ => {
            // Apply config change locally (client-side state) and also
            // forward to the server so it takes effect for pane spawning.
            crate::config::parse_config_line(app, &cmdline);
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmdline), &app.session_key);
            }
        }
    }
    Ok(())
}

/// Execute a command string (used by menus, hooks, confirm dialogs, etc.)
pub fn execute_command_string(app: &mut AppState, cmd: &str) -> io::Result<()> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() { return Ok(()); }
    
    match parts[0] {
        "new-window" | "neww" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "new-window\n", &app.session_key);
            }
        }
        "split-window" | "splitw" => {
            if let Some(port) = app.control_port {
                // Forward the full command string to preserve -c, -d, -p etc. flags
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "kill-pane" => {
            let _ = kill_active_pane(app);
        }
        "kill-window" | "killw" => {
            if app.windows.len() > 1 {
                let mut win = app.windows.remove(app.active_idx);
                kill_all_children(&mut win.root);
                if app.active_idx >= app.windows.len() {
                    app.active_idx = app.windows.len() - 1;
                }
            }
        }
        "next-window" | "next" => {
            if !app.windows.is_empty() {
                switch_with_copy_save(app, |app| {
                    app.last_window_idx = app.active_idx;
                    app.active_idx = (app.active_idx + 1) % app.windows.len();
                });
            }
        }
        "previous-window" | "prev" => {
            if !app.windows.is_empty() {
                switch_with_copy_save(app, |app| {
                    app.last_window_idx = app.active_idx;
                    app.active_idx = (app.active_idx + app.windows.len() - 1) % app.windows.len();
                });
            }
        }
        "last-window" | "last" => {
            if app.last_window_idx < app.windows.len() {
                switch_with_copy_save(app, |app| {
                    let tmp = app.active_idx;
                    app.active_idx = app.last_window_idx;
                    app.last_window_idx = tmp;
                });
            }
        }
        "select-window" | "selectw" => {
            if let Some(t_pos) = parts.iter().position(|p| *p == "-t") {
                if let Some(t) = parts.get(t_pos + 1) {
                    if let Some(idx) = parse_window_target(t) {
                        if idx >= app.window_base_index {
                            let internal_idx = idx - app.window_base_index;
                            if internal_idx < app.windows.len() {
                                switch_with_copy_save(app, |app| {
                                    app.last_window_idx = app.active_idx;
                                    app.active_idx = internal_idx;
                                });
                            }
                        }
                    }
                }
            }
        }
        "select-pane" | "selectp" => {
            // Save/restore copy mode across pane switches (tmux parity #43)
            let is_last = parts.iter().any(|p| *p == "-l");
            if is_last {
                switch_with_copy_save(app, |app| {
                    let win = &mut app.windows[app.active_idx];
                    if !app.last_pane_path.is_empty() {
                        let tmp = win.active_path.clone();
                        win.active_path = app.last_pane_path.clone();
                        app.last_pane_path = tmp;
                    }
                });
                return Ok(());
            }
            let dir = if parts.iter().any(|p| *p == "-U") { FocusDir::Up }
                else if parts.iter().any(|p| *p == "-D") { FocusDir::Down }
                else if parts.iter().any(|p| *p == "-L") { FocusDir::Left }
                else if parts.iter().any(|p| *p == "-R") { FocusDir::Right }
                else { return Ok(()); };
            // Zoom-aware directional navigation (tmux parity #134):
            // If zoomed, check if there's a direct neighbor OR a wrap target.
            // If yes: cancel zoom and navigate to it.
            // If no (single-pane window): no-op — stay zoomed.
            if app.windows[app.active_idx].zoom_saved.is_some() {
                // Temporarily unzoom to compute real geometry
                let saved = app.windows[app.active_idx].zoom_saved.take();
                if let Some(ref s) = saved {
                    let win = &mut app.windows[app.active_idx];
                    for (p, sz) in s.iter() {
                        if let Some(Node::Split { sizes, .. }) = crate::tree::get_split_mut(&mut win.root, p) { *sizes = sz.clone(); }
                    }
                }
                crate::tree::resize_all_panes(app);
                // Find direct neighbor only (no wrap when zoomed — tmux parity)
                let win = &app.windows[app.active_idx];
                let mut rects: Vec<(Vec<usize>, ratatui::layout::Rect)> = Vec::new();
                crate::tree::compute_rects(&win.root, app.last_window_area, &mut rects);
                let active_idx = rects.iter().position(|(path, _)| *path == win.active_path);
                let has_target = if let Some(ai) = active_idx {
                    let (_, arect) = &rects[ai];
                    crate::input::find_best_pane_in_direction(&rects, ai, arect, dir, &[], &[])
                        .is_some()
                } else { false };
                if has_target {
                    // Cancel zoom (already unzoomed) and navigate
                    switch_with_copy_save(app, |app| {
                        let win = &app.windows[app.active_idx];
                        app.last_pane_path = win.active_path.clone();
                        crate::input::move_focus(app, dir);
                    });
                } else {
                    // No target (single-pane) — re-zoom (restore saved zoom state)
                    if let Some(s) = saved {
                        let win = &mut app.windows[app.active_idx];
                        for (p, sz) in s.iter() {
                            if let Some(Node::Split { sizes, .. }) = crate::tree::get_split_mut(&mut win.root, p) { *sizes = sz.clone(); }
                        }
                        win.zoom_saved = Some(s);
                    }
                    crate::tree::resize_all_panes(app);
                }
            } else {
                switch_with_copy_save(app, |app| {
                    let win = &app.windows[app.active_idx];
                    app.last_pane_path = win.active_path.clone();
                    crate::input::move_focus(app, dir);
                });
            }
        }
        "last-pane" | "lastp" => {
            switch_with_copy_save(app, |app| {
                let win = &mut app.windows[app.active_idx];
                if !app.last_pane_path.is_empty() {
                    let tmp = win.active_path.clone();
                    win.active_path = app.last_pane_path.clone();
                    app.last_pane_path = tmp;
                }
            });
        }
        "rename-window" | "renamew" => {
            if let Some(name) = parts.get(1) {
                let win = &mut app.windows[app.active_idx];
                win.name = name.to_string();
            }
        }
        "list-windows" | "lsw" => {
            let output = generate_list_windows(app);
            show_output_popup(app, "list-windows", output);
        }
        "list-panes" | "lsp" => {
            let output = generate_list_panes(app);
            show_output_popup(app, "list-panes", output);
        }
        "list-clients" | "lsc" => {
            let output = generate_list_clients(app);
            show_output_popup(app, "list-clients", output);
        }
        "list-commands" | "lscm" => {
            let output = generate_list_commands();
            show_output_popup(app, "list-commands", output);
        }
        "show-hooks" => {
            let output = generate_show_hooks(app);
            show_output_popup(app, "show-hooks", output);
        }
        "zoom-pane" | "zoom" | "resizep -Z" => {
            toggle_zoom(app);
        }
        "copy-mode" => {
            enter_copy_mode(app);
        }
        "display-panes" | "displayp" => {
            let win = &app.windows[app.active_idx];
            let mut rects: Vec<(Vec<usize>, ratatui::layout::Rect)> = Vec::new();
            compute_rects(&win.root, app.last_window_area, &mut rects);
            app.display_map.clear();
            for (i, (path, _)) in rects.into_iter().enumerate() {
                if i >= 10 { break; }
                let digit = (i + app.pane_base_index) % 10;
                app.display_map.push((digit, path));
            }
            app.mode = Mode::PaneChooser { opened_at: Instant::now() };
        }
        "confirm-before" | "confirm" => {
            let rest = parts[1..].join(" ");
            app.mode = Mode::ConfirmMode {
                prompt: format!("Run '{}'?", rest),
                command: rest,
                input: String::new(),
            };
        }
        "display-menu" | "menu" => {
            let rest = parts[1..].join(" ");
            let menu = parse_menu_definition(&rest, None, None);
            if !menu.items.is_empty() {
                app.mode = Mode::MenuMode { menu };
            }
        }
        "display-popup" | "popup" => {
            // Parse -w width, -h height, -E close-on-exit flags
            let mut width: u16 = 80;
            let mut height: u16 = 24;
            let close_on_exit = parts.iter().any(|p| *p == "-E");
            if let Some(pos) = parts.iter().position(|p| *p == "-w") {
                if let Some(v) = parts.get(pos + 1) { width = v.parse().unwrap_or(80); }
            }
            if let Some(pos) = parts.iter().position(|p| *p == "-h") {
                if let Some(v) = parts.get(pos + 1) { height = v.parse().unwrap_or(24); }
            }
            // Collect command (non-flag args)
            let cmd_parts: Vec<&str> = parts[1..].iter()
                .filter(|a| !a.starts_with('-'))
                .copied()
                .collect();
            // Skip width/height values
            let rest = cmd_parts.join(" ");
            
            // Try PTY-based popup for interactive commands
            let pty_result = if !rest.is_empty() {
                Some(portable_pty::native_pty_system())
                    .and_then(|pty_sys| {
                        let pty_size = portable_pty::PtySize { rows: height.saturating_sub(2), cols: width.saturating_sub(2), pixel_width: 0, pixel_height: 0 };
                        let pair = pty_sys.openpty(pty_size).ok()?;
                        let mut cmd_builder = portable_pty::CommandBuilder::new(if cfg!(windows) { "pwsh" } else { "sh" });
                        if let Ok(dir) = std::env::current_dir() { cmd_builder.cwd(dir); }
                        if cfg!(windows) { cmd_builder.args(["-NoProfile", "-Command", &rest]); } else { cmd_builder.args(["-c", &rest]); }
                        let child = pair.slave.spawn_command(cmd_builder).ok()?;
                        // Close the slave handle immediately – required for ConPTY.
                        drop(pair.slave);
                        let term = std::sync::Arc::new(std::sync::Mutex::new(vt100::Parser::new(pty_size.rows, pty_size.cols, 0)));
                        let term_reader = term.clone();
                        if let Ok(mut reader) = pair.master.try_clone_reader() {
                            std::thread::spawn(move || {
                                let mut buf = [0u8; 8192];
                                loop {
                                    match std::io::Read::read(&mut reader, &mut buf) {
                                        Ok(n) if n > 0 => { if let Ok(mut p) = term_reader.lock() { p.process(&buf[..n]); } }
                                        _ => break,
                                    }
                                }
                            });
                        }
                        let mut pty_writer = pair.master.take_writer().ok()?;
                        crate::pane::conpty_preemptive_dsr_response(&mut *pty_writer);
                        Some(PopupPty { master: pair.master, writer: pty_writer, child, term })
                    })
            } else { None };
            
            app.mode = Mode::PopupMode {
                command: rest,
                output: String::new(),
                process: None,
                width,
                height,
                close_on_exit,
                popup_pty: pty_result,
            };
        }
        "resize-pane" | "resizep" => {
            if parts.iter().any(|p| *p == "-Z") {
                toggle_zoom(app);
            } else {
                // Forward to server for actual resize
                if let Some(port) = app.control_port {
                    let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
                }
            }
        }
        "swap-pane" | "swapp" => {
            if let Some(port) = app.control_port {
                let dir = if parts.iter().any(|p| *p == "-U") { "-U" } else { "-D" };
                let _ = send_control_to_port(port, &format!("swap-pane {}\n", dir), &app.session_key);
            }
        }
        "rotate-window" | "rotatew" => {
            if let Some(port) = app.control_port {
                let flag = if parts.iter().any(|p| *p == "-D") { "-D" } else { "" };
                let _ = send_control_to_port(port, &format!("rotate-window {}\n", flag), &app.session_key);
            }
        }
        "break-pane" | "breakp" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "break-pane\n", &app.session_key);
            }
        }
        "respawn-pane" | "respawnp" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "respawn-pane\n", &app.session_key);
            }
        }
        "toggle-sync" => {
            app.sync_input = !app.sync_input;
        }
        "set-option" | "set" | "set-window-option" | "setw" => {
            // Forward to server for option handling
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "bind-key" | "bind" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "unbind-key" | "unbind" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "source-file" | "source" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "send-keys" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "detach-client" | "detach" => {
            // handled by caller to set quit flag
        }
        "rename-session" => {
            if let Some(name) = parts.get(1) {
                app.session_name = name.to_string();
            }
        }
        "select-layout" | "selectl" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "next-layout" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "next-layout\n", &app.session_key);
            }
        }
        "pipe-pane" | "pipep" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
        "choose-tree" | "choose-window" => {
            let tree = build_choose_tree(app);
            let selected = tree.iter().position(|e| e.is_current_session && e.is_active_window && !e.is_session_header).unwrap_or(0);
            app.mode = Mode::WindowChooser { selected, tree };
        }
        "command-prompt" => {
            // Support -I initial_text, -p prompt (ignored), -1 (ignored)
            let initial = parts.windows(2).find(|w| w[0] == "-I").map(|w| w[1].to_string()).unwrap_or_default();
            app.mode = Mode::CommandPrompt { input: initial.clone(), cursor: initial.len() };
        }
        "paste-buffer" | "pasteb" => {
            paste_latest(app)?;
        }
        "set-buffer" => {
            if let Some(text) = parts.get(1) {
                app.paste_buffers.insert(0, text.to_string());
                if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
            }
        }
        "delete-buffer" => {
            if !app.paste_buffers.is_empty() { app.paste_buffers.remove(0); }
        }
        "clear-history" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "clear-history\n", &app.session_key);
            }
        }
        "kill-session" => {
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, "kill-session\n", &app.session_key);
            }
        }
        "run-shell" | "run" => {
            // Parse with quote-aware parser to handle nested quotes properly
            let args = parse_command_line(cmd);
            let mut cmd_parts: Vec<&str> = Vec::new();
            for arg in &args[1..] {
                if arg == "-b" { /* always spawn non-blocking */ }
                else { cmd_parts.push(arg); }
            }
            let shell_cmd = cmd_parts.join(" ");
            if !shell_cmd.is_empty() {
                // Expand ~ to home directory
                let shell_cmd = if shell_cmd.contains('~') {
                    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
                    shell_cmd.replace("~/", &format!("{}/", home)).replace("~\\", &format!("{}\\", home))
                } else {
                    shell_cmd
                };
                // Set PSMUX_TARGET_SESSION so child scripts connect to the correct server
                let target_session = app.port_file_base();
                #[cfg(windows)]
                {
                    let mut c = std::process::Command::new("pwsh");
                    c.args(["-NoProfile", "-Command", &shell_cmd]);
                    if !target_session.is_empty() {
                        c.env("PSMUX_TARGET_SESSION", &target_session);
                    }
                    let _ = c.spawn();
                }
                #[cfg(not(windows))]
                {
                    let mut c = std::process::Command::new("sh");
                    c.args(["-c", &shell_cmd]);
                    if !target_session.is_empty() {
                        c.env("PSMUX_TARGET_SESSION", &target_session);
                    }
                    let _ = c.spawn();
                }
            }
        }
        _ => {
            // Apply config locally (handles set, bind, source, etc.)
            let old_shell = app.default_shell.clone();
            crate::config::parse_config_line(app, cmd);
            if app.default_shell != old_shell {
                if let Some(mut wp) = app.warm_pane.take() {
                    wp.child.kill().ok();
                }
            }
            // Also forward unknown commands to server (catch-all for tmux compat)
            if let Some(port) = app.control_port {
                let _ = send_control_to_port(port, &format!("{}\n", cmd), &app.session_key);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_app() -> AppState {
        let mut app = AppState::new("test_session".to_string());
        app.window_base_index = 0;
        app.pane_base_index = 0;
        app
    }

    #[test]
    fn test_generate_list_clients() {
        let mut app = mock_app();
        // Need at least one window for list-clients
        let win = crate::types::Window {
            root: Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] },
            active_path: vec![],
            name: "shell".to_string(),
            id: 0,
            activity_flag: false,
            bell_flag: false,
            silence_flag: false,
            last_output_time: std::time::Instant::now(),
            last_seen_version: 0,
            manual_rename: false,
            layout_index: 0,
            pane_mru: vec![],
            zoom_saved: None,
        };
        app.windows.push(win);
        let output = generate_list_clients(&app);
        assert!(output.contains("test_session"), "should contain session name");
        assert!(output.contains("(utf8)"), "should contain encoding");
        assert!(output.contains("shell"), "should contain window name");
    }

    #[test]
    fn test_generate_show_hooks_empty() {
        let app = mock_app();
        let output = generate_show_hooks(&app);
        assert_eq!(output, "(no hooks)\n");
    }

    #[test]
    fn test_generate_show_hooks_with_hooks() {
        let mut app = mock_app();
        app.hooks.insert("after-new-window".to_string(), vec!["run-shell 'echo hello'".to_string()]);
        let output = generate_show_hooks(&app);
        assert!(output.contains("after-new-window"), "should contain hook name");
        assert!(output.contains("run-shell"), "should contain hook command");
    }

    #[test]
    fn test_generate_list_commands() {
        let output = generate_list_commands();
        assert!(output.contains("list-windows"), "should list list-windows command");
        assert!(output.contains("show-hooks"), "should list show-hooks command");
        assert!(output.contains("list-commands"), "should list list-commands command");
        assert!(output.contains("list-clients"), "should list list-clients command");
    }

    #[test]
    fn test_show_output_popup_sets_mode() {
        let mut app = mock_app();
        show_output_popup(&mut app, "test-cmd", "line1\nline2\nline3".to_string());
        match &app.mode {
            Mode::PopupMode { command, output, .. } => {
                assert_eq!(command, "test-cmd");
                assert!(output.contains("line1"));
                assert!(output.contains("line3"));
            }
            _ => panic!("expected PopupMode"),
        }
    }
}
