#[cfg(windows)]
use super::*;

#[cfg(windows)]
#[test]
fn ime_detection_ascii_only() {
    // Pure ASCII text should NOT be detected as IME input
    assert!(!paste_buffer_has_non_ascii("abc"));
    assert!(!paste_buffer_has_non_ascii("hello world"));
    assert!(!paste_buffer_has_non_ascii("12345"));
    assert!(!paste_buffer_has_non_ascii(""));
}

#[cfg(windows)]
#[test]
fn ime_detection_japanese() {
    // Japanese IME input should be detected as non-ASCII
    assert!(paste_buffer_has_non_ascii("日本語"));
    assert!(paste_buffer_has_non_ascii("にほんご"));
    assert!(paste_buffer_has_non_ascii("abc日本語"));
}

#[cfg(windows)]
#[test]
fn ime_detection_chinese() {
    assert!(paste_buffer_has_non_ascii("中文"));
    assert!(paste_buffer_has_non_ascii("你好世界"));
}

#[cfg(windows)]
#[test]
fn ime_detection_korean() {
    assert!(paste_buffer_has_non_ascii("한국어"));
}

#[cfg(windows)]
#[test]
fn ime_detection_mixed() {
    // Mixed ASCII + CJK should be detected as non-ASCII
    assert!(paste_buffer_has_non_ascii("hello世界"));
    assert!(paste_buffer_has_non_ascii("a日b"));
}

#[cfg(windows)]
#[test]
fn flush_paste_pend_ascii_sends_as_paste() {
    // ASCII buffer with ≥3 chars should send as send-paste (paste detection intact)
    let mut buf = String::from("abcdef");
    let mut start: Option<std::time::Instant> = Some(std::time::Instant::now());
    let mut stage2 = true;
    let mut cmds: Vec<String> = Vec::new();
    flush_paste_pend_as_text(&mut buf, &mut start, &mut stage2, &mut cmds);
    assert_eq!(cmds.len(), 1);
    assert!(cmds[0].starts_with("send-paste "));
}

#[cfg(windows)]
#[test]
fn flush_paste_pend_cjk_sends_as_text() {
    // Non-ASCII buffer should NEVER send as send-paste, even with ≥3 chars.
    // This is the core fix for issue #91.
    let mut buf = String::from("日本語テスト");
    let mut start: Option<std::time::Instant> = Some(std::time::Instant::now());
    let mut stage2 = false;
    let mut cmds: Vec<String> = Vec::new();
    flush_paste_pend_as_text(&mut buf, &mut start, &mut stage2, &mut cmds);
    // Each character should be sent as individual send-text
    assert!(cmds.len() > 1, "CJK should be sent as individual send-text commands");
    for cmd in &cmds {
        assert!(cmd.starts_with("send-text "), "CJK char should be send-text, got: {}", cmd);
    }
}

#[cfg(windows)]
#[test]
fn flush_paste_pend_short_ascii_sends_as_text() {
    // <3 ASCII chars should be sent as individual keystrokes
    let mut buf = String::from("ab");
    let mut start: Option<std::time::Instant> = Some(std::time::Instant::now());
    let mut stage2 = false;
    let mut cmds: Vec<String> = Vec::new();
    flush_paste_pend_as_text(&mut buf, &mut start, &mut stage2, &mut cmds);
    assert_eq!(cmds.len(), 2);
    assert!(cmds[0].starts_with("send-text "));
    assert!(cmds[1].starts_with("send-text "));
}

// ── Issue #164: status-format[] must parse inline styles end-to-end ──

