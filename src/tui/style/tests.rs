#[allow(unused_imports)]

use ratatui::prelude::*;
use ratatui::style::{Style, Color, Modifier};

use crate::debug_log::style_log;

use super::*;

#[cfg(test)]

/// Issue #164: parse_inline_styles must parse #[fg=red] and apply the style,
/// NOT render it as literal text.
#[test]
fn parse_inline_styles_fg_red() {
    let base = Style::default().fg(Color::White).bg(Color::Black);
    let spans = parse_inline_styles("#[fg=red]Custom Line 2", base);

    // Should produce exactly one span with the visible text (no style directive text)
    assert_eq!(spans.len(), 1, "Expected 1 span, got {:?}", spans);
    assert_eq!(spans[0].content.as_ref(), "Custom Line 2");
    // The style should have fg=Red applied
    assert_eq!(spans[0].style.fg, Some(Color::Red),
        "fg should be Red, got {:?}", spans[0].style.fg);
    // bg should remain from base
    assert_eq!(spans[0].style.bg, Some(Color::Black),
        "bg should remain Black from base, got {:?}", spans[0].style.bg);
}

/// Issue #164: #[align=left] should be consumed (not rendered as literal text)
#[test]
fn parse_inline_styles_align_left() {
    let base = Style::default();
    let spans = parse_inline_styles("#[align=left]Custom Line 1", base);

    assert_eq!(spans.len(), 1, "Expected 1 span, got {:?}", spans);
    assert_eq!(spans[0].content.as_ref(), "Custom Line 1");
    // No literal "#[align=left]" text should appear
}

/// Issue #164: Multiple style directives in one format string
#[test]
fn parse_inline_styles_multiple_directives() {
    let base = Style::default();
    let spans = parse_inline_styles("#[fg=red]Hello #[fg=green]World", base);

    assert_eq!(spans.len(), 2, "Expected 2 spans, got {:?}", spans);
    assert_eq!(spans[0].content.as_ref(), "Hello ");
    assert_eq!(spans[0].style.fg, Some(Color::Red));
    assert_eq!(spans[1].content.as_ref(), "World");
    assert_eq!(spans[1].style.fg, Some(Color::Green));
}

/// Issue #164: fg+bg combined in one directive
#[test]
fn parse_inline_styles_fg_and_bg() {
    let base = Style::default();
    let spans = parse_inline_styles("#[fg=yellow,bg=blue]Styled", base);

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].content.as_ref(), "Styled");
    assert_eq!(spans[0].style.fg, Some(Color::Yellow));
    assert_eq!(spans[0].style.bg, Some(Color::Blue));
}

/// Issue #164: Plain text without directives passes through unchanged
#[test]
fn parse_inline_styles_plain_text() {
    let base = Style::default().fg(Color::White);
    let spans = parse_inline_styles("No styles here", base);

    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].content.as_ref(), "No styles here");
    assert_eq!(spans[0].style.fg, Some(Color::White));
}

/// Issue #164: Empty string produces no spans
#[test]
fn parse_inline_styles_empty() {
    let base = Style::default();
    let spans = parse_inline_styles("", base);
    assert!(spans.is_empty());
}

/// Issue #182: bg=default should map to Color::Reset (terminal default),
/// not None (which causes fallback to hardcoded green).
#[test]
fn parse_tmux_color_default_returns_reset() {
    let c = parse_tmux_color("default");
    assert_eq!(c, Some(Color::Reset),
        "parse_tmux_color(\"default\") should return Some(Color::Reset), got {:?}", c);
}

/// Issue #182: parse_tmux_style_components should propagate bg=default as Some(Color::Reset)
#[test]
fn parse_tmux_style_components_bg_default() {
    let (fg, bg, bold) = parse_tmux_style_components("fg=white,bg=default");
    assert_eq!(fg, Some(Color::White));
    assert_eq!(bg, Some(Color::Reset),
        "bg=default should yield Some(Color::Reset), got {:?}", bg);
    assert!(!bold);
}

/// Issue #182: map_color("default") should return Color::Reset
#[test]
fn map_color_default_is_reset() {
    assert_eq!(map_color("default"), Color::Reset);
    assert_eq!(map_color("terminal"), Color::Reset);
}

/// Empty color string should remain None (not specified)
#[test]
fn parse_tmux_color_empty_returns_none() {
    assert_eq!(parse_tmux_color(""), None);
}

#[cfg(test)]
#[path = "../../../tests-rs/test_issue185_layout_directives.rs"]
mod test_issue185_layout_directives;