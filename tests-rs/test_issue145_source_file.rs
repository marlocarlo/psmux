// Tests for issue #145: source-file command not working inside a session.
// Validates:
// 1. source-file with tilde (~) path expansion
// 2. source-file with backslash tilde (~\) Windows paths
// 3. source-file with forward-slash tilde (~/) Unix-style paths
// 4. UTF-8 BOM handling (first line must not be silently dropped)
// 5. source-file via parse_config_content (direct parsing path)
// 6. Missing file handling
// 7. Multiple config options applied in a single source-file

use super::*;

fn mock_app() -> AppState {
    AppState::new("test_session".to_string())
}

// ═══════════════════════════════════════════════════════════════════
//  Basic source-file with absolute path
// ═══════════════════════════════════════════════════════════════════

#[test]
fn source_file_applies_status_left() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_status.conf");
    std::fs::write(&tmp, "set -g status-left 'SOURCED_OK'\n").unwrap();
    source_file(&mut app, &tmp.display().to_string());
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.status_left, "SOURCED_OK", "source-file should update status-left");
}

#[test]
fn source_file_applies_multiple_options() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_multi.conf");
    std::fs::write(&tmp, "set -g status-left 'LEFT_VAL'\nset -g status-right 'RIGHT_VAL'\nset -g history-limit 7777\n").unwrap();
    source_file(&mut app, &tmp.display().to_string());
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.status_left, "LEFT_VAL");
    assert_eq!(app.status_right, "RIGHT_VAL");
    assert_eq!(app.history_limit, 7777);
}

// ═══════════════════════════════════════════════════════════════════
//  Tilde path expansion
// ═══════════════════════════════════════════════════════════════════

#[test]
fn source_file_tilde_backslash_expansion() {
    let mut app = mock_app();
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap();
    let filename = ".psmux_test_145_tilde.conf";
    let full_path = format!("{}\\{}", home, filename);
    std::fs::write(&full_path, "set -g history-limit 4444\n").unwrap();

    let tilde_path = format!("~\\{}", filename);
    source_file(&mut app, &tilde_path);
    let _ = std::fs::remove_file(&full_path);
    assert_eq!(app.history_limit, 4444, "source-file with ~\\ should expand tilde");
}

#[test]
fn source_file_tilde_forward_slash_expansion() {
    let mut app = mock_app();
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap();
    let filename = ".psmux_test_145_tilde_fwd.conf";
    let full_path = format!("{}\\{}", home, filename);
    std::fs::write(&full_path, "set -g history-limit 3333\n").unwrap();

    let tilde_path = format!("~/{}", filename);
    source_file(&mut app, &tilde_path);
    let _ = std::fs::remove_file(&full_path);
    assert_eq!(app.history_limit, 3333, "source-file with ~/ should expand tilde");
}

// ═══════════════════════════════════════════════════════════════════
//  UTF-8 BOM handling
// ═══════════════════════════════════════════════════════════════════

#[test]
fn source_file_bom_first_line_not_dropped() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_bom.conf");
    let bom = "\u{FEFF}";
    let content = format!("{}set -g history-limit 6666\nset -g status-left 'BOM_OK'\n", bom);
    std::fs::write(&tmp, content).unwrap();

    source_file(&mut app, &tmp.display().to_string());
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.history_limit, 6666, "first line after BOM must be parsed");
    assert_eq!(app.status_left, "BOM_OK", "second line after BOM must be parsed");
}

#[test]
fn parse_config_content_bom_stripped() {
    let mut app = mock_app();
    let bom_content = "\u{FEFF}set -g history-limit 5555\nset -g status-right 'BOM_STRIP'\n";
    parse_config_content(&mut app, bom_content);
    assert_eq!(app.history_limit, 5555, "parse_config_content should strip BOM from first line");
    assert_eq!(app.status_right, "BOM_STRIP");
}

#[test]
fn parse_config_content_no_bom_still_works() {
    let mut app = mock_app();
    let content = "set -g history-limit 1234\n";
    parse_config_content(&mut app, content);
    assert_eq!(app.history_limit, 1234, "content without BOM should still parse normally");
}

// ═══════════════════════════════════════════════════════════════════
//  Missing file handling
// ═══════════════════════════════════════════════════════════════════

#[test]
fn source_file_missing_file_does_not_crash() {
    let mut app = mock_app();
    source_file(&mut app, "/nonexistent/path/config.conf");
    assert_eq!(app.history_limit, 2000, "missing file should not change defaults");
}

// ═══════════════════════════════════════════════════════════════════
//  Quoted paths
// ═══════════════════════════════════════════════════════════════════

#[test]
fn source_file_quoted_path() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_quoted.conf");
    std::fs::write(&tmp, "set -g history-limit 2222\n").unwrap();
    let quoted = format!("\"{}\"", tmp.display());
    source_file(&mut app, &quoted);
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.history_limit, 2222, "source-file should handle quoted paths");
}

#[test]
fn source_file_single_quoted_path() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_sq.conf");
    std::fs::write(&tmp, "set -g history-limit 1111\n").unwrap();
    let quoted = format!("'{}'", tmp.display());
    source_file(&mut app, &quoted);
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.history_limit, 1111, "source-file should handle single-quoted paths");
}

// ═══════════════════════════════════════════════════════════════════
//  Config content with bind-key inside
// ═══════════════════════════════════════════════════════════════════

#[test]
fn source_file_with_bind_key_inside() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_bind.conf");
    std::fs::write(&tmp, "bind-key r source-file ~/.tmux.conf\nset -g status-left 'BIND_OK'\n").unwrap();
    source_file(&mut app, &tmp.display().to_string());
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.status_left, "BIND_OK", "source-file with bind-key inside should work");
}

// ═══════════════════════════════════════════════════════════════════
//  Windows line endings (CRLF)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn source_file_crlf_line_endings() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_crlf.conf");
    std::fs::write(&tmp, "set -g history-limit 7070\r\nset -g status-left 'CRLF_OK'\r\n").unwrap();
    source_file(&mut app, &tmp.display().to_string());
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.history_limit, 7070, "CRLF line endings should parse correctly");
    assert_eq!(app.status_left, "CRLF_OK");
}

#[test]
fn source_file_bom_plus_crlf() {
    let mut app = mock_app();
    let tmp = std::env::temp_dir().join("psmux_test_145_bom_crlf.conf");
    let content = "\u{FEFF}set -g history-limit 9090\r\nset -g status-left 'BOM_CRLF_OK'\r\n";
    std::fs::write(&tmp, content).unwrap();
    source_file(&mut app, &tmp.display().to_string());
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(app.history_limit, 9090, "BOM + CRLF should both be handled");
    assert_eq!(app.status_left, "BOM_CRLF_OK");
}
