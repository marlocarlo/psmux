#[allow(unused_imports)]
use std::io;

use serde::{Serialize, Deserialize};

use crate::types::{AppState, Node};

/// Expand `~` to the user's home directory in a shell command string,
/// then rewrite `~/.psmux/plugins/` to `~/.config/psmux/plugins/` when
/// the classic path does not exist but the XDG path does (issue psmux-plugins#2).
use super::*;

#[cfg(test)]
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