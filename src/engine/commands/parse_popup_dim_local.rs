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

pub(crate) fn parse_popup_dim_local(spec: &str, term_dim: u16, default: u16) -> u16 {
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

/// The default format string for `display-message` when no argument is given (tmux parity).
pub(crate) const DISPLAY_MESSAGE_DEFAULT_FMT: &str =
    "[#{session_name}] #{window_index}:#{window_name}#{window_flags} \"#{pane_title}\" #{pane_index} #{pane_current_command}";

/// Resolve the shell and its invocation prefix for `run-shell` commands.
/// Returns (program, prefix_args) where prefix_args are flags like ["-NoProfile", "-Command"].
/// On Windows: tries pwsh -> powershell -> cmd.
/// On Unix: uses sh -c.
pub fn resolve_run_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        if let Ok(path) = which::which("pwsh") {
            return (path.to_string_lossy().into_owned(), vec!["-NoProfile".to_string(), "-Command".to_string()]);
        }
        if let Ok(path) = which::which("powershell") {
            return (path.to_string_lossy().into_owned(), vec!["-NoProfile".to_string(), "-Command".to_string()]);
        }
        if let Ok(system_root) = std::env::var("SystemRoot").or_else(|_| std::env::var("SYSTEMROOT")) {
            let powershell = PathBuf::from(&system_root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe");
            if powershell.is_file() {
                return (powershell.to_string_lossy().into_owned(), vec!["-NoProfile".to_string(), "-Command".to_string()]);
            }
            let cmd = PathBuf::from(&system_root).join("System32").join("cmd.exe");
            if cmd.is_file() {
                return (cmd.to_string_lossy().into_owned(), vec!["/c".to_string()]);
            }
        }
        if let Ok(comspec) = std::env::var("ComSpec").or_else(|_| std::env::var("COMSPEC")) {
            let trimmed = comspec.trim();
            if !trimmed.is_empty() {
                return (trimmed.to_string(), vec!["/c".to_string()]);
            }
        }
        ("cmd".to_string(), vec!["/c".to_string()])
    }
    #[cfg(not(windows))]
    {
        ("sh".to_string(), vec!["-c".to_string()])
    }
}

/// Resolve a shell binary name to an available executable path.
/// Handles fallback between `pwsh` and `powershell` when one is not installed.
/// For `cmd`/`cmd.exe` or already-resolved paths, returns the input unchanged.
#[cfg(windows)]
pub(crate) fn resolve_shell_binary(name: &str) -> String {
    let lower = name.to_lowercase();
    let is_pwsh = lower == "pwsh" || lower == "pwsh.exe";
    let is_powershell = lower == "powershell" || lower == "powershell.exe";

    if is_pwsh {
        // Requested pwsh: verify it exists, fall back to powershell
        if which::which("pwsh").is_ok() {
            return name.to_string();
        }
        if let Ok(p) = which::which("powershell") {
            return p.to_string_lossy().into_owned();
        }
    } else if is_powershell {
        // Requested powershell: verify it exists, fall back to pwsh
        if which::which("powershell").is_ok() {
            return name.to_string();
        }
        if let Ok(p) = which::which("pwsh") {
            return p.to_string_lossy().into_owned();
        }
    }

    // cmd, cmd.exe, or already a full path: use as-is
    name.to_string()
}

/// Try to locate an existing file at the start of a command string.
/// Handles paths with spaces by progressively trying longer path prefixes
/// against the filesystem (e.g. "C:\Program Files\App\run.ps1 arg1 arg2"
/// tries "C:\Program", then "C:\Program Files\App\run.ps1", etc.).
/// Returns `Some((file_path, remaining_args))` on success.
#[cfg(windows)]
pub(crate) fn find_file_in_command(cmd: &str) -> Option<(String, String)> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() { return None; }
    let bytes = trimmed.as_bytes();
    let mut end = 0;
    loop {
        // Advance to the next whitespace boundary
        while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        let candidate = &trimmed[..end];
        if std::path::Path::new(candidate).is_file() {
            let rest = trimmed[end..].trim_start().to_string();
            return Some((candidate.to_string(), rest));
        }
        if end >= bytes.len() { return None; }
        // Skip whitespace to the next word
        while end < bytes.len() && bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        if end >= bytes.len() { return None; }
    }
}

