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

/// Local join-pane: extract source pane and graft into target window.
pub(crate) fn join_pane_local(app: &mut AppState, src_win: Option<usize>, src_pane: Option<usize>,
                   target_win: Option<usize>, target_pane: Option<usize>, horizontal: bool) {
    let src_idx = src_win.unwrap_or(app.active_idx);
    let raw_target_win = target_win.unwrap_or(app.active_idx);
    if src_idx < app.windows.len() && raw_target_win < app.windows.len() && src_idx != raw_target_win {
        // Resolve source pane path
        let src_path = if let Some(pidx) = src_pane {
            let mut leaves = Vec::new();
            crate::tree::collect_leaf_paths_pub(&app.windows[src_idx].root, &mut Vec::new(), &mut leaves);
            if let Some((_, p)) = leaves.get(pidx) {
                p.clone()
            } else {
                app.windows[src_idx].active_path.clone()
            }
        } else {
            app.windows[src_idx].active_path.clone()
        };
        let src_root = std::mem::replace(&mut app.windows[src_idx].root,
            Node::Split { kind: LayoutKind::Horizontal, sizes: vec![], children: vec![] });
        let (remaining, extracted) = crate::tree::extract_node(src_root, &src_path);
        if let Some(pane_node) = extracted {
            let src_empty = remaining.is_none();
            if let Some(rem) = remaining {
                app.windows[src_idx].root = rem;
                app.windows[src_idx].active_path = crate::tree::first_leaf_path(&app.windows[src_idx].root);
            }
            let tgt = if src_empty && raw_target_win > src_idx { raw_target_win - 1 } else { raw_target_win };
            if src_empty {
                app.windows.remove(src_idx);
                if app.active_idx >= app.windows.len() {
                    app.active_idx = app.windows.len().saturating_sub(1);
                }
            }
            if tgt < app.windows.len() {
                // Resolve target pane path
                let tgt_path = if let Some(tpidx) = target_pane {
                    let mut leaves = Vec::new();
                    crate::tree::collect_leaf_paths_pub(&app.windows[tgt].root, &mut Vec::new(), &mut leaves);
                    if let Some((_, p)) = leaves.get(tpidx) {
                        p.clone()
                    } else {
                        app.windows[tgt].active_path.clone()
                    }
                } else {
                    app.windows[tgt].active_path.clone()
                };
                let split_kind = if horizontal { LayoutKind::Horizontal } else { LayoutKind::Vertical };
                crate::tree::replace_leaf_with_split(&mut app.windows[tgt].root, &tgt_path, split_kind, pane_node);
                app.active_idx = tgt;
            }
        } else {
            if let Some(rem) = remaining {
                app.windows[src_idx].root = rem;
            }
        }
    }
}

