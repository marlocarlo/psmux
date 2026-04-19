#[allow(unused_imports)]
use std::io;

use serde::{Serialize, Deserialize};

use crate::types::{AppState, Node};

/// Expand `~` to the user's home directory in a shell command string,
/// then rewrite `~/.psmux/plugins/` to `~/.config/psmux/plugins/` when
/// the classic path does not exist but the XDG path does (issue psmux-plugins#2).
use super::*;

pub fn expand_run_shell_path(cmd: &str) -> String {
    // Step 1: expand ~ to home directory
    let cmd = if cmd.contains('~') {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        cmd.replace("~/", &format!("{}/", home))
           .replace("~\\", &format!("{}\\", home))
    } else {
        cmd.to_string()
    };

    // Step 2: XDG fallback for plugin paths
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let classic_fwd = format!("{}/.psmux/plugins/", home);
    let classic_win = format!("{}\\.psmux\\plugins\\", home);
    if cmd.contains(&classic_fwd) || cmd.contains(&classic_win) {
        let classic_dir = std::path::Path::new(&home).join(".psmux").join("plugins");
        let xdg_base = std::env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| format!("{}\\.config", home));
        let xdg_dir = std::path::Path::new(&xdg_base).join("psmux").join("plugins");
        if !classic_dir.is_dir() && xdg_dir.is_dir() {
            let xdg_fwd = format!("{}/psmux/plugins/", xdg_base.replace('\\', "/"));
            let xdg_win = format!("{}\\psmux\\plugins\\", xdg_base);
            cmd.replace(&classic_fwd, &xdg_fwd).replace(&classic_win, &xdg_win)
        } else {
            cmd
        }
    } else {
        cmd
    }
}

pub fn infer_title_from_prompt(screen: &vt100::Screen, rows: u16, cols: u16) -> Option<String> {
    // Scan from cursor row (most likely prompt location) then fall back to last non-empty row
    let cursor_row = screen.cursor_position().0;
    let mut candidate_row: Option<u16> = None;
    // Try cursor row first, then scan downward, then scan upward
    for &r in [cursor_row].iter().chain((cursor_row + 1..rows).collect::<Vec<_>>().iter()).chain((0..cursor_row).rev().collect::<Vec<_>>().iter()) {
        let mut s = String::new();
        for c in 0..cols { if let Some(cell) = screen.cell(r, c) { s.push_str(cell.contents()); } else { s.push(' '); } }
        let t = s.trim_end();
        if !t.is_empty() && (t.contains('>') || t.contains('$') || t.contains('#') || t.contains(':')) {
            candidate_row = Some(r);
            break;
        }
    }
    // Fall back: use the row the cursor is on even if no prompt marker
    let row = candidate_row.unwrap_or(cursor_row);
    let mut s = String::new();
    for c in 0..cols { if let Some(cell) = screen.cell(row, c) { s.push_str(cell.contents()); } else { s.push(' '); } }
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() { return None; }
    // Only infer title from lines that look like prompts (contain a prompt marker)
    let has_prompt_marker = trimmed.contains('>') || trimmed.ends_with('$') || trimmed.ends_with('#');
    if !has_prompt_marker {
        // If no prompt marker, don't change the title — this is likely command output
        return None;
    }
    if let Some(pos) = trimmed.rfind('>') {
        let before = trimmed[..pos].trim().to_string();
        if before.contains("\\") || before.contains("/") {
            let parts: Vec<&str> = before.trim_matches(|ch: char| ch == '"').split(['\\','/']).collect();
            if let Some(base) = parts.last() { return Some(base.to_string()); }
        }
        return Some(before);
    }
    if let Some(pos) = trimmed.rfind('$') { return Some(trimmed[..pos].trim().to_string()); }
    if let Some(pos) = trimmed.rfind('#') { return Some(trimmed[..pos].trim().to_string()); }
    Some(trimmed)
}

// resolve_last_session_name and resolve_default_session_name are in session.rs

#[derive(Serialize, Deserialize)]
pub struct WinInfo { pub id: usize, pub name: String, pub active: bool, #[serde(default)] pub activity: bool, #[serde(default)] pub tab_text: String }

#[derive(Clone, Serialize, Deserialize)]
pub struct PaneInfo { pub id: usize, pub title: String }

#[derive(Clone, Serialize, Deserialize)]
pub struct WinTree { pub id: usize, pub name: String, pub active: bool, pub panes: Vec<PaneInfo> }

pub fn list_windows_json(app: &AppState) -> io::Result<String> {
    let mut v: Vec<WinInfo> = Vec::new();
    for (i, w) in app.windows.iter().enumerate() { v.push(WinInfo { id: w.id, name: w.name.clone(), active: i == app.active_idx, activity: w.activity_flag, tab_text: String::new() }); }
    let s = serde_json::to_string(&v).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("json error: {e}")))?;
    Ok(s)
}

