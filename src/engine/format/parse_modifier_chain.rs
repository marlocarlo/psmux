use super::*;

/// Parse a modifier chain string (e.g. "s|foo|bar|;=5" ) into modifiers.
pub(crate) fn parse_modifier_chain(spec: &str) -> Vec<Modifier> {
    let mut modifiers = Vec::new();
    let parts = split_at_depth0(spec, b';');
    for part in &parts {
        if let Some(m) = parse_single_modifier(part) {
            modifiers.push(m);
        }
    }
    modifiers
}