/// Generate list-commands output.
pub(crate) fn generate_list_commands() -> String {
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
pub(crate) fn parse_window_target(target: &str) -> Option<usize> {
    let s = target.trim_start_matches(':').trim_start_matches('=');
    s.parse::<usize>().ok()
}

/// Parse a command string to an Action
pub fn parse_command_to_action(cmd: &str) -> Option<Action> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() { return None; }
    
    match parts[0] {
        "display-panes" | "displayp" => Some(Action::DisplayPanes),
        "new-window" | "neww" => {
            // If extra flags like -c, -d, -n, -F, -e or a shell command are present,
            // store as Command to preserve the full argument string (esp. -c for start dir).
            let has_extra = parts.len() > 1;
            if has_extra {
                Some(Action::Command(cmd.to_string()))
            } else {
                Some(Action::NewWindow)
            }
        }
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
        "choose-window" | "choose-tree" => Some(Action::WindowChooser),
        "choose-session" => Some(Action::SessionChooser),
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
        "respawn-window" | "respawnw" => Some(Action::Command(cmd.to_string())),
        "kill-window" | "killw" => Some(Action::Command(cmd.to_string())),
        "kill-session" | "kill-ses" => Some(Action::Command(cmd.to_string())),
        "kill-server" => Some(Action::Command(cmd.to_string())),
        "select-window" | "selectw" => Some(Action::Command(cmd.to_string())),
        "toggle-sync" => Some(Action::Command("toggle-sync".to_string())),
        "send-keys" | "send" => Some(Action::Command(cmd.to_string())),
        "send-prefix" => Some(Action::Command(cmd.to_string())),
        "set-option" | "set" | "setw" | "set-window-option" => Some(Action::Command(cmd.to_string())),
        "show-options" | "show" | "show-window-options" | "showw" => Some(Action::Command(cmd.to_string())),
        "source-file" | "source" => Some(Action::Command(cmd.to_string())),
        "select-layout" | "selectl" => Some(Action::Command(cmd.to_string())),
        "next-layout" | "nextl" => Some(Action::Command("next-layout".to_string())),
        "previous-layout" | "prevl" => Some(Action::Command("previous-layout".to_string())),
        "confirm-before" | "confirm" => Some(Action::Command(cmd.to_string())),
        "display-menu" | "menu" => Some(Action::Command(cmd.to_string())),
        "display-popup" | "popup" => Some(Action::Command(cmd.to_string())),
        "display-message" | "display" => Some(Action::Command(cmd.to_string())),
        "pipe-pane" | "pipep" => Some(Action::Command(cmd.to_string())),
        "rename-session" | "rename" => Some(Action::Command(cmd.to_string())),
        "clear-history" | "clearhist" => Some(Action::Command("clear-history".to_string())),
        "set-buffer" | "setb" => Some(Action::Command(cmd.to_string())),
        "delete-buffer" | "deleteb" => Some(Action::Command("delete-buffer".to_string())),
        "list-buffers" | "lsb" => Some(Action::Command(cmd.to_string())),
        "show-buffer" | "showb" => Some(Action::Command(cmd.to_string())),
        "choose-buffer" | "chooseb" => Some(Action::Command(cmd.to_string())),
        "load-buffer" | "loadb" => Some(Action::Command(cmd.to_string())),
        "save-buffer" | "saveb" => Some(Action::Command(cmd.to_string())),
        "capture-pane" | "capturep" => Some(Action::Command(cmd.to_string())),
        "list-windows" | "lsw" => Some(Action::Command(cmd.to_string())),
        "list-panes" | "lsp" => Some(Action::Command(cmd.to_string())),
        "list-clients" | "lsc" => Some(Action::Command(cmd.to_string())),
        "list-commands" | "lscm" => Some(Action::Command(cmd.to_string())),
        "list-keys" | "lsk" => Some(Action::Command(cmd.to_string())),
        "list-sessions" | "ls" => Some(Action::Command(cmd.to_string())),
        "show-hooks" => Some(Action::Command(cmd.to_string())),
        "show-messages" | "showmsgs" => Some(Action::Command(cmd.to_string())),
        "clock-mode" => Some(Action::Command(cmd.to_string())),
        "command-prompt" => Some(Action::Command(cmd.to_string())),
        "has-session" | "has" => Some(Action::Command(cmd.to_string())),
        "move-window" | "movew" => Some(Action::Command(cmd.to_string())),
        "swap-window" | "swapw" => Some(Action::Command(cmd.to_string())),
        "link-window" | "linkw" => Some(Action::Command(cmd.to_string())),
        "unlink-window" | "unlinkw" => Some(Action::Command(cmd.to_string())),
        "find-window" | "findw" => Some(Action::Command(cmd.to_string())),
        "move-pane" | "movep" => Some(Action::Command(cmd.to_string())),
        "join-pane" | "joinp" => Some(Action::Command(cmd.to_string())),
        "resize-window" | "resizew" => Some(Action::Command(cmd.to_string())),
        "run-shell" | "run" => Some(Action::Command(cmd.to_string())),
        "if-shell" | "if" => Some(Action::Command(cmd.to_string())),
        "wait-for" | "wait" => Some(Action::Command(cmd.to_string())),
        "set-environment" | "setenv" => Some(Action::Command(cmd.to_string())),
        "show-environment" | "showenv" => Some(Action::Command(cmd.to_string())),
        "set-hook" => Some(Action::Command(cmd.to_string())),
        "bind-key" | "bind" => Some(Action::Command(cmd.to_string())),
        "unbind-key" | "unbind" => Some(Action::Command(cmd.to_string())),
        "attach-session" | "attach" | "a" | "at" => Some(Action::Command(cmd.to_string())),
        "new-session" | "new" => Some(Action::Command(cmd.to_string())),
        "server-info" | "info" => Some(Action::Command(cmd.to_string())),
        "start-server" | "start" => Some(Action::Command(cmd.to_string())),
        "lock-client" | "lockc" => Some(Action::Command(cmd.to_string())),
        "lock-server" | "lock" => Some(Action::Command(cmd.to_string())),
        "lock-session" | "locks" => Some(Action::Command(cmd.to_string())),
        "refresh-client" | "refresh" => Some(Action::Command(cmd.to_string())),
        "suspend-client" | "suspendc" => Some(Action::Command(cmd.to_string())),
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
        Action::SessionChooser => "choose-session".to_string(),
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

/// Ensure a run-shell command uses -b (background) so it does not
/// set "running: ..." status messages or create output popups.
pub fn ensure_background(cmd: &str) -> String {
    let t = cmd.trim_start();
    let prefix = if t.starts_with("run-shell ") {
        Some("run-shell")
    } else if t.starts_with("run ") {
        Some("run")
    } else {
        None
    };
    if let Some(p) = prefix {
        let rest = t[p.len()..].trim_start();
        if !rest.starts_with("-b") {
            return format!("{} -b {}", p, rest);
        }
    }
    cmd.to_string()
}

/// Fire hooks for a given event.
/// All run-shell commands from hooks are forced into background mode
/// to avoid "running: ..." status bar noise and output popups.
pub fn fire_hooks(app: &mut AppState, event: &str) {
    if let Some(commands) = app.hooks.get(event).cloned() {
        for cmd in commands {
            let bg_cmd = ensure_background(&cmd);
            let _ = execute_command_string(app, &bg_cmd);
        }
    }
}
