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
fn make_leaf(id: usize, rows: &[&str]) -> crate::layout::LayoutJson {
    let cols = rows.first().map(|row| row.chars().count()).unwrap_or(0) as u16;
    crate::layout::LayoutJson::Leaf {
        id,
        rows: rows.len() as u16,
        cols,
        cursor_row: 0,
        cursor_col: 0,
        alternate_screen: false,
        hide_cursor: false,
        cursor_shape: 0,
        active: id == 0,
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
        rows_v2: rows
            .iter()
            .map(|row| make_row(vec![make_run(row, cols)]))
            .collect(),
        title: None,
    }
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
    let text = extract_selection_text(&layout, 10, 3, (2, 0), (5, 2), true, None);
    assert_eq!(text, "2345\ncdef\nCDEF");

    // Non-block (reading order) same coordinates should give full intermediate rows
    let text_normal = extract_selection_text(&layout, 10, 3, (2, 0), (5, 2), false, None);
    assert_eq!(text_normal, "23456789\nabcdefghij\nABCDEF");
}

#[cfg(windows)]
#[test]
fn extract_selection_text_clips_reading_order_to_origin_pane() {
    let layout = crate::layout::LayoutJson::Split {
        kind: "Horizontal".to_string(),
        sizes: vec![50, 50],
        children: vec![
            make_leaf(0, &["abcde", "fghij", "klmno"]),
            make_leaf(1, &["ABCDE", "FGHIJ", "KLMNO"]),
        ],
    };
    let pane_clip = ratatui::layout::Rect { x: 0, y: 0, width: 5, height: 3 };

    let text = extract_selection_text(&layout, 11, 3, (1, 0), (3, 2), false, Some(pane_clip));

    assert_eq!(text, "bcde\nfghij\nklmn");
}

#[cfg(windows)]
#[test]
fn extract_selection_text_clips_block_mode_to_origin_pane() {
    let layout = crate::layout::LayoutJson::Split {
        kind: "Horizontal".to_string(),
        sizes: vec![50, 50],
        children: vec![
            make_leaf(0, &["abcde", "fghij", "klmno"]),
            make_leaf(1, &["ABCDE", "FGHIJ", "KLMNO"]),
        ],
    };
    let pane_clip = ratatui::layout::Rect { x: 0, y: 0, width: 5, height: 3 };

    let text = extract_selection_text(&layout, 11, 3, (1, 0), (8, 2), true, Some(pane_clip));

    assert_eq!(text, "bcde\nghij\nlmno");
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

// ── Issue #290: paste must not leak past the command prompt ─────────────
// route_paste_to_overlay is the helper that the Event::Paste branch in the
// client loop delegates to.  When an overlay returns true, the loop skips
// the `send-paste` forwarding, so paste content cannot reach the shell.

#[test]
fn paste_into_command_prompt_inserts_and_advances_cursor() {
    let mut command_buf = String::new();
    let mut command_cursor = 0;
    let mut rename_buf = String::new();
    let mut pane_title_buf = String::new();
    let mut window_idx_buf = String::new();
    let consumed = super::route_paste_to_overlay(
        "hello",
        true, &mut command_buf, &mut command_cursor,
        false, &mut rename_buf,
        false, &mut pane_title_buf,
        false, &mut window_idx_buf,
    );
    assert!(consumed, "command_input overlay must consume paste");
    assert_eq!(command_buf, "hello");
    assert_eq!(command_cursor, 5);
}

#[test]
fn paste_into_command_prompt_inserts_at_cursor_position() {
    // User typed "abdef", moved cursor between b and d, then pastes "c".
    let mut command_buf = String::from("abdef");
    let mut command_cursor = 2;
    let mut rename_buf = String::new();
    let mut pane_title_buf = String::new();
    let mut window_idx_buf = String::new();
    let consumed = super::route_paste_to_overlay(
        "c",
        true, &mut command_buf, &mut command_cursor,
        false, &mut rename_buf,
        false, &mut pane_title_buf,
        false, &mut window_idx_buf,
    );
    assert!(consumed);
    assert_eq!(command_buf, "abcdef");
    assert_eq!(command_cursor, 3);
}

#[test]
fn paste_with_no_overlay_active_is_not_consumed() {
    // Caller must forward via send-paste when this returns false.
    let mut command_buf = String::new();
    let mut command_cursor = 0;
    let mut rename_buf = String::new();
    let mut pane_title_buf = String::new();
    let mut window_idx_buf = String::new();
    let consumed = super::route_paste_to_overlay(
        "hello",
        false, &mut command_buf, &mut command_cursor,
        false, &mut rename_buf,
        false, &mut pane_title_buf,
        false, &mut window_idx_buf,
    );
    assert!(!consumed);
    assert!(command_buf.is_empty());
    assert!(rename_buf.is_empty());
    assert!(pane_title_buf.is_empty());
    assert!(window_idx_buf.is_empty());
}

#[test]
fn paste_into_rename_prompt_appends() {
    let mut command_buf = String::new();
    let mut command_cursor = 0;
    let mut rename_buf = String::from("foo");
    let mut pane_title_buf = String::new();
    let mut window_idx_buf = String::new();
    let consumed = super::route_paste_to_overlay(
        "bar",
        false, &mut command_buf, &mut command_cursor,
        true, &mut rename_buf,
        false, &mut pane_title_buf,
        false, &mut window_idx_buf,
    );
    assert!(consumed);
    assert_eq!(rename_buf, "foobar");
}

#[test]
fn paste_into_pane_title_appends() {
    let mut command_buf = String::new();
    let mut command_cursor = 0;
    let mut rename_buf = String::new();
    let mut pane_title_buf = String::from("title");
    let mut window_idx_buf = String::new();
    let consumed = super::route_paste_to_overlay(
        "-suffix",
        false, &mut command_buf, &mut command_cursor,
        false, &mut rename_buf,
        true, &mut pane_title_buf,
        false, &mut window_idx_buf,
    );
    assert!(consumed);
    assert_eq!(pane_title_buf, "title-suffix");
}

#[test]
fn paste_into_window_idx_prompt_keeps_only_digits() {
    let mut command_buf = String::new();
    let mut command_cursor = 0;
    let mut rename_buf = String::new();
    let mut pane_title_buf = String::new();
    let mut window_idx_buf = String::new();
    let consumed = super::route_paste_to_overlay(
        "1a2b3",
        false, &mut command_buf, &mut command_cursor,
        false, &mut rename_buf,
        false, &mut pane_title_buf,
        true, &mut window_idx_buf,
    );
    assert!(consumed);
    assert_eq!(window_idx_buf, "123");
}

#[test]
fn paste_command_prompt_takes_precedence_over_other_overlays() {
    // If multiple overlay flags are accidentally true, command_input wins
    // (matches the if/else-if order in the helper).
    let mut command_buf = String::new();
    let mut command_cursor = 0;
    let mut rename_buf = String::new();
    let mut pane_title_buf = String::new();
    let mut window_idx_buf = String::new();
    let consumed = super::route_paste_to_overlay(
        "x",
        true, &mut command_buf, &mut command_cursor,
        true, &mut rename_buf,
        true, &mut pane_title_buf,
        true, &mut window_idx_buf,
    );
    assert!(consumed);
    assert_eq!(command_buf, "x");
    assert!(rename_buf.is_empty());
    assert!(pane_title_buf.is_empty());
    assert!(window_idx_buf.is_empty());
}
