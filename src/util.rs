use std::io;

use serde::{Serialize, Deserialize};

use crate::types::{AppState, Node};

/// Expand `~` to the user's home directory in a shell command string,
/// then rewrite `~/.psmux/plugins/` to `~/.config/psmux/plugins/` when
/// the classic path does not exist but the XDG path does (issue psmux-plugins#2).
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

fn is_valid_env_var_name(name: &str) -> bool {
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

    #[test]
    fn test_claim_session_roundtrip_root_dir() {
        // Root paths like C:\ end in a backslash which must survive
        // the quote_arg -> parse_command_line roundtrip.
        let name = "mysession";
        let cwd = "C:\\";
        let cmd = format!("claim-session {} {}", quote_arg(name), quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "mysession", "C:\\"]);
    }

    #[test]
    fn test_claim_session_roundtrip_trailing_backslash_dir() {
        // Paths ending in backslash (e.g. D:\Projects\) must roundtrip.
        let cwd = "D:\\Projects\\";
        let cmd = format!("claim-session sess {}", quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "sess", "D:\\Projects\\"]);
    }

    #[test]
    fn test_claim_session_roundtrip_path_with_spaces() {
        let cwd = "C:\\Program Files\\My App\\Data";
        let cmd = format!("claim-session s1 {}", quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "s1", "C:\\Program Files\\My App\\Data"]);
    }

    #[test]
    fn test_claim_session_roundtrip_deep_nested_path() {
        let cwd = "C:\\Users\\test\\Documents\\workspace\\project\\src\\components";
        let cmd = format!("claim-session s1 {}", quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "s1", cwd]);
    }

    #[test]
    fn test_claim_session_roundtrip_unc_path() {
        let cwd = "\\\\server\\share\\folder";
        let cmd = format!("claim-session s1 {}", quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "s1", "\\\\server\\share\\folder"]);
    }

    #[test]
    fn test_claim_session_roundtrip_path_with_parens() {
        let cwd = "C:\\Program Files (x86)\\App";
        let cmd = format!("claim-session s1 {}", quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "s1", "C:\\Program Files (x86)\\App"]);
    }

    #[test]
    fn test_claim_session_roundtrip_path_with_ampersand() {
        let cwd = "C:\\R&D\\project";
        let cmd = format!("claim-session s1 {}", quote_arg(cwd));
        let args = parse_command_line(&cmd);
        assert_eq!(args, vec!["claim-session", "s1", "C:\\R&D\\project"]);
    }

    /// Verify that send-keys with Claude Code agent spawn commands preserves
    /// Windows paths and POSIX-escaped characters (psmux#172, #173, #180).
    /// The CLI wraps the key in double-quotes without escaping backslashes,
    /// and parse_command_line keeps lone backslashes literal (Windows paths).
    #[test]
    fn test_send_keys_claude_code_agent_command_preserves_backslashes() {
        // Simulate the control-protocol line built by the CLI send-keys handler:
        // send-keys "cd 'C:\path with spaces' && env CLAUDECODE=1 'C:\...\claude.exe' --agent-id ..." Enter
        let agent_cmd = "cd 'C:\\cctest\\a long dir name' && env CLAUDECODE=1 'C:\\Users\\foo\\.local\\bin\\claude.exe' --agent-id researcher\\@my-team";
        let line = format!("send-keys \"{}\" Enter", agent_cmd);
        let args = parse_command_line(&line);
        assert_eq!(args[0], "send-keys");
        assert_eq!(args[1], agent_cmd);
        assert_eq!(args[2], "Enter");
    }

    #[test]
    fn test_send_keys_single_quoted_windows_path() {
        // Single-quoted paths from shell-quote: 'C:\Users\foo'
        let line = "send-keys \"cd 'C:\\Users\\foo\\project'\" Enter";
        let args = parse_command_line(line);
        assert_eq!(args[1], "cd 'C:\\Users\\foo\\project'");
    }

    #[test]
    fn parse_env_assignment_basic() {
        assert_eq!(
            parse_env_assignment("FOO=bar").unwrap(),
            ("FOO".to_string(), "bar".to_string())
        );
    }

    #[test]
    fn parse_env_assignment_empty_value() {
        assert_eq!(
            parse_env_assignment("VAR=").unwrap(),
            ("VAR".to_string(), "".to_string())
        );
    }

    #[test]
    fn parse_env_assignment_value_with_equals() {
        assert_eq!(
            parse_env_assignment("FOO=a=b=c").unwrap(),
            ("FOO".to_string(), "a=b=c".to_string())
        );
    }

    #[test]
    fn parse_env_assignment_rejects_no_equals() {
        assert!(parse_env_assignment("FOO").is_err());
    }

    #[test]
    fn parse_env_assignment_rejects_bad_name() {
        assert!(parse_env_assignment("123=x").is_err());
        assert!(parse_env_assignment("bad-name=x").is_err());
    }

    #[test]
    fn parse_new_session_e_value_token_missing() {
        assert_eq!(
            parse_new_session_e_value_token(None).unwrap_err(),
            "-e requires a value"
        );
    }

    #[test]
    fn parse_new_session_e_value_token_ok() {
        let p = parse_new_session_e_value_token(Some("Z=1")).unwrap();
        assert_eq!(p, ("Z".to_string(), "1".to_string()));
    }

    #[test]
    fn collect_server_session_env_skips_after_dd() {
        let args: Vec<String> = vec![
            "psmux".into(), "server".into(), "-s".into(), "s1".into(),
            "-e".into(), "A=1".into(),
            "--".into(), "cmd".into(), "-e".into(), "IGNORE=me".into(),
        ];
        let v = collect_server_session_env_args(&args).unwrap();
        assert_eq!(v, vec![("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn collect_server_session_env_duplicate_key_last_wins() {
        let args: Vec<String> = vec![
            "psmux".into(), "server".into(), "-s".into(), "s1".into(),
            "-e".into(), "FOO=first".into(),
            "-e".into(), "FOO=last".into(),
        ];
        let v = collect_server_session_env_args(&args).unwrap();
        assert_eq!(v.len(), 2);
        let mut app = crate::types::AppState::new("t".to_string());
        merge_session_env_into_app(&mut app, &v);
        assert_eq!(app.environment.get("FOO").map(|s| s.as_str()), Some("last"));
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
