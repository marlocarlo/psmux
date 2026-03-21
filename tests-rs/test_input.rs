use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

/// Helper: build a KeyEvent with the given code and modifiers.
fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

// ── AltGr characters (Ctrl+Alt on Windows) should be forwarded verbatim ──

#[test]
fn altgr_backslash_german_layout() {
    // German: AltGr+ß → '\'   reported as Ctrl+Alt+'\'
    let ev = key(KeyCode::Char('\\'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\\", "AltGr+backslash must produce literal backslash");
}

#[test]
fn altgr_at_sign_german_layout() {
    // German: AltGr+Q → '@'   reported as Ctrl+Alt+'@'
    let ev = key(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"@", "AltGr+@ must produce literal @");
}

#[test]
fn altgr_open_curly_brace() {
    // German: AltGr+7 → '{'   reported as Ctrl+Alt+'{'
    let ev = key(KeyCode::Char('{'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"{", "AltGr+{{ must produce literal {{");
}

#[test]
fn altgr_close_curly_brace() {
    // German: AltGr+0 → '}'
    let ev = key(KeyCode::Char('}'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"}", "AltGr+}} must produce literal }}");
}

#[test]
fn altgr_open_bracket() {
    // German: AltGr+8 → '['
    let ev = key(KeyCode::Char('['), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"[", "AltGr+[ must produce literal [");
}

#[test]
fn altgr_close_bracket() {
    // German: AltGr+9 → ']'
    let ev = key(KeyCode::Char(']'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"]", "AltGr+] must produce literal ]");
}

#[test]
fn altgr_pipe() {
    // German: AltGr+< → '|'
    let ev = key(KeyCode::Char('|'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"|", "AltGr+| must produce literal |");
}

#[test]
fn altgr_tilde() {
    // German: AltGr++ → '~'
    let ev = key(KeyCode::Char('~'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"~", "AltGr+~ must produce literal ~");
}

#[test]
fn altgr_euro_sign() {
    // German: AltGr+E → '€'   (multi-byte UTF-8)
    let ev = key(KeyCode::Char('€'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, "€".as_bytes(), "AltGr+euro must produce UTF-8 euro sign");
}

#[test]
fn altgr_dollar_czech_layout() {
    // Czech: AltGr produces '$'
    let ev = key(KeyCode::Char('$'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"$", "AltGr+$ must produce literal $");
}

// ── Genuine Ctrl+Alt+letter must still produce ESC + ctrl-char ──

#[test]
fn ctrl_alt_a_is_esc_ctrl_a() {
    let ev = key(KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, vec![0x1b, 0x01], "Ctrl+Alt+a → ESC + ^A");
}

#[test]
fn ctrl_alt_c_is_esc_ctrl_c() {
    let ev = key(KeyCode::Char('c'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, vec![0x1b, 0x03], "Ctrl+Alt+c → ESC + ^C");
}

#[test]
fn ctrl_alt_z_is_esc_ctrl_z() {
    let ev = key(KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, vec![0x1b, 0x1a], "Ctrl+Alt+z → ESC + ^Z");
}

// ── Plain characters / other modifier combos (regression checks) ──

#[test]
fn plain_char_no_modifiers() {
    let ev = key(KeyCode::Char('a'), KeyModifiers::NONE);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"a");
}

#[test]
fn alt_a_produces_esc_a() {
    let ev = key(KeyCode::Char('a'), KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\x1ba");
}

#[test]
fn ctrl_a_produces_soh() {
    let ev = key(KeyCode::Char('a'), KeyModifiers::CONTROL);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, vec![0x01]); // ^A = SOH
}

#[test]
fn plain_backslash_no_modifiers() {
    let ev = key(KeyCode::Char('\\'), KeyModifiers::NONE);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\\");
}

// ── Modified Enter key tests (PR #115) ──

#[test]
fn plain_enter_produces_cr() {
    let ev = key(KeyCode::Enter, KeyModifiers::NONE);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\r", "plain Enter must produce CR");
}

#[test]
fn shift_enter_produces_correct_encoding() {
    let ev = key(KeyCode::Enter, KeyModifiers::SHIFT);
    let bytes = encode_key_event(&ev).unwrap();
    #[cfg(windows)]
    assert_eq!(bytes, b"\x1b\r", "Shift+Enter on Windows must produce ESC+CR for ConPTY");
    #[cfg(not(windows))]
    assert_eq!(bytes, b"\x1b[13;2~", "Shift+Enter must produce CSI 13;2~");
}

#[test]
fn ctrl_enter_produces_csi_13_5() {
    let ev = key(KeyCode::Enter, KeyModifiers::CONTROL);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\x1b[13;5~", "Ctrl+Enter must produce CSI 13;5~");
}

#[test]
fn ctrl_shift_enter_produces_csi_13_6() {
    let ev = key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\x1b[13;6~", "Ctrl+Shift+Enter must produce CSI 13;6~");
}

#[test]
fn alt_enter_produces_correct_encoding() {
    let ev = key(KeyCode::Enter, KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    #[cfg(windows)]
    assert_eq!(bytes, b"\x1b\r", "Alt+Enter on Windows must produce ESC+CR for ConPTY");
    #[cfg(not(windows))]
    assert_eq!(bytes, b"\x1b[13;3~", "Alt+Enter must produce CSI 13;3~");
}

// ── parse_modified_special_key tests (PR #115) ──

#[test]
fn parse_shift_enter() {
    assert_eq!(parse_modified_special_key("S-Enter"), Some("\x1b[13;2~".to_string()));
}

#[test]
fn parse_ctrl_enter() {
    assert_eq!(parse_modified_special_key("C-Enter"), Some("\x1b[13;5~".to_string()));
}

#[test]
fn parse_ctrl_shift_enter() {
    assert_eq!(parse_modified_special_key("C-S-Enter"), Some("\x1b[13;6~".to_string()));
}

#[test]
fn parse_plain_enter_returns_none() {
    assert_eq!(parse_modified_special_key("enter"), None, "no modifiers should return None");
}

#[test]
fn parse_shift_left_works() {
    // Regression: S-Left was broken because m started at 1 and S- did m|=1 (no-op)
    assert_eq!(parse_modified_special_key("S-Left"), Some("\x1b[1;2D".to_string()));
}

#[test]
fn parse_ctrl_tab_unchanged() {
    assert_eq!(parse_modified_special_key("C-Tab"), Some("\x1b[9;5~".to_string()));
}

#[test]
fn parse_ctrl_left_unchanged() {
    assert_eq!(parse_modified_special_key("C-Left"), Some("\x1b[1;5D".to_string()));
}

// ── PR #131: paste line-ending normalization tests ──

/// Helper: capture what write_paste_chunked writes to a Vec<u8>.
fn capture_paste(text: &[u8], bracket: bool) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    super::write_paste_chunked(&mut buf, text, bracket);
    buf
}

#[test]
fn paste_lf_normalized_to_cr() {
    // Multi-line paste with LF line endings should produce CR
    let input = b"line1\nline2\nline3";
    let output = capture_paste(input, false);
    assert_eq!(output, b"line1\rline2\rline3",
        "bare LF must be normalized to CR for ConPTY; got {:?}", String::from_utf8_lossy(&output));
}

#[test]
fn paste_crlf_normalized_to_cr() {
    // Multi-line paste with CRLF line endings should produce CR (not CRLF)
    let input = b"line1\r\nline2\r\nline3";
    let output = capture_paste(input, false);
    assert_eq!(output, b"line1\rline2\rline3",
        "CRLF must be normalized to CR for ConPTY; got {:?}", String::from_utf8_lossy(&output));
}

#[test]
fn paste_mixed_endings_normalized() {
    // Mixed: some lines LF, some CRLF
    let input = b"a\nb\r\nc";
    let output = capture_paste(input, false);
    assert_eq!(output, b"a\rb\rc",
        "mixed line endings must all become CR; got {:?}", String::from_utf8_lossy(&output));
}

#[test]
fn paste_no_line_endings_unchanged() {
    // Text without newlines should pass through unchanged
    let input = b"hello world";
    let output = capture_paste(input, false);
    assert_eq!(output, b"hello world");
}

#[test]
fn paste_bracket_markers_with_normalization() {
    // Bracketed paste should still wrap with markers AND normalize
    let input = b"a\nb";
    let output = capture_paste(input, true);
    assert_eq!(output, b"\x1b[200~a\rb\x1b[201~",
        "bracketed paste must normalize line endings; got {:?}", String::from_utf8_lossy(&output));
}

// ── PR #132: Shift+Enter ConPTY encoding tests ──

#[cfg(windows)]
#[test]
fn shift_enter_encoding_for_conpty() {
    // On Windows, Shift+Enter should produce \x1b\r (ESC+CR) instead of
    // \x1b[13;2~ which ConPTY drops (code 13 is non-standard).
    let ev = key(KeyCode::Enter, KeyModifiers::SHIFT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\x1b\r",
        "Shift+Enter on Windows must produce ESC+CR for ConPTY compatibility; got {:?}", bytes);
}

#[cfg(windows)]
#[test]
fn alt_enter_encoding_for_conpty() {
    // Alt+Enter should also produce \x1b\r on Windows
    let ev = key(KeyCode::Enter, KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\x1b\r",
        "Alt+Enter on Windows must produce ESC+CR for ConPTY; got {:?}", bytes);
}

// ── Issue #121 (whil0012 follow-up): PSReadLine Shift+Enter via native injection ──

/// augment_enter_shift must remap Alt+Enter → Shift+Enter when physical Shift
/// is held (ConPTY misreports Shift+Enter as Alt+Enter).
#[cfg(windows)]
#[test]
fn augment_enter_shift_noop_when_already_shift() {
    use crossterm::event::KeyModifiers;
    let mut ev = key(KeyCode::Enter, KeyModifiers::SHIFT);
    crate::platform::augment_enter_shift(&mut ev);
    assert!(ev.modifiers.contains(KeyModifiers::SHIFT),
        "augment_enter_shift must preserve existing SHIFT modifier");
}

#[cfg(windows)]
#[test]
fn augment_enter_shift_ignores_non_enter() {
    use crossterm::event::KeyModifiers;
    let mut ev = key(KeyCode::Char('a'), KeyModifiers::ALT);
    crate::platform::augment_enter_shift(&mut ev);
    assert!(ev.modifiers.contains(KeyModifiers::ALT),
        "augment_enter_shift must not change non-Enter keys");
    assert!(!ev.modifiers.contains(KeyModifiers::SHIFT),
        "augment_enter_shift must not add SHIFT to non-Enter keys");
}

/// VT fallback encoding for modified Enter still works (encode_key_event path).
#[test]
fn ctrl_shift_enter_vt_encoding_works() {
    let ev = key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\x1b[13;6~",
        "Ctrl+Shift+Enter VT encoding must be CSI 13;6~");
}

#[test]
fn ctrl_alt_enter_vt_encoding_works() {
    let ev = key(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    assert_eq!(bytes, b"\x1b[13;7~",
        "Ctrl+Alt+Enter VT encoding must be CSI 13;7~");
}

#[test]
fn shift_alt_enter_on_non_windows_produces_csi() {
    // On non-Windows, Shift+Alt+Enter should use CSI encoding
    let ev = key(KeyCode::Enter, KeyModifiers::SHIFT | KeyModifiers::ALT);
    let bytes = encode_key_event(&ev).unwrap();
    #[cfg(windows)]
    assert_eq!(bytes, b"\x1b\r", "Shift+Alt+Enter on Windows → ESC+CR");
    #[cfg(not(windows))]
    assert_eq!(bytes, b"\x1b[13;4~", "Shift+Alt+Enter on non-Windows → CSI 13;4~");
}

// ── Issue #134: wrapped directional navigation geometry tests ──

/// Build a two-pane horizontal layout (left | right) for geometry tests.
fn two_pane_h_rects() -> Vec<(Vec<usize>, ratatui::layout::Rect)> {
    use ratatui::layout::Rect;
    vec![
        (vec![0], Rect { x: 0,  y: 0, width: 40, height: 24 }), // left
        (vec![1], Rect { x: 40, y: 0, width: 40, height: 24 }), // right
    ]
}

#[test]
fn issue134_wrap_right_from_rightmost_pane() {
    // From the rightmost pane (index 1), going Right should find no direct
    // neighbor but find a wrap target (the leftmost pane, index 0).
    let rects = two_pane_h_rects();
    let ai = 1; // rightmost pane
    let arect = &rects[ai].1;
    let direct = find_best_pane_in_direction(
        &rects, ai, arect, crate::types::FocusDir::Right, &[], &[],
    );
    assert!(direct.is_none(), "rightmost pane should have no direct Right neighbor");
    let wrap = find_wrap_target(
        &rects, ai, arect, crate::types::FocusDir::Right, &[], &[],
    );
    assert_eq!(wrap, Some(0), "wrap Right from rightmost should reach leftmost (index 0)");
}

#[test]
fn issue134_wrap_left_from_leftmost_pane() {
    let rects = two_pane_h_rects();
    let ai = 0; // leftmost pane
    let arect = &rects[ai].1;
    let direct = find_best_pane_in_direction(
        &rects, ai, arect, crate::types::FocusDir::Left, &[], &[],
    );
    assert!(direct.is_none(), "leftmost pane should have no direct Left neighbor");
    let wrap = find_wrap_target(
        &rects, ai, arect, crate::types::FocusDir::Left, &[], &[],
    );
    assert_eq!(wrap, Some(1), "wrap Left from leftmost should reach rightmost (index 1)");
}

#[test]
fn issue134_direct_neighbor_takes_priority_over_wrap() {
    // From left pane (index 0), going Right should find a direct neighbor (index 1),
    // ensuring wrap is NOT used when a direct neighbor exists.
    let rects = two_pane_h_rects();
    let ai = 0;
    let arect = &rects[ai].1;
    let direct = find_best_pane_in_direction(
        &rects, ai, arect, crate::types::FocusDir::Right, &[], &[],
    );
    assert_eq!(direct, Some(1), "left pane should have direct Right neighbor (right pane)");
}

// ── Issue #141: wrapped nav must not jump columns/rows ──

/// Build a three-pane horizontal layout (left | center | right) for issue #141.
fn three_pane_h_rects() -> Vec<(Vec<usize>, ratatui::layout::Rect)> {
    use ratatui::layout::Rect;
    vec![
        (vec![0], Rect { x: 0,  y: 0, width: 60, height: 30 }), // %1 left
        (vec![1], Rect { x: 61, y: 0, width: 29, height: 30 }), // %2 center
        (vec![2], Rect { x: 91, y: 0, width: 30, height: 30 }), // %3 right
    ]
}

#[test]
fn issue141_wrap_up_single_row_stays_on_self() {
    // Three panes in a single row. From %2 (center), select-pane -U should
    // not jump to %3 or %1. There is no pane above or below, so wrapping
    // should return None (stay on the current pane).
    let rects = three_pane_h_rects();
    let ai = 1; // center pane
    let arect = &rects[ai].1;
    let direct = find_best_pane_in_direction(
        &rects, ai, arect, crate::types::FocusDir::Up, &[], &[],
    );
    assert!(direct.is_none(), "no pane above center in single row");
    let wrap = find_wrap_target(
        &rects, ai, arect, crate::types::FocusDir::Up, &[], &[],
    );
    assert!(wrap.is_none(), "wrap Up in single row must not jump columns (issue #141)");
}

#[test]
fn issue141_wrap_down_single_row_stays_on_self() {
    let rects = three_pane_h_rects();
    let ai = 1;
    let arect = &rects[ai].1;
    let direct = find_best_pane_in_direction(
        &rects, ai, arect, crate::types::FocusDir::Down, &[], &[],
    );
    assert!(direct.is_none(), "no pane below center in single row");
    let wrap = find_wrap_target(
        &rects, ai, arect, crate::types::FocusDir::Down, &[], &[],
    );
    assert!(wrap.is_none(), "wrap Down in single row must not jump columns (issue #141)");
}

/// Build a three-pane vertical layout (top / middle / bottom) for issue #141.
fn three_pane_v_rects() -> Vec<(Vec<usize>, ratatui::layout::Rect)> {
    use ratatui::layout::Rect;
    vec![
        (vec![0], Rect { x: 0, y: 0,  width: 80, height: 10 }), // top
        (vec![1], Rect { x: 0, y: 11, width: 80, height: 10 }), // middle
        (vec![2], Rect { x: 0, y: 22, width: 80, height: 10 }), // bottom
    ]
}

#[test]
fn issue141_wrap_left_single_column_stays_on_self() {
    // Three panes stacked vertically. From middle, select-pane -L should
    // stay on self since there are no panes to the left or right.
    let rects = three_pane_v_rects();
    let ai = 1;
    let arect = &rects[ai].1;
    let direct = find_best_pane_in_direction(
        &rects, ai, arect, crate::types::FocusDir::Left, &[], &[],
    );
    assert!(direct.is_none(), "no pane left of middle in single column");
    let wrap = find_wrap_target(
        &rects, ai, arect, crate::types::FocusDir::Left, &[], &[],
    );
    assert!(wrap.is_none(), "wrap Left in single column must not jump rows (issue #141)");
}

#[test]
fn issue141_wrap_right_single_column_stays_on_self() {
    let rects = three_pane_v_rects();
    let ai = 1;
    let arect = &rects[ai].1;
    let direct = find_best_pane_in_direction(
        &rects, ai, arect, crate::types::FocusDir::Right, &[], &[],
    );
    assert!(direct.is_none(), "no pane right of middle in single column");
    let wrap = find_wrap_target(
        &rects, ai, arect, crate::types::FocusDir::Right, &[], &[],
    );
    assert!(wrap.is_none(), "wrap Right in single column must not jump rows (issue #141)");
}

#[test]
fn issue141_wrap_up_still_works_with_column_overlap() {
    // Two panes stacked vertically. Wrap Up from bottom should still reach top
    // because they overlap on the perpendicular (x) axis.
    use ratatui::layout::Rect;
    let rects: Vec<(Vec<usize>, Rect)> = vec![
        (vec![0], Rect { x: 0, y: 0,  width: 80, height: 12 }),
        (vec![1], Rect { x: 0, y: 13, width: 80, height: 12 }),
    ];
    let ai = 0; // top pane
    let arect = &rects[ai].1;
    let wrap = find_wrap_target(
        &rects, ai, arect, crate::types::FocusDir::Up, &[], &[],
    );
    assert_eq!(wrap, Some(1), "wrap Up from top should reach bottom when they share a column");
}
