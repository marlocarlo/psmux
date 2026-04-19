use super::*;

/// Get the current copy-mode cursor position (from copy_pos or screen cursor).
pub fn get_copy_pos(app: &mut AppState) -> Option<(u16, u16)> {
    if let Some(pos) = app.copy_pos { return Some(pos); }
    current_prompt_pos(app)
}

/// Move cursor to first non-blank character (^ key in vi copy mode).
pub fn move_to_first_nonblank(app: &mut AppState) {
    if let Some((r, _)) = get_copy_pos(app) {
        if let Some((text, _)) = read_row_text(app, r) {
            let col = text.find(|c: char| !c.is_whitespace()).unwrap_or(0) as u16;
            app.copy_pos = Some((r, col));
        }
    }
}

/// Classify a character for word boundary detection.
/// Returns: 0 = whitespace, 1 = word char (alnum/_), 2 = punctuation/other
#[inline]
pub(crate) fn char_class(ch: char, seps: &str) -> u8 {
    if ch.is_whitespace() { 0 }
    else if seps.contains(ch) { 2 }
    else if ch.is_alphanumeric() || ch == '_' { 1 }
    else { 2 }
}
