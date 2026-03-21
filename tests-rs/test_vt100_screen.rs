use super::*;

// ── parse_osc7_uri tests ──────────────────────────────────

#[test]
fn osc7_full_uri_with_hostname() {
    assert_eq!(parse_osc7_uri("file://myhost/home/user/project"), "/home/user/project");
}

#[test]
fn osc7_localhost() {
    assert_eq!(parse_osc7_uri("file://localhost/home/user"), "/home/user");
}

#[test]
fn osc7_empty_hostname() {
    assert_eq!(parse_osc7_uri("file:///home/user"), "/home/user");
}

#[test]
fn osc7_bare_path_no_scheme() {
    assert_eq!(parse_osc7_uri("/home/user/code"), "/home/user/code");
}

#[test]
fn osc7_percent_encoded_spaces() {
    assert_eq!(parse_osc7_uri("file:///home/user/my%20project"), "/home/user/my project");
}

#[test]
fn osc7_percent_encoded_special_chars() {
    assert_eq!(parse_osc7_uri("file:///path/%23hash%25pct"), "/path/#hash%pct");
}

#[test]
fn osc7_windows_path_via_uri() {
    // WezTerm-style: file://hostname/C:/Users/foo
    assert_eq!(parse_osc7_uri("file://DESKTOP-ABC/C:/Users/foo"), "/C:/Users/foo");
}

#[test]
fn osc7_empty_string() {
    assert_eq!(parse_osc7_uri(""), "");
}

#[test]
fn osc7_file_no_slash_after_host() {
    // Malformed: file://hostname-only (no path)
    assert_eq!(parse_osc7_uri("file://hostname-only"), "hostname-only");
}

// ── OSC title tests ──────────────────────────────────────────

#[test]
fn screen_title_initially_empty() {
    let screen = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    assert_eq!(screen.title(), "");
}

#[test]
fn screen_set_title_from_utf8() {
    let mut screen = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    screen.set_title(b"my terminal");
    assert_eq!(screen.title(), "my terminal");
}

#[test]
fn screen_set_title_overwrites() {
    let mut screen = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    screen.set_title(b"first");
    screen.set_title(b"second");
    assert_eq!(screen.title(), "second");
}

#[test]
fn screen_set_title_empty_string() {
    let mut screen = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    screen.set_title(b"something");
    screen.set_title(b"");
    assert_eq!(screen.title(), "");
}

#[test]
fn screen_set_title_invalid_utf8_ignored() {
    let mut screen = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    screen.set_title(b"good");
    screen.set_title(&[0xff, 0xfe, 0xfd]);
    // Invalid UTF-8 should be ignored, old title preserved
    assert_eq!(screen.title(), "good");
}

// ── percent_decode tests ──────────────────────────────────

#[test]
fn decode_no_encoding() {
    assert_eq!(percent_decode("/simple/path"), "/simple/path");
}

#[test]
fn decode_space() {
    assert_eq!(percent_decode("/my%20path"), "/my path");
}

#[test]
fn decode_mixed_case_hex() {
    assert_eq!(percent_decode("%2f%2F"), "//");
}

#[test]
fn decode_invalid_hex_passthrough() {
    assert_eq!(percent_decode("%ZZ"), "%ZZ");
}

#[test]
fn decode_truncated_percent() {
    assert_eq!(percent_decode("trail%2"), "trail%2");
}

// ── Screen::set_path / path() integration ─────────────────

#[test]
fn screen_path_initially_none() {
    let s = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    assert!(s.path().is_none());
}

#[test]
fn screen_set_path_from_osc7() {
    let mut s = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    s.set_path(b"file:///home/user/code");
    assert_eq!(s.path(), Some("/home/user/code"));
}

#[test]
fn screen_set_path_overwrites() {
    let mut s = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    s.set_path(b"file:///first");
    s.set_path(b"file:///second");
    assert_eq!(s.path(), Some("/second"));
}

#[test]
fn screen_set_path_ignores_invalid_utf8() {
    let mut s = Screen::new(crate::grid::Size { rows: 24, cols: 80 }, 0);
    s.set_path(&[0xff, 0xfe, 0xfd]);
    assert!(s.path().is_none());
}

// ── Full parser round-trip via VTE ─────────────────────────

#[test]
fn parser_osc7_roundtrip() {
    let mut parser = crate::Parser::new(24, 80, 0);
    // OSC 7 ; file:///tmp/test ST
    parser.process(b"\x1b]7;file:///tmp/test\x1b\\");
    assert_eq!(parser.screen().path(), Some("/tmp/test"));
}

#[test]
fn parser_osc7_bel_terminated() {
    let mut parser = crate::Parser::new(24, 80, 0);
    // OSC 7 ; file://host/path BEL
    parser.process(b"\x1b]7;file://host/home/user\x07");
    assert_eq!(parser.screen().path(), Some("/home/user"));
}

#[test]
fn parser_osc7_with_percent_encoding() {
    let mut parser = crate::Parser::new(24, 80, 0);
    parser.process(b"\x1b]7;file:///home/user/my%20project\x07");
    assert_eq!(parser.screen().path(), Some("/home/user/my project"));
}

#[test]
fn parser_osc7_updates_on_cd() {
    let mut parser = crate::Parser::new(24, 80, 0);
    parser.process(b"\x1b]7;file:///first/dir\x07");
    assert_eq!(parser.screen().path(), Some("/first/dir"));
    parser.process(b"\x1b]7;file:///second/dir\x07");
    assert_eq!(parser.screen().path(), Some("/second/dir"));
}

#[test]
fn parser_other_osc_does_not_affect_path() {
    let mut parser = crate::Parser::new(24, 80, 0);
    // OSC 0 (set title) should not touch path
    parser.process(b"\x1b]0;my-title\x07");
    assert!(parser.screen().path().is_none());
}