/// Verify that status_format strings from JSON deserialization flow through
/// parse_inline_styles correctly and produce styled (not literal) output.
#[cfg(windows)]
#[test]
fn status_format_inline_styles_end_to_end() {
    use ratatui::style::{Color, Style};
    use unicode_width::UnicodeWidthStr;

    // Simulate what the server sends: status_format with style directives
    let status_format: Vec<String> = vec![
        "#[align=left]Custom Line 1".to_string(),
        "#[fg=red]Custom Line 2".to_string(),
    ];

    let sb_base = Style::default().fg(Color::White).bg(Color::Black);

    // Test line 0 (status_format[0]) rendering path
    {
        let use_status_format_0 = !status_format.is_empty() && !status_format[0].is_empty();
        assert!(use_status_format_0, "status_format[0] should be detected as set");

        let fmt0_spans = crate::style::parse_inline_styles(&status_format[0], sb_base);
        assert_eq!(fmt0_spans.len(), 1, "Line 0 should produce 1 span, got {}", fmt0_spans.len());
        assert_eq!(fmt0_spans[0].content.as_ref(), "Custom Line 1",
            "Line 0 should NOT contain literal #[align=left], got: {:?}", fmt0_spans[0].content);
        // align=left is silently consumed, style stays at base
        assert_eq!(fmt0_spans[0].style.fg, Some(Color::White));
        assert_eq!(fmt0_spans[0].style.bg, Some(Color::Black));
    }

    // Test line 1 (status_format[1]) rendering path
    {
        let text = &status_format[1];
        let parsed_spans = crate::style::parse_inline_styles(text, sb_base);
        assert_eq!(parsed_spans.len(), 1, "Line 1 should produce 1 span, got {}", parsed_spans.len());
        assert_eq!(parsed_spans[0].content.as_ref(), "Custom Line 2",
            "Line 1 should NOT contain literal #[fg=red], got: {:?}", parsed_spans[0].content);
        assert_eq!(parsed_spans[0].style.fg, Some(Color::Red),
            "Line 1 fg should be Red (parsed from #[fg=red]), got {:?}", parsed_spans[0].style.fg);
        assert_eq!(parsed_spans[0].style.bg, Some(Color::Black),
            "Line 1 bg should remain Black from base, got {:?}", parsed_spans[0].style.bg);

        // Also verify padding uses visible width, not raw text length
        let visible_w: usize = parsed_spans.iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        assert_eq!(visible_w, 13, "Visible width should be 13 (Custom Line 2), got {}", visible_w);
        // The raw status_format[1] is 23 chars (#[fg=red]Custom Line 2)
        // but visible is only 13 chars — padding must use 13, not 23
        assert!(text.len() > visible_w,
            "Raw text ({}) should be longer than visible width ({}) due to style directives",
            text.len(), visible_w);
    }
}

/// Verify that the JSON server payload correctly round-trips status_format
/// through serde deserialization without mangling style directives.
#[cfg(windows)]
#[test]
fn status_format_json_roundtrip_preserves_styles() {
    // Simulate the JSON fragment the server sends
    let json_fragment = r##"{"status_format":["","#[fg=red]Hello","#[fg=green,bg=blue]World"]}"##;

    #[derive(serde::Deserialize)]
    struct Partial {
        #[serde(default)]
        status_format: Vec<String>,
    }
    let parsed: Partial = serde_json::from_str(json_fragment).unwrap();
    assert_eq!(parsed.status_format.len(), 3);
    assert_eq!(parsed.status_format[0], "");
    assert_eq!(parsed.status_format[1], "#[fg=red]Hello",
        "Style directives must survive JSON roundtrip");
    assert_eq!(parsed.status_format[2], "#[fg=green,bg=blue]World",
        "Multi-directive styles must survive JSON roundtrip");

    // Now verify parse_inline_styles produces correct output from deserialized data
    use ratatui::style::{Color, Style};
    let base = Style::default();

    let spans1 = crate::style::parse_inline_styles(&parsed.status_format[1], base);
    assert_eq!(spans1.len(), 1);
    assert_eq!(spans1[0].content.as_ref(), "Hello");
    assert_eq!(spans1[0].style.fg, Some(Color::Red));

    let spans2 = crate::style::parse_inline_styles(&parsed.status_format[2], base);
    assert_eq!(spans2.len(), 1);
    assert_eq!(spans2[0].content.as_ref(), "World");
    assert_eq!(spans2[0].style.fg, Some(Color::Green));
    assert_eq!(spans2[0].style.bg, Some(Color::Blue));
}

// ── Issue #211: pwsh-mouse-selection helpers ──

/// Helper to create a CellRunJson for tests.
#[cfg(windows)]
fn make_run(text: &str, width: u16) -> crate::layout::CellRunJson {
    crate::layout::CellRunJson {
        text: text.to_string(),
        fg: String::new(),
        bg: String::new(),
        flags: 0,
        width,
    }
}

/// Helper to create a RowRunsJson for tests.
#[cfg(windows)]
fn make_row(runs: Vec<crate::layout::CellRunJson>) -> crate::layout::RowRunsJson {
    crate::layout::RowRunsJson { runs }
}

#[cfg(windows)]
#[test]
fn normalize_selection_reading_order() {
    // Start before end: no swap
    let (r0, c0, r1, c1) = normalize_selection((2, 1), (5, 3), false);
    assert_eq!((r0, c0, r1, c1), (1, 2, 3, 5));

    // Start after end: swapped
    let (r0, c0, r1, c1) = normalize_selection((5, 3), (2, 1), false);
    assert_eq!((r0, c0, r1, c1), (1, 2, 3, 5));
}

#[cfg(windows)]
#[test]
fn normalize_selection_block_mode() {
    // Block mode: min/max of each axis independently
    let (r0, c0, r1, c1) = normalize_selection((8, 5), (3, 2), true);
    assert_eq!((r0, c0, r1, c1), (2, 3, 5, 8));
}

#[cfg(windows)]
#[test]
fn row_chars_basic() {
    let runs = vec![
        make_run("AB", 2),
        make_run("C", 1),
        make_run(" ", 3),
    ];
    let chars = row_chars(&runs, 6);
    assert_eq!(chars, vec!['A', 'B', 'C', ' ', ' ', ' ']);
}