/// Build a `std::process::Command` for a run-shell invocation.
///
/// Avoids double-wrapping when the command already starts with a shell binary
/// (e.g., `pwsh -NoProfile -File script.ps1`). Also detects file paths
/// (including those with spaces) and uses the appropriate execution strategy:
/// `-File` for `.ps1`, direct `Command::new` for `.exe`/`.cmd`/`.bat`,
/// and PowerShell call operator `& 'path'` for other files with spaces.
pub fn build_run_shell_command(shell_cmd: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use crate::platform::HideWindowCommandExt;
        let lower = shell_cmd.trim_start().to_lowercase();

        // Case 1: Command already starts with a shell binary (pwsh, powershell, cmd).
        // Run it directly to avoid nesting `pwsh -Command "pwsh -File ..."`.
        // If the specified shell isn't found, fall back to the alternative
        // (e.g. pwsh -> powershell) so plugin configs work on systems that
        // only have one of the two installed.
        if lower.starts_with("pwsh ") || lower.starts_with("pwsh.exe ")
            || lower.starts_with("powershell ") || lower.starts_with("powershell.exe ")
            || lower.starts_with("cmd ") || lower.starts_with("cmd.exe ")
        {
            let parts = parse_command_line(shell_cmd);
            if parts.len() >= 2 {
                let prog = resolve_shell_binary(&parts[0]);
                let mut c = std::process::Command::new(&prog);
                for p in &parts[1..] { c.arg(p); }
                c.hide_window();
                return c;
            }
        }

        // Case 2: File path detection (handles spaces in paths).
        // Uses progressive path probing: for "C:\Program Files\App\run.ps1 arg1",
        // tries "C:\Program" (not a file), then "C:\Program Files\App\run.ps1"
        // (found!), returning the file path and remaining arguments separately.
        let trimmed = shell_cmd.trim();
        // Strip matching outer quotes (single or double) so file detection works
        // for run-shell "'~/path/to/script.ps1'" syntax from config or CLI
        let trimmed = if (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
                       || (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2) {
            &trimmed[1..trimmed.len()-1]
        } else {
            trimmed
        };
        if let Some((file_path, rest_args)) = find_file_in_command(trimmed) {
            let lower_path = file_path.to_lowercase();

            // .ps1 scripts: use -File which never splits paths at whitespace
            if lower_path.ends_with(".ps1") {
                let shell = if which::which("pwsh").is_ok() { "pwsh" } else { "powershell" };
                let mut c = std::process::Command::new(shell);
                c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &file_path]);
                if !rest_args.is_empty() {
                    for a in &parse_command_line(&rest_args) { c.arg(a); }
                }
                c.hide_window();
                return c;
            }

            // For other file types with spaces in the path, we must avoid
            // the Case 3 shell wrapping which breaks on spaces.
            if file_path.contains(' ') {
                let ext = std::path::Path::new(&file_path).extension()
                    .and_then(|e| e.to_str()).map(|e| e.to_lowercase());

                match ext.as_deref() {
                    // Native executables: Command::new handles path quoting via CreateProcess
                    Some("exe") | Some("com") => {
                        let mut c = std::process::Command::new(&file_path);
                        if !rest_args.is_empty() {
                            for a in &parse_command_line(&rest_args) { c.arg(a); }
                        }
                        c.hide_window();
                        return c;
                    }
                    // Batch files: run via cmd.exe /c with the path as a separate arg
                    // so CreateProcess quotes just the path, not path+args together
                    Some("cmd") | Some("bat") => {
                        let mut c = std::process::Command::new("cmd.exe");
                        c.arg("/c");
                        c.arg(&file_path);
                        if !rest_args.is_empty() {
                            for a in &parse_command_line(&rest_args) { c.arg(a); }
                        }
                        c.hide_window();
                        return c;
                    }
                    // Unknown extension with spaces: use the resolved shell with
                    // proper quoting. For PowerShell, use the call operator & 'path'
                    // so the path is treated as a single literal string.
                    _ => {
                        let (shell_prog, shell_args) = resolve_run_shell();
                        let lower_shell = shell_prog.to_lowercase();
                        let is_powershell = lower_shell.contains("pwsh")
                            || lower_shell.contains("powershell");
                        let mut c = std::process::Command::new(&shell_prog);
                        for a in &shell_args { c.arg(a); }
                        if is_powershell {
                            let escaped = file_path.replace('\'', "''");
                            let wrapped = if rest_args.is_empty() {
                                format!("& '{}'", escaped)
                            } else {
                                format!("& '{}' {}", escaped, rest_args)
                            };
                            c.arg(&wrapped);
                        } else {
                            // cmd.exe /c: pass path and args separately
                            c.arg(&file_path);
                            if !rest_args.is_empty() {
                                for a in &parse_command_line(&rest_args) { c.arg(a); }
                            }
                        }
                        c.hide_window();
                        return c;
                    }
                }
            }
            // File found but path has no spaces: fall through to Case 3.
            // The simple shell wrapping works fine without spaces.
        }

        // Case 3: Regular command string (no file path with spaces detected).
        // Wrap in the resolved shell (pwsh -Command / cmd /c / sh -c).
        let (shell_prog, shell_args) = resolve_run_shell();
        let mut c = std::process::Command::new(&shell_prog);
        for a in &shell_args { c.arg(a); }
        c.arg(shell_cmd);
        c.hide_window();
        c
    }
    #[cfg(not(windows))]
    {
        let (shell_prog, shell_args) = resolve_run_shell();
        let mut c = std::process::Command::new(&shell_prog);
        for a in &shell_args { c.arg(a); }
        c.arg(shell_cmd);
        c
    }
}

