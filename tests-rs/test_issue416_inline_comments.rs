// Tests for issue #416: a trailing inline `# comment` on a config line
// broke parsing.
//
// The config executor only skipped whole-line comments (lines whose first
// non-whitespace char is `#`). A line like `set -g base-index 1   # test`
// was handed verbatim to the option parser, so the directive never took
// effect — while the same line without the comment worked.
//
// `strip_inline_comment` removes an unquoted, whitespace-preceded `#`
// comment while preserving a `#` inside quotes (formats like "#{...}"),
// an escaped `\#`, and a mid-word `#` (e.g. a `colour#aabbcc` token).

use super::*;

fn mock_app() -> AppState {
    AppState::new("test_session".to_string())
}

// ── strip_inline_comment unit behaviour ───────────────────────────

#[test]
fn strips_trailing_comment_after_whitespace() {
    assert_eq!(
        strip_inline_comment("set -g base-index 1   # test"),
        "set -g base-index 1"
    );
}

#[test]
fn strips_comment_with_single_space() {
    assert_eq!(
        strip_inline_comment("set -g base-index 1 #c"),
        "set -g base-index 1"
    );
}

#[test]
fn keeps_line_without_comment() {
    assert_eq!(
        strip_inline_comment("set -g base-index 1"),
        "set -g base-index 1"
    );
}

#[test]
fn preserves_hash_inside_double_quotes() {
    let line = r#"set -g status-left "#H session""#;
    assert_eq!(strip_inline_comment(line), line);
}

#[test]
fn preserves_hash_inside_single_quotes() {
    let line = "set -g status-right '#{session_name}'";
    assert_eq!(strip_inline_comment(line), line);
}

#[test]
fn strips_comment_after_quoted_value() {
    assert_eq!(
        strip_inline_comment(r#"set -g status-left "#H"   # hostname"#),
        r#"set -g status-left "#H""#
    );
}

#[test]
fn preserves_mid_word_hash() {
    // An unquoted `#` that is not preceded by whitespace is part of the token
    // (matching tmux), so it is not a comment.
    assert_eq!(
        strip_inline_comment("set -g foo bar#baz"),
        "set -g foo bar#baz"
    );
}

#[test]
fn preserves_escaped_hash() {
    assert_eq!(
        strip_inline_comment(r"send-keys \# Enter"),
        r"send-keys \# Enter"
    );
}

#[test]
fn whole_line_comment_strips_to_empty() {
    // Leading `#`: the start of line is treated as a word boundary, so the
    // entire line is a comment.
    assert_eq!(strip_inline_comment("# a comment"), "");
}

// ── end-to-end: directives take effect despite a trailing comment ──

#[test]
fn inline_comment_does_not_break_set_option() {
    let mut app = mock_app();
    parse_config_content(&mut app, "set -g base-index 1   # test\n");
    assert_eq!(
        app.window_base_index, 1,
        "base-index should be 1 with a trailing inline comment"
    );
}

#[test]
fn inline_comment_matches_no_comment_behaviour() {
    let mut with_comment = mock_app();
    parse_config_content(&mut with_comment, "set -g pane-base-index 1  # comment\n");

    let mut without_comment = mock_app();
    parse_config_content(&mut without_comment, "set -g pane-base-index 1\n");

    assert_eq!(with_comment.pane_base_index, without_comment.pane_base_index);
    assert_eq!(with_comment.pane_base_index, 1);
}

#[test]
fn quoted_format_value_survives_comment_stripping() {
    let mut app = mock_app();
    parse_config_content(
        &mut app,
        r#"set -g status-left "#{session_name}"  # show session"#,
    );
    assert_eq!(
        app.status_left, "#{session_name}",
        "a quoted #{{...}} format must not be truncated by comment stripping"
    );
}
