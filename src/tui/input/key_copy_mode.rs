use super::*;

pub(crate) fn handle_key_copy_mode(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    // Check copy-mode key table for user bindings first (used by plugins like tmux-yank)
    let table_name = if app.mode_keys == "vi" { "copy-mode-vi" } else { "copy-mode" };
    let key_tuple = normalize_key_for_binding((key.code, key.modifiers));
    if let Some(bind) = app.key_tables.get(table_name)
        .and_then(|t| t.iter().find(|b| b.key == key_tuple))
        .cloned()
    {
        return execute_action(app, &bind.action);
    }
    // Handle register pending state (waiting for a-z after ")
    if app.copy_register_pending {
        app.copy_register_pending = false;
        if let KeyCode::Char(ch) = key.code {
            if ch.is_ascii_lowercase() {
                app.copy_register = Some(ch);
            }
        }
        return Ok(false);
    }
    // Handle text-object pending state (waiting for w/W after a/i)
    if let Some(prefix) = app.copy_text_object_pending.take() {
        if let KeyCode::Char(ch) = key.code {
            match (prefix, ch) {
                (0, 'w') => { crate::copy_mode::select_a_word(app); }
                (1, 'w') => { crate::copy_mode::select_inner_word(app); }
                (0, 'W') => { crate::copy_mode::select_a_word_big(app); }
                (1, 'W') => { crate::copy_mode::select_inner_word_big(app); }
                _ => {}
            }
        }
        return Ok(false);
    }
    // Handle find-char pending state (waiting for char after f/F/t/T)
    if let Some(pending) = app.copy_find_char_pending.take() {
        let n = app.copy_count.take().unwrap_or(1);
        if let KeyCode::Char(ch) = key.code {
            match pending {
                0 => { for _ in 0..n { crate::copy_mode::find_char_forward(app, ch); } }
                1 => { for _ in 0..n { crate::copy_mode::find_char_backward(app, ch); } }
                2 => { for _ in 0..n { crate::copy_mode::find_char_to_forward(app, ch); } }
                3 => { for _ in 0..n { crate::copy_mode::find_char_to_backward(app, ch); } }
                _ => {}
            }
        }
        return Ok(false);
    }
    // Handle numeric prefix accumulation for copy-mode motions (vi-style)
    if let KeyCode::Char(d) = key.code {
        if d.is_ascii_digit() && !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
            let digit = d.to_digit(10).unwrap() as usize;
            if let Some(count) = app.copy_count {
                // Accumulate: multiply by 10 and add digit (cap at 9999)
                app.copy_count = Some((count * 10 + digit).min(9999));
                return Ok(false);
            } else if digit >= 1 {
                // Start new count with 1-9
                app.copy_count = Some(digit);
                return Ok(false);
            }
            // digit == 0 with no existing count → fall through to line-start handler
        }
    }
    let copy_repeat = app.copy_count.take().unwrap_or(1);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(']') => { 
            exit_copy_mode(app);
        }
        // Ctrl+C exits copy mode (tmux parity, fixes #25)
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            exit_copy_mode(app);
        }
        KeyCode::Left | KeyCode::Char('h') => { for _ in 0..copy_repeat { move_copy_cursor(app, -1, 0); } }
        KeyCode::Right | KeyCode::Char('l') => { for _ in 0..copy_repeat { move_copy_cursor(app, 1, 0); } }
        KeyCode::Up | KeyCode::Char('k') => { for _ in 0..copy_repeat { move_copy_cursor(app, 0, -1); } }
        KeyCode::Down | KeyCode::Char('j') => { for _ in 0..copy_repeat { move_copy_cursor(app, 0, 1); } }
        // Page scroll: C-b / PageUp = page up, C-f / PageDown = page down
        KeyCode::PageUp => { scroll_copy_up(app, 10); }
        KeyCode::PageDown => { scroll_copy_down(app, 10); }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.mode_keys == "emacs" { move_copy_cursor(app, -1, 0); }
            else { scroll_copy_up(app, 10); }
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.mode_keys == "emacs" { move_copy_cursor(app, 1, 0); }
            else { scroll_copy_down(app, 10); }
        }
        // Half-page scroll: C-u / C-d
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = app.windows.get(app.active_idx)
                .and_then(|w| active_pane(&w.root, &w.active_path))
                .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
            scroll_copy_up(app, half);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = app.windows.get(app.active_idx)
                .and_then(|w| active_pane(&w.root, &w.active_path))
                .map(|p| (p.last_rows / 2) as usize).unwrap_or(10);
            scroll_copy_down(app, half);
        }
        // Emacs copy-mode keys (must be before unqualified char matches)
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => { scroll_copy_down(app, 1); }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => { scroll_copy_up(app, 1); }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => { crate::copy_mode::move_to_line_start(app); }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => { crate::copy_mode::move_to_line_end(app); }
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::ALT) => { scroll_copy_up(app, 10); }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => { crate::copy_mode::move_word_forward(app); }
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => { crate::copy_mode::move_word_backward(app); }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::ALT) => { yank_selection(app)?; exit_copy_mode(app); }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::CopySearch { input: String::new(), forward: true };
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::CopySearch { input: String::new(), forward: false };
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            exit_copy_mode(app);
        }
        KeyCode::Char('g') => { scroll_to_top(app); }
        KeyCode::Char('G') => { scroll_to_bottom(app); }
        // Word motions: w = next word, b = prev word, e = end of word
        KeyCode::Char('w') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_forward(app); } }
        KeyCode::Char('b') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_backward(app); } }
        KeyCode::Char('e') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_end(app); } }
        // WORD motions: W = next WORD, B = prev WORD, E = end WORD
        KeyCode::Char('W') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_forward_big(app); } }
        KeyCode::Char('B') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_backward_big(app); } }
        KeyCode::Char('E') => { for _ in 0..copy_repeat { crate::copy_mode::move_word_end_big(app); } }
        // Screen position: H = top, M = middle, L = bottom
        KeyCode::Char('H') => { crate::copy_mode::move_to_screen_top(app); }
        KeyCode::Char('M') => { crate::copy_mode::move_to_screen_middle(app); }
        KeyCode::Char('L') => { crate::copy_mode::move_to_screen_bottom(app); }
        // Find char: f/F/t/T — sets pending state for next char
        KeyCode::Char('f') => { app.copy_find_char_pending = Some(0); app.copy_count = Some(copy_repeat); }
        KeyCode::Char('F') => { app.copy_find_char_pending = Some(1); app.copy_count = Some(copy_repeat); }
        KeyCode::Char('t') => { app.copy_find_char_pending = Some(2); app.copy_count = Some(copy_repeat); }
        KeyCode::Char('T') => { app.copy_find_char_pending = Some(3); app.copy_count = Some(copy_repeat); }
        // D = copy from cursor to end of line
        KeyCode::Char('D') => { crate::copy_mode::copy_end_of_line(app)?; exit_copy_mode(app); }
        // Bracket matching: % = jump to matching bracket/paren/brace
        KeyCode::Char('%') => { crate::copy_mode::move_matching_bracket(app); }
        // Paragraph jump: { = previous paragraph, } = next paragraph
        KeyCode::Char('{') => { for _ in 0..copy_repeat { crate::copy_mode::move_prev_paragraph(app); } }
        KeyCode::Char('}') => { for _ in 0..copy_repeat { crate::copy_mode::move_next_paragraph(app); } }
        // Line motions: 0 = start, $ = end, ^ = first non-blank
        KeyCode::Char('0') => { crate::copy_mode::move_to_line_start(app); }
        KeyCode::Char('$') => { crate::copy_mode::move_to_line_end(app); }
        KeyCode::Char('^') => { crate::copy_mode::move_to_first_nonblank(app); }
        KeyCode::Home => { crate::copy_mode::move_to_line_start(app); }
        KeyCode::End => { crate::copy_mode::move_to_line_end(app); }
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // vi: toggle rectangle selection, emacs: page down
            if app.mode_keys == "emacs" {
                scroll_copy_down(app, 10);
            } else {
                app.copy_selection_mode = crate::types::SelectionMode::Rect;
            }
        }
        KeyCode::Char('v') => {
            // tmux parity #62: rectangle-toggle (not begin-selection)
            app.copy_selection_mode = match app.copy_selection_mode {
                crate::types::SelectionMode::Rect => crate::types::SelectionMode::Char,
                _ => crate::types::SelectionMode::Rect,
            };
        }
        KeyCode::Char('V') => {
            // Start line-wise selection (vi visual-line mode)
            if let Some((r,c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r,c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r,c));
                app.copy_selection_mode = crate::types::SelectionMode::Line;
            }
        }
        KeyCode::Char('o') => {
            // Swap cursor and anchor
            if let (Some(a), Some(p)) = (app.copy_anchor, app.copy_pos) {
                app.copy_anchor = Some(p);
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some(a);
            }
        }
        KeyCode::Char('A') => {
            // Append to buffer (yank + append to buffer 0)
            if let (Some(_), Some(_)) = (app.copy_anchor, app.copy_pos) {
                // Save current buffer 0
                let prev = app.paste_buffers.first().cloned().unwrap_or_default();
                yank_selection(app)?;
                // buffer 0 is now the new yank; prepend old text
                if let Some(buf) = app.paste_buffers.first_mut() {
                    let new_text = buf.clone();
                    *buf = format!("{}{}", prev, new_text);
                }
                exit_copy_mode(app);
            }
        }
        // Space = begin selection (vi mode), Enter = copy-selection-and-cancel
        KeyCode::Char(' ') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some((r,c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r,c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r,c));
                app.copy_selection_mode = crate::types::SelectionMode::Char;
            }
        }
        KeyCode::Enter => {
            // Copy selection and exit copy mode (vi Enter)
            if app.copy_anchor.is_some() {
                yank_selection(app)?;
            }
            exit_copy_mode(app);
        }
        KeyCode::Char('y') => { yank_selection(app)?; exit_copy_mode(app); }
        // --- copy-mode search ---
        KeyCode::Char('/') => {
            app.mode = Mode::CopySearch { input: String::new(), forward: true };
        }
        KeyCode::Char('?') => {
            app.mode = Mode::CopySearch { input: String::new(), forward: false };
        }
        KeyCode::Char('n') => { search_next(app); }
        KeyCode::Char('N') => { search_prev(app); }
        KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Set mark (anchor)
            if let Some((r, c)) = crate::copy_mode::get_copy_pos(app) {
                app.copy_anchor = Some((r, c));
                app.copy_anchor_scroll_offset = app.copy_scroll_offset;
                app.copy_pos = Some((r, c));
            }
        }
        // Named register prefix: " then a-z
        KeyCode::Char('"') => { app.copy_register_pending = true; }
        // Text-object prefixes: a/i then w/W
        KeyCode::Char('a') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
            app.copy_text_object_pending = Some(0);
        }
        KeyCode::Char('i') if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) => {
            app.copy_text_object_pending = Some(1);
        }
        _ => {}
    }
    Ok(false)
}

pub(crate) fn handle_key_copy_search(app: &mut AppState, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            // Cancel search, return to copy mode
            app.mode = Mode::CopyMode;
        }
        KeyCode::Enter => {
            // Execute search
            if let Mode::CopySearch { ref input, forward } = app.mode {
                let query = input.clone();
                let fwd = forward;
                app.copy_search_query = query.clone();
                app.copy_search_forward = fwd;
                search_copy_mode(app, &query, fwd);
                // Jump to first match
                if !app.copy_search_matches.is_empty() {
                    let (r, c, _) = app.copy_search_matches[0];
                    app.copy_pos = Some((r, c));
                }
            }
            app.mode = Mode::CopyMode;
        }
        KeyCode::Backspace => {
            if let Mode::CopySearch { ref mut input, .. } = app.mode { let _ = input.pop(); }
        }
        KeyCode::Char(c) => {
            if let Mode::CopySearch { ref mut input, .. } = app.mode { input.push(c); }
        }
        _ => {}
    }
    Ok(false)
}
