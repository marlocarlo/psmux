use super::*;

/// Extract spans from a column range within a list of spans.
///
/// Returns spans whose visible content falls within `[col_start, col_start + max_width)`.
pub(crate) fn extract_span_range(spans: &[Span<'static>], col_start: usize, max_width: usize) -> Vec<Span<'static>> {
    use unicode_width::UnicodeWidthChar;
    let mut result = Vec::new();
    let mut col = 0usize;
    let mut remaining = max_width;

    for span in spans {
        let sw = spans_visual_width(&[span.clone()]);
        if col + sw <= col_start {
            col += sw;
            continue;
        }
        // This span overlaps with our range
        let mut text = String::new();
        for ch in span.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col < col_start {
                col += cw;
                continue;
            }
            if cw > remaining { break; }
            remaining -= cw;
            col += cw;
            text.push(ch);
        }
        if !text.is_empty() {
            result.push(Span::styled(text, span.style));
        }
        if remaining == 0 { break; }
    }
    result
}