/// Show text output in a popup overlay (used by list-* commands inside a session).
pub(crate) fn show_output_popup(app: &mut AppState, title: &str, output: String) {
    let lines: Vec<&str> = output.lines().collect();
    let width = lines.iter().map(|l| l.len()).max().unwrap_or(40).max(20) as u16 + 4;
    let height = (lines.len() as u16 + 2).max(5);
    app.mode = Mode::PopupMode {
        command: title.to_string(),
        output,
        process: None,
        width: width.min(120),
        height,
        close_on_exit: false,
        popup_pane: None,
        scroll_offset: 0,
    };
}

/// Generate list-windows output from AppState (tmux-compatible format).
pub(crate) fn generate_list_windows(app: &AppState) -> String {
    crate::util::list_windows_tmux(app)
}

/// Generate list-panes output from AppState.
pub(crate) fn generate_list_panes(app: &AppState) -> String {
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
pub(crate) fn generate_list_clients(app: &AppState) -> String {
    format!("/dev/pts/0: {}: {} [{}x{}] (utf8)\n",
        app.session_name,
        app.windows[app.active_idx].name,
        app.last_window_area.width,
        app.last_window_area.height)
}

/// Generate show-hooks output from AppState.
pub(crate) fn generate_show_hooks(app: &AppState) -> String {
    let mut output = String::new();
    for (name, commands) in &app.hooks {
        if commands.len() == 1 {
            output.push_str(&format!("{} -> {}\n", name, commands[0]));
        } else {
            for (i, cmd) in commands.iter().enumerate() {
                output.push_str(&format!("{}[{}] -> {}\n", name, i, cmd));
            }
        }
    }
    if output.is_empty() {
        output.push_str("(no hooks)\n");
    }
    output
}

/// Generate show-options output locally (embedded mode fallback).
pub(crate) fn generate_show_options(app: &AppState) -> String {
    let mut output = String::new();
    output.push_str(&format!("prefix {}\n", crate::config::format_key_binding(&app.prefix_key)));
    output.push_str(&format!("base-index {}\n", app.window_base_index));
    output.push_str(&format!("pane-base-index {}\n", app.pane_base_index));
    output.push_str(&format!("escape-time {}\n", app.escape_time_ms));
    output.push_str(&format!("mouse {}\n", if app.mouse_enabled { "on" } else { "off" }));
    output.push_str(&format!("scroll-enter-copy-mode {}\n", if app.scroll_enter_copy_mode { "on" } else { "off" }));
    output.push_str(&format!("status {}\n", if app.status_visible { "on" } else { "off" }));
    output.push_str(&format!("status-position {}\n", app.status_position));
    output.push_str(&format!("status-left \"{}\"\n", app.status_left));
    output.push_str(&format!("status-right \"{}\"\n", app.status_right));
    output.push_str(&format!("history-limit {}\n", app.history_limit));
    output.push_str(&format!("display-time {}\n", app.display_time_ms));
    output.push_str(&format!("mode-keys {}\n", app.mode_keys));
    output.push_str(&format!("focus-events {}\n", if app.focus_events { "on" } else { "off" }));
    output.push_str(&format!("renumber-windows {}\n", if app.renumber_windows { "on" } else { "off" }));
    output.push_str(&format!("automatic-rename {}\n", if app.automatic_rename { "on" } else { "off" }));
    output.push_str(&format!("monitor-activity {}\n", if app.monitor_activity { "on" } else { "off" }));
    output.push_str(&format!("synchronize-panes {}\n", if app.sync_input { "on" } else { "off" }));
    output.push_str(&format!("remain-on-exit {}\n", if app.remain_on_exit { "on" } else { "off" }));
    output.push_str(&format!("allow-predictions {}\n", if app.allow_predictions { "on" } else { "off" }));
    // Include @user-options
    for (key, val) in &app.user_options {
        output.push_str(&format!("{} \"{}\"\n", key, val));
    }
    output
}
