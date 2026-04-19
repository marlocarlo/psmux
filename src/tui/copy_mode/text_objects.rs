use super::*;

// ── Text Object Selection ──────────────────────────────────────────────

/// Select "inner word" (iw) — word under cursor without surrounding whitespace.
/// Uses `char_class` for word boundary detection (same as `w`/`b`/`e` motions).
pub fn select_inner_word(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let seps = app.word_separators.clone();
    let (text, _cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let col = c as usize;
    if col >= bytes.len() { return; }
    let cls = char_class(bytes[col], &seps);
    // Find start of word
    let mut start = col;
    while start > 0 && char_class(bytes[start - 1], &seps) == cls { start -= 1; }
    // Find end of word
    let mut end = col;
    while end + 1 < bytes.len() && char_class(bytes[end + 1], &seps) == cls { end += 1; }
    app.copy_anchor = Some((r, start as u16));
    app.copy_anchor_scroll_offset = app.copy_scroll_offset;
    app.copy_pos = Some((r, end as u16));
    app.copy_selection_mode = crate::types::SelectionMode::Char;
}

/// Select "a word" (aw) — word under cursor plus trailing whitespace.
pub fn select_a_word(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let seps = app.word_separators.clone();
    let (text, _cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let col = c as usize;
    if col >= bytes.len() { return; }
    let cls = char_class(bytes[col], &seps);
    // Find start of word
    let mut start = col;
    while start > 0 && char_class(bytes[start - 1], &seps) == cls { start -= 1; }
    // Find end of word
    let mut end = col;
    while end + 1 < bytes.len() && char_class(bytes[end + 1], &seps) == cls { end += 1; }
    // Include trailing whitespace
    while end + 1 < bytes.len() && bytes[end + 1].is_whitespace() { end += 1; }
    app.copy_anchor = Some((r, start as u16));
    app.copy_anchor_scroll_offset = app.copy_scroll_offset;
    app.copy_pos = Some((r, end as u16));
    app.copy_selection_mode = crate::types::SelectionMode::Char;
}
