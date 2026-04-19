use super::*;

/// Normalize a key tuple for binding comparison.
/// Strips SHIFT from Char events since the character itself encodes shift information.
/// e.g., '|' already implies Shift was pressed, so (Char('|'), SHIFT) and (Char('|'), NONE) should match.
pub fn normalize_key_for_binding(key: (KeyCode, KeyModifiers)) -> (KeyCode, KeyModifiers) {
    match key.0 {
        KeyCode::Char(_) => (key.0, key.1.difference(KeyModifiers::SHIFT)),
        _ => key,
    }
}