/// tmux-compatible list-windows output: one line per window
/// Format: `<index>: <name><flag> (<pane_count> panes) [<width>x<height>]`
pub fn list_windows_tmux(app: &AppState) -> String {
    use crate::tree::*;
    fn count_panes(node: &Node) -> usize {
        match node {
            Node::Leaf(_) => 1,
            Node::Split { children, .. } => children.iter().map(|c| count_panes(c)).sum(),
        }
    }
    let mut lines = Vec::new();
    for (i, w) in app.windows.iter().enumerate() {
        let flag = if i == app.active_idx { "*" } else if w.activity_flag { "#" } else { "-" };
        let pane_count = count_panes(&w.root);
        let (width, height) = if let Some(p) = active_pane(&w.root, &w.active_path) {
            (p.last_cols, p.last_rows)
        } else { (120, 30) };
        lines.push(format!("{}: {}{} ({} panes) [{}x{}]", i + app.window_base_index, w.name, flag, pane_count, width, height));
    }
    lines.join("\n")
}

pub fn list_tree_json(app: &AppState) -> io::Result<String> {
    fn collect_panes(node: &Node, out: &mut Vec<PaneInfo>) {
        match node {
            Node::Leaf(p) => { out.push(PaneInfo { id: p.id, title: p.title.clone() }); }
            Node::Split { children, .. } => { for c in children.iter() { collect_panes(c, out); } }
        }
    }
    let mut v: Vec<WinTree> = Vec::new();
    for (i, w) in app.windows.iter().enumerate() {
        let mut panes = Vec::new();
        collect_panes(&w.root, &mut panes);
        v.push(WinTree { id: w.id, name: w.name.clone(), active: i == app.active_idx, panes });
    }
    let s = serde_json::to_string(&v).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("json error: {e}")))?;
    Ok(s)
}

pub const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &str) -> String {
    let bytes = data.as_bytes();
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        result.push(BASE64_CHARS[b0 >> 2] as char);
        result.push(BASE64_CHARS[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            result.push(BASE64_CHARS[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(BASE64_CHARS[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }
    result
}

pub fn base64_decode(encoded: &str) -> Option<String> {
    let mut result = Vec::new();
    let chars: Vec<u8> = encoded.bytes().filter(|&b| b != b'=').collect();
    for chunk in chars.chunks(4) {
        if chunk.len() < 2 { break; }
        let b0 = BASE64_CHARS.iter().position(|&c| c == chunk[0])? as u8;
        let b1 = BASE64_CHARS.iter().position(|&c| c == chunk[1])? as u8;
        result.push((b0 << 2) | (b1 >> 4));
        if chunk.len() > 2 {
            let b2 = BASE64_CHARS.iter().position(|&c| c == chunk[2])? as u8;
            result.push((b1 << 4) | (b2 >> 2));
            if chunk.len() > 3 {
                let b3 = BASE64_CHARS.iter().position(|&c| c == chunk[3])? as u8;
                result.push((b2 << 6) | b3);
            }
        }
    }
    String::from_utf8(result).ok()
}

/// Return color name as a string. Uses static strings for Default and
/// the 256 indexed colors to avoid heap allocations on every cell.
/// Quote and escape an argument for safe transmission over the control protocol.
/// Wraps the value in double quotes and escapes any embedded double quotes or backslashes.
pub fn quote_arg(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

/// Parse `VARIABLE=value` for tmux `new-session -e` / internal `server -e`
/// (split on the first `=` so values may contain `=`).
pub fn parse_env_assignment(s: &str) -> Result<(String, String), &'static str> {
    let s = s.trim();
    let eq = s.find('=').ok_or("expected VARIABLE=value")?;
    if eq == 0 {
        return Err("invalid environment variable name");
    }
    let name = &s[..eq];
    let value = &s[eq + 1..];
    if !is_valid_env_var_name(name) {
        return Err("invalid environment variable name");
    }
    Ok((name.to_string(), value.to_string()))
}

/// Parse the token after `-e` on `new-session` or internal `server -e`.
/// Shared by CLI short flags, control protocol `new-session`, and [`collect_server_session_env_args`].
pub fn parse_new_session_e_value_token(next_arg: Option<&str>) -> Result<(String, String), String> {
    let Some(s) = next_arg else {
        return Err("-e requires a value".to_string());
    };
    parse_env_assignment(s).map_err(|e| format!("invalid -e: {}", e))
}

pub(crate) fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    true
}

/// Merge CLI `new-session -e` pairs into session environment.
pub fn merge_session_env_into_app(app: &mut crate::types::AppState, session_env: &[(String, String)]) {
    for (k, v) in session_env {
        app.environment.insert(k.clone(), v.clone());
    }
}

/// Collect `-e` flags from internal `server` argv (only before `--`).
pub fn collect_server_session_env_args(args: &[String]) -> Result<Vec<(String, String)>, String> {
    let limit = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let mut out = Vec::new();
    let mut i = 0;
    while i < limit {
        if args[i] == "-e" {
            let pair = parse_new_session_e_value_token(args.get(i + 1).map(|s| s.as_str()))?;
            out.push(pair);
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(out)
}
