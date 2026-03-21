use std::io;

use serde::{Serialize, Deserialize};

use crate::types::{AppState, Node};

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

#[derive(Serialize, Deserialize)]
pub struct PaneInfo { pub id: usize, pub title: String }

#[derive(Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::parse_command_line;

    #[test]
    fn test_quote_arg_simple() {
        assert_eq!(quote_arg("hello"), "\"hello\"");
    }

    #[test]
    fn test_quote_arg_with_spaces() {
        assert_eq!(quote_arg("cc 123"), "\"cc 123\"");
    }

    #[test]
    fn test_quote_arg_with_embedded_quotes() {
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn test_quote_arg_with_backslash() {
        assert_eq!(quote_arg("C:\\Users\\foo"), "\"C:\\\\Users\\\\foo\"");
    }

    #[test]
    fn test_quote_arg_empty() {
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn test_rename_session_roundtrip_with_spaces() {
        let name = "cc 123";
        let cmd = format!("rename-session {}", quote_arg(name));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["rename-session", "cc 123"]);
    }

    #[test]
    fn test_rename_window_roundtrip_with_spaces() {
        let name = "my window";
        let cmd = format!("rename-window {}", quote_arg(name));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["rename-window", "my window"]);
    }

    #[test]
    fn test_set_pane_title_roundtrip_with_spaces() {
        let title = "pane title here";
        let cmd = format!("set-pane-title {}", quote_arg(title));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["set-pane-title", "pane title here"]);
    }

    #[test]
    fn test_source_file_roundtrip_windows_path_with_spaces() {
        let path = "C:\\Program Files\\psmux\\config.conf";
        let cmd = format!("source-file {}", quote_arg(path));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["source-file", "C:\\Program Files\\psmux\\config.conf"]);
    }

    #[test]
    fn test_claim_session_roundtrip_with_spaces() {
        let name = "my session";
        let cwd = "C:\\Users\\My Name\\Documents";
        let cmd = format!("claim-session {} {}", quote_arg(name), quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "my session", "C:\\Users\\My Name\\Documents"]);
    }

    #[test]
    fn test_roundtrip_name_with_embedded_quotes() {
        let name = "say \"hello\" world";
        let cmd = format!("rename-session {}", quote_arg(name));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["rename-session", "say \"hello\" world"]);
    }

    #[test]
    fn test_roundtrip_no_spaces_still_works() {
        let name = "simple";
        let cmd = format!("rename-session {}", quote_arg(name));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["rename-session", "simple"]);
    }
}

pub fn color_to_name(c: vt100::Color) -> std::borrow::Cow<'static, str> {
    use std::borrow::Cow;
    match c {
        vt100::Color::Default => Cow::Borrowed("default"),
        vt100::Color::Idx(i) => {
            // Static lookup table for all 256 indexed colors
            static IDX_STRINGS: std::sync::LazyLock<[String; 256]> = std::sync::LazyLock::new(|| {
                std::array::from_fn(|i| format!("idx:{}", i))
            });
            Cow::Borrowed(&IDX_STRINGS[i as usize])
        }
        vt100::Color::Rgb(r,g,b) => Cow::Owned(format!("rgb:{},{},{}", r,g,b)),
    }
}
