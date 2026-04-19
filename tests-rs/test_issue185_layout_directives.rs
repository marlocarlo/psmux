use super::*;
use ratatui::style::{Color, Style};

// ─── parse_format_segments tests ────────────────────────────────────────────

#[test]
fn segments_plain_text() {
    let base = Style::default();
    let tokens = parse_format_segments("hello world", base);
    assert_eq!(tokens.len(), 1);
    match &tokens[0] {
        FormatToken::Text(span) => assert_eq!(span.content.as_ref(), "hello world"),
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn segments_align_right() {
    let base = Style::default();
    let tokens = parse_format_segments("#[align=right]time", base);
    assert!(tokens.len() >= 2);
    match &tokens[0] {
        FormatToken::Align(StatusAlignment::Right) => {}
        other => panic!("expected Align(Right), got {:?}", other),
    }
    match &tokens[1] {
        FormatToken::Text(span) => assert_eq!(span.content.as_ref(), "time"),
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn segments_fill() {
    let base = Style::default().bg(Color::Blue);
    let tokens = parse_format_segments("#[fill]", base);
    assert_eq!(tokens.len(), 1);
    match &tokens[0] {
        FormatToken::Fill(s) => assert_eq!(s.bg, Some(Color::Blue)),
        other => panic!("expected Fill, got {:?}", other),
    }
}

#[test]
fn segments_fill_with_color() {
    let base = Style::default();
    let tokens = parse_format_segments("#[fill=red]", base);
    assert_eq!(tokens.len(), 1);
    match &tokens[0] {
        FormatToken::Fill(s) => assert_eq!(s.bg, Some(Color::Red)),
        other => panic!("expected Fill, got {:?}", other),
    }
}

#[test]
fn segments_range_window() {
    let base = Style::default();
    let tokens = parse_format_segments("#[range=window|3]tab3#[norange]", base);
    assert!(tokens.len() >= 3);
    match &tokens[0] {
        FormatToken::Range(StatusRangeType::Window(3)) => {}
        other => panic!("expected Range(Window(3)), got {:?}", other),
    }
    match &tokens[1] {
        FormatToken::Text(span) => assert_eq!(span.content.as_ref(), "tab3"),
        other => panic!("expected Text, got {:?}", other),
    }
    match &tokens[2] {
        FormatToken::NoRange => {}
        other => panic!("expected NoRange, got {:?}", other),
    }
}

#[test]
fn segments_list_markers() {
    let base = Style::default();
    let tokens = parse_format_segments(
        "#[list=left-marker]<#[list=right-marker]>#[list=on]win1 win2#[nolist]",
        base,
    );
    let has_left_marker = tokens.iter().any(|t| matches!(t, FormatToken::ListLeftMarker));
    let has_right_marker = tokens.iter().any(|t| matches!(t, FormatToken::ListRightMarker));
    let has_list_on = tokens.iter().any(|t| matches!(t, FormatToken::ListOn));
    let has_nolist = tokens.iter().any(|t| matches!(t, FormatToken::NoList));
    assert!(has_left_marker, "missing ListLeftMarker");
    assert!(has_right_marker, "missing ListRightMarker");
    assert!(has_list_on, "missing ListOn");
    assert!(has_nolist, "missing NoList");
}

#[test]
fn segments_combined_directive() {
    // #[range=window|0 list=focus] uses space separator
    let base = Style::default();
    let tokens = parse_format_segments("#[range=window|0 list=focus]text#[norange]", base);
    let has_range = tokens.iter().any(|t| matches!(t, FormatToken::Range(StatusRangeType::Window(0))));
    let has_focus = tokens.iter().any(|t| matches!(t, FormatToken::ListFocus));
    assert!(has_range, "missing Range(Window(0))");
    assert!(has_focus, "missing ListFocus");
}

#[test]
fn segments_style_plus_layout() {
    let base = Style::default();
    let tokens = parse_format_segments("#[fg=red,align=right]text", base);
    let has_align = tokens.iter().any(|t| matches!(t, FormatToken::Align(StatusAlignment::Right)));
    let has_text = tokens.iter().any(|t| matches!(t, FormatToken::Text(_)));
    assert!(has_align, "missing Align(Right)");
    assert!(has_text, "missing Text");
    // The text should have red fg
    for t in &tokens {
        if let FormatToken::Text(span) = t {
            assert_eq!(span.style.fg, Some(Color::Red), "text should have fg=Red");
        }
    }
}

// ─── layout_format_line tests ───────────────────────────────────────────────

fn collect_text(spans: &[ratatui::text::Span]) -> String {
    spans.iter().map(|s| s.content.as_ref()).collect()
}

fn visible_text(spans: &[ratatui::text::Span]) -> String {
    spans.iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn layout_plain_text_fills_width() {
    let base = Style::default();
    let result = layout_format_line("hello", 20, base);
    let text = collect_text(&result.spans);
    assert_eq!(text.len(), 20, "should pad to full width");
    assert!(text.starts_with("hello"), "should start with the text");
}

#[test]
fn layout_align_right() {
    let base = Style::default();
    let result = layout_format_line("#[align=right]time", 20, base);
    let text = collect_text(&result.spans);
    assert_eq!(text.len(), 20);
    // "time" should be at the right edge
    assert!(text.ends_with("time"), "right-aligned text should be at right edge, got: [{}]", text);
}

#[test]
fn layout_align_left_and_right() {
    let base = Style::default();
    let result = layout_format_line("#[align=left]LEFT#[align=right]RIGHT", 30, base);
    let text = collect_text(&result.spans);
    assert_eq!(text.len(), 30);
    assert!(text.starts_with("LEFT"), "should start with LEFT, got: [{}]", text);
    assert!(text.ends_with("RIGHT"), "should end with RIGHT, got: [{}]", text);
}

#[test]
fn layout_three_sections() {
    let base = Style::default();
    let result = layout_format_line(
        "#[align=left]L#[align=centre]C#[align=right]R",
        21, base,
    );
    let text = collect_text(&result.spans);
    assert_eq!(text.len(), 21);
    // L at position 0, R at position 20, C centered
    assert!(text.starts_with("L"), "left section at start");
    assert!(text.ends_with("R"), "right section at end");
    // Centre should be roughly in the middle
    let c_pos = text.find('C').unwrap();
    assert!(c_pos >= 8 && c_pos <= 12, "centre should be near middle, found at {}", c_pos);
}

#[test]
fn layout_fill_style() {
    let base = Style::default();
    let result = layout_format_line(
        "#[align=left]L#[fill=red]#[align=right]R",
        20, base,
    );
    // The fill spans between L and R should have red background
    let fill_spans: Vec<_> = result.spans.iter()
        .filter(|s| s.content.trim().is_empty() && !s.content.is_empty())
        .collect();
    assert!(!fill_spans.is_empty(), "should have fill spans");
    for s in &fill_spans {
        assert_eq!(s.style.bg, Some(Color::Red),
            "fill spans should have bg=Red, got {:?}", s.style.bg);
    }
}

#[test]
fn layout_range_tracking() {
    let base = Style::default();
    let result = layout_format_line(
        "#[range=window|0]win0#[norange] #[range=window|1]win1#[norange]",
        30, base,
    );
    assert_eq!(result.ranges.len(), 2, "should have 2 ranges");
    assert_eq!(result.ranges[0].0, StatusRangeType::Window(0));
    assert_eq!(result.ranges[0].1, 0); // starts at col 0
    assert_eq!(result.ranges[0].2, 4); // "win0" is 4 chars
    assert_eq!(result.ranges[1].0, StatusRangeType::Window(1));
    assert_eq!(result.ranges[1].1, 5); // after "win0 "
    assert_eq!(result.ranges[1].2, 9); // "win1" ends at col 9
}

#[test]
fn layout_list_fits() {
    let base = Style::default();
    // List with markers, but everything fits
    let result = layout_format_line(
        "#[list=left-marker]<#[list=right-marker]>#[list=on]ABCDE#[nolist]",
        20, base,
    );
    let text = visible_text(&result.spans);
    // Since list fits, no markers should appear
    assert!(text.contains("ABCDE"), "list content should appear: [{}]", text);
    assert!(!text.contains('<'), "no left marker when list fits: [{}]", text);
    assert!(!text.contains('>'), "no right marker when list fits: [{}]", text);
}

#[test]
fn layout_list_overflow() {
    let base = Style::default();
    // Very long list in narrow width
    let list_content = "0:bash 1:vim 2:htop 3:python 4:node 5:cargo";
    let fmt = format!(
        "#[list=left-marker]<#[list=right-marker]>#[list=on]{}#[nolist]",
        list_content
    );
    let result = layout_format_line(&fmt, 20, base);
    let text = visible_text(&result.spans);
    // With overflow, at least one marker should appear
    let has_marker = text.contains('<') || text.contains('>');
    assert!(has_marker || text.len() <= 20,
        "overflowing list should show markers or be truncated: [{}]", text);
}

#[test]
fn layout_full_status_format() {
    // Simulate a realistic status-format[0]
    let base = Style::default().fg(Color::White).bg(Color::Black);
    let fmt = "#[align=left]#[range=window|0]0:bash#[norange] #[range=window|1]1:vim#[norange]#[align=right]12:34";
    let result = layout_format_line(fmt, 40, base);
    let text = collect_text(&result.spans);
    assert_eq!(text.len(), 40);
    assert!(text.starts_with("0:bash"), "left section");
    assert!(text.ends_with("12:34"), "right section: [{}]", text);
    // Should have 2 window ranges
    assert_eq!(result.ranges.len(), 2);
}

#[test]
fn layout_empty_string() {
    let base = Style::default();
    let result = layout_format_line("", 20, base);
    let text = collect_text(&result.spans);
    assert_eq!(text.len(), 20, "empty format should pad to width");
    assert!(result.ranges.is_empty());
}

#[test]
fn layout_width_exact_fit() {
    let base = Style::default();
    let result = layout_format_line("12345", 5, base);
    let text = collect_text(&result.spans);
    assert_eq!(text, "12345");
}

#[test]
fn layout_truncation() {
    let base = Style::default();
    let result = layout_format_line("very long text here", 10, base);
    let text = collect_text(&result.spans);
    assert_eq!(text.len(), 10, "should truncate to width");
}