#[cfg(windows)]
#[test]
fn row_chars_width_clamp() {
    let runs = vec![make_run("ABCDE", 5)];
    let chars = row_chars(&runs, 3);
    assert_eq!(chars, vec!['A', 'B', 'C']);
}

#[cfg(windows)]
#[test]
fn is_word_char_basics() {
    assert!(is_word_char('a'));
    assert!(is_word_char('Z'));
    assert!(is_word_char('0'));
    assert!(is_word_char('_'));
    assert!(!is_word_char(' '));
    assert!(!is_word_char('-'));
    assert!(!is_word_char('.'));
}

#[cfg(windows)]
#[test]
fn char_at_col_basics() {
    let runs = vec![
        make_run("He", 2),
        make_run("llo", 3),
    ];
    assert_eq!(char_at_col(&runs, 0), 'H');
    assert_eq!(char_at_col(&runs, 1), 'e');
    assert_eq!(char_at_col(&runs, 2), 'l');
    assert_eq!(char_at_col(&runs, 3), 'l');
    assert_eq!(char_at_col(&runs, 4), 'o');
    // Out of range returns space
    assert_eq!(char_at_col(&runs, 10), ' ');
}

#[cfg(windows)]
#[test]
fn extract_selection_text_block_mode() {
    use ratatui::layout::Rect as Rect;
    // A single leaf pane 10 cols wide, 3 rows
    let layout = crate::layout::LayoutJson::Leaf {
        id: 0,
        rows: 3,
        cols: 10,
        cursor_row: 0,
        cursor_col: 0,
        alternate_screen: false,
        hide_cursor: false,
        cursor_shape: 0,
        active: true,
        copy_mode: false,
        scroll_offset: 0,
        sel_start_row: None,
        sel_start_col: None,
        sel_end_row: None,
        sel_end_col: None,
        sel_mode: None,
        copy_cursor_row: None,
        copy_cursor_col: None,
        content: Vec::new(),
        rows_v2: vec![
            make_row(vec![make_run("0123456789", 10)]),
            make_row(vec![make_run("abcdefghij", 10)]),
            make_row(vec![make_run("ABCDEFGHIJ", 10)]),
        ],
        title: None,
    };

    // Block select cols 2..5, rows 0..2
    let text = extract_selection_text(&layout, 10, 3, (2, 0), (5, 2), true);
    assert_eq!(text, "2345\ncdef\nCDEF");

    // Non-block (reading order) same coordinates should give full intermediate rows
    let text_normal = extract_selection_text(&layout, 10, 3, (2, 0), (5, 2), false);
    assert_eq!(text_normal, "23456789\nabcdefghij\nABCDEF");
}

#[cfg(windows)]
#[test]
fn word_bounds_at_finds_word() {
    let layout = crate::layout::LayoutJson::Leaf {
        id: 0,
        rows: 1,
        cols: 20,
        cursor_row: 0,
        cursor_col: 0,
        alternate_screen: false,
        hide_cursor: false,
        cursor_shape: 0,
        active: true,
        copy_mode: false,
        scroll_offset: 0,
        sel_start_row: None,
        sel_start_col: None,
        sel_end_row: None,
        sel_end_col: None,
        sel_mode: None,
        copy_cursor_row: None,
        copy_cursor_col: None,
        content: Vec::new(),
        rows_v2: vec![
            make_row(vec![make_run("hello world_test   ", 19), make_run(" ", 1)]),
        ],
        title: None,
    };

    let pane_rect = ratatui::layout::Rect { x: 0, y: 0, width: 20, height: 1 };

    // Click on 'h' (col 0): word is "hello" -> (0, 4)
    assert_eq!(word_bounds_at(&layout, 20, 1, pane_rect, 0, 0), Some((0, 4)));
    // Click on 'l' (col 3): still "hello" -> (0, 4)
    assert_eq!(word_bounds_at(&layout, 20, 1, pane_rect, 3, 0), Some((0, 4)));
    // Click on space (col 5): no word
    assert_eq!(word_bounds_at(&layout, 20, 1, pane_rect, 5, 0), None);
    // Click on 'w' (col 6): "world_test" -> (6, 15)
    assert_eq!(word_bounds_at(&layout, 20, 1, pane_rect, 6, 0), Some((6, 15)));
    // Click on '_' (col 11): still "world_test" since _ is a word char -> (6, 15)
    assert_eq!(word_bounds_at(&layout, 20, 1, pane_rect, 11, 0), Some((6, 15)));
}

#[cfg(windows)]
#[test]
fn pwsh_mouse_selection_option_default_off() {
    let state = crate::types::AppState::new("test-session".to_string());
    assert!(!state.pwsh_mouse_selection, "pwsh_mouse_selection should default to off");
}
