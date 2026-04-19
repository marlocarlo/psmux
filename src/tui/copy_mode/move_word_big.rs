use super::*;

/// Move by WORD (whitespace-delimited) forward — W key
pub fn move_word_forward_big(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let (text, cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let mut col = c as usize;
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);
    // Skip non-whitespace
    while col < bytes.len() && !bytes[col].is_whitespace() { col += 1; }
    // Skip whitespace
    while col < bytes.len() && bytes[col].is_whitespace() { col += 1; }
    if col < cols as usize {
        app.copy_pos = Some((r, col as u16));
    } else {
        let nr = (r + 1).min(rows.saturating_sub(1));
        if nr != r {
            if let Some((next_text, _)) = read_row_text(app, nr) {
                let next_bytes: Vec<char> = next_text.chars().collect();
                let mut nc = 0usize;
                while nc < next_bytes.len() && next_bytes[nc].is_whitespace() { nc += 1; }
                app.copy_pos = Some((nr, nc as u16));
            } else { app.copy_pos = Some((nr, 0)); }
        }
    }
}

/// Move by WORD backward — B key
pub fn move_word_backward_big(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let (text, _prev_cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let mut col = c as usize;
    if col == 0 {
        if r > 0 {
            let nr = r - 1;
            if let Some((prev_text, prev_cols)) = read_row_text(app, nr) {
                let prev_bytes: Vec<char> = prev_text.chars().collect();
                let mut nc = (prev_cols as usize).min(prev_bytes.len()).saturating_sub(1);
                while nc > 0 && prev_bytes[nc].is_whitespace() { nc -= 1; }
                while nc > 0 && !prev_bytes[nc - 1].is_whitespace() { nc -= 1; }
                app.copy_pos = Some((nr, nc as u16));
            } else { app.copy_pos = Some((r - 1, 0)); }
        }
        return;
    }
    while col > 0 && bytes[col - 1].is_whitespace() { col -= 1; }
    while col > 0 && !bytes[col - 1].is_whitespace() { col -= 1; }
    app.copy_pos = Some((r, col as u16));
}

/// Move to WORD end — E key
pub fn move_word_end_big(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let (text, cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let mut col = (c as usize) + 1;
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);
    while col < bytes.len() && bytes[col].is_whitespace() { col += 1; }
    while col + 1 < bytes.len() && !bytes[col + 1].is_whitespace() { col += 1; }
    if col < cols as usize {
        app.copy_pos = Some((r, col as u16));
    } else {
        let nr = (r + 1).min(rows.saturating_sub(1));
        if nr != r {
            if let Some((next_text, _)) = read_row_text(app, nr) {
                let next_bytes: Vec<char> = next_text.chars().collect();
                let mut nc = 0usize;
                while nc < next_bytes.len() && next_bytes[nc].is_whitespace() { nc += 1; }
                while nc + 1 < next_bytes.len() && !next_bytes[nc + 1].is_whitespace() { nc += 1; }
                app.copy_pos = Some((nr, nc as u16));
            } else { app.copy_pos = Some((nr, 0)); }
        }
    }
}
