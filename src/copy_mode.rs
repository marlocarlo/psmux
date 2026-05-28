use std::io::{self, Write};

use crate::clipboard::copy_to_system_clipboard;
use crate::types::{AppState, Mode, CopyModeState};
use crate::tree::{active_pane, active_pane_mut};

/// Emit an OSC 52 escape sequence to set the terminal clipboard.
/// This works over SSH because the sequence travels through the SSH pipe
/// to the local terminal emulator (e.g. Windows Terminal, iTerm2).
/// The `writer` should be the client's stdout (not the server's).
pub fn emit_osc52<W: Write>(writer: &mut W, text: &str) {
    let encoded = crate::util::base64_encode(text);
    // OSC 52 ; c ; <base64> ST   (ST = ESC \\ or BEL)
    // Use BEL (\x07) as ST for broadest terminal compatibility.
    let _ = write!(writer, "\x1b]52;c;{}\x07", encoded);
    let _ = writer.flush();
}

pub fn enter_copy_mode(app: &mut AppState) { 
    app.mode = Mode::CopyMode; 
    app.copy_scroll_offset = 0;
    app.copy_selection_mode = crate::types::SelectionMode::Char;
    app.copy_anchor = None;
    // Initialize copy_pos from the terminal cursor so the cursor is
    // visible immediately on entering copy mode (fixes #25).
    app.copy_pos = current_prompt_pos(app);
    app.copy_mouse_down_cell = None;
    app.copy_find_char_pending = None;
    app.copy_text_object_pending = None;
    app.copy_register_pending = false;
    app.copy_register = None;
    app.copy_count = None;
    // Mark the active pane as being in copy mode (pane-local state).
    save_copy_state_to_pane(app);
}

/// Exit copy mode: reset all copy state and scroll the active pane back to
/// live output.  Every copy-mode exit path should call this to avoid leaving
/// a pane scrolled while no longer in copy mode (fixes #43).
pub fn exit_copy_mode(app: &mut AppState) {
    app.mode = Mode::Passthrough;
    app.copy_anchor = None;
    app.copy_pos = None;
    app.copy_mouse_down_cell = None;
    app.copy_scroll_offset = 0;
    let win = &mut app.windows[app.active_idx];
    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
        // Clear the pane-local copy state so re-entering this pane won't
        // restore a stale copy mode.
        p.copy_state = None;
        if let Ok(mut parser) = p.term.lock() {
            parser.screen_mut().set_scrollback(0);
        }
    }
}

/// Save the current global copy-mode state into the active pane.
/// Called whenever we are about to switch away from a pane that is in copy mode.
pub fn save_copy_state_to_pane(app: &mut AppState) {
    let (in_search, search_input, search_input_forward) = match &app.mode {
        Mode::CopySearch { input, forward } => (true, input.clone(), *forward),
        _ => (false, String::new(), true),
    };
    let state = CopyModeState {
        anchor: app.copy_anchor,
        anchor_scroll_offset: app.copy_anchor_scroll_offset,
        pos: app.copy_pos,
        scroll_offset: app.copy_scroll_offset,
        selection_mode: app.copy_selection_mode,
        search_query: app.copy_search_query.clone(),
        count: app.copy_count,
        search_matches: app.copy_search_matches.clone(),
        search_idx: app.copy_search_idx,
        search_forward: app.copy_search_forward,
        find_char_pending: app.copy_find_char_pending,
        text_object_pending: app.copy_text_object_pending,
        register_pending: app.copy_register_pending,
        register: app.copy_register,
        in_search,
        search_input,
        search_input_forward,
    };
    let win = &mut app.windows[app.active_idx];
    if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) {
        p.copy_state = Some(state);
    }
}

/// Restore copy-mode state from the newly-focused pane into the global
/// AppState fields.  If the pane has no saved copy state, set mode to
/// Passthrough.
pub fn restore_copy_state_from_pane(app: &mut AppState) {
    let win = &app.windows[app.active_idx];
    let state = active_pane(&win.root, &win.active_path)
        .and_then(|p| p.copy_state.clone());
    if let Some(s) = state {
        app.copy_anchor = s.anchor;
        app.copy_anchor_scroll_offset = s.anchor_scroll_offset;
        app.copy_pos = s.pos;
        app.copy_scroll_offset = s.scroll_offset;
        app.copy_selection_mode = s.selection_mode;
        app.copy_search_query = s.search_query;
        app.copy_count = s.count;
        app.copy_search_matches = s.search_matches;
        app.copy_search_idx = s.search_idx;
        app.copy_search_forward = s.search_forward;
        app.copy_find_char_pending = s.find_char_pending;
        app.copy_text_object_pending = s.text_object_pending;
        app.copy_register_pending = s.register_pending;
        app.copy_register = s.register;
        if s.in_search {
            app.mode = Mode::CopySearch { input: s.search_input, forward: s.search_input_forward };
        } else {
            app.mode = Mode::CopyMode;
        }
    } else {
        // New pane is not in copy mode — switch to passthrough.
        app.mode = Mode::Passthrough;
    }
}

/// Handle a pane or window focus change: save current copy state if in copy
/// mode, then after the switch, restore the new pane's state.
/// Call the `switch_fn` closure between save and restore to perform the
/// actual focus change.
pub fn switch_with_copy_save<F: FnOnce(&mut AppState)>(app: &mut AppState, switch_fn: F) {
    let was_copy = matches!(app.mode, Mode::CopyMode | Mode::CopySearch { .. });
    if was_copy {
        save_copy_state_to_pane(app);
    }
    switch_fn(app);
    // After switching, check if the new pane has copy state to restore.
    let win = &app.windows[app.active_idx];
    let new_pane_has_copy = active_pane(&win.root, &win.active_path)
        .map_or(false, |p| p.copy_state.is_some());
    if new_pane_has_copy {
        restore_copy_state_from_pane(app);
    } else if was_copy {
        // We were in copy mode but new pane is not — switch to passthrough.
        app.mode = Mode::Passthrough;
    }
}

pub fn current_prompt_pos(app: &mut AppState) -> Option<(u16,u16)> {
    let win = &mut app.windows[app.active_idx];
    let p = active_pane_mut(&mut win.root, &win.active_path)?;
    let parser = p.term.lock().ok()?;
    let (r,c) = parser.screen().cursor_position();
    Some((r,c))
}

pub fn move_copy_cursor(app: &mut AppState, dx: i16, dy: i16) {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let mut parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    // Use tracked copy_pos if available, otherwise fall back to terminal cursor
    let (r, c) = app.copy_pos.unwrap_or_else(|| parser.screen().cursor_position());
    let rows = p.last_rows;
    let cols = p.last_cols;
    let desired_r = r as i16 + dy;
    let nc = (c as i16 + dx).max(0).min(cols as i16 - 1) as u16;
    // If cursor would move above the visible area, scroll up into scrollback
    if desired_r < 0 {
        let scroll_lines = (-desired_r) as usize;
        let current = parser.screen().scrollback();
        parser.screen_mut().set_scrollback(current.saturating_add(scroll_lines));
        app.copy_scroll_offset = parser.screen().scrollback();
        app.copy_pos = Some((0, nc));
    }
    // If cursor would move below the visible area, scroll down (reduce scrollback)
    else if desired_r >= rows as i16 {
        let scroll_lines = (desired_r - rows as i16 + 1) as usize;
        let current = parser.screen().scrollback();
        if current > 0 {
            parser.screen_mut().set_scrollback(current.saturating_sub(scroll_lines));
            app.copy_scroll_offset = parser.screen().scrollback();
            app.copy_pos = Some((rows.saturating_sub(1), nc));
        } else {
            // Already at bottom, clamp
            app.copy_pos = Some((rows.saturating_sub(1), nc));
        }
    } else {
        app.copy_pos = Some((desired_r as u16, nc));
    }
}

/// Helper: read a full row of text from the active pane's screen.
fn read_row_text(app: &mut AppState, row: u16) -> Option<(String, u16)> {
    let win = &mut app.windows[app.active_idx];
    let p = active_pane_mut(&mut win.root, &win.active_path)?;
    let parser = p.term.lock().ok()?;
    let screen = parser.screen();
    let cols = p.last_cols;
    let mut text = String::with_capacity(cols as usize);
    for c in 0..cols {
        if let Some(cell) = screen.cell(row, c) {
            let t = cell.contents();
            if t.is_empty() { text.push(' '); } else { text.push_str(t); }
        } else {
            text.push(' ');
        }
    }
    Some((text, cols))
}

/// Get the current copy-mode cursor position (from copy_pos or screen cursor).
pub fn get_copy_pos(app: &mut AppState) -> Option<(u16, u16)> {
    if let Some(pos) = app.copy_pos { return Some(pos); }
    current_prompt_pos(app)
}

/// Move cursor to start of line (0 key in vi copy mode).
pub fn move_to_line_start(app: &mut AppState) {
    if let Some((r, _)) = get_copy_pos(app) {
        app.copy_pos = Some((r, 0));
    }
}

/// Move cursor to end of line ($ key in vi copy mode).
pub fn move_to_line_end(app: &mut AppState) {
    if let Some((r, _)) = get_copy_pos(app) {
        let win = &app.windows[app.active_idx];
        if let Some(p) = active_pane(&win.root, &win.active_path) {
            let cols = p.last_cols;
            app.copy_pos = Some((r, cols.saturating_sub(1)));
        }
    }
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
fn char_class(ch: char, seps: &str) -> u8 {
    if ch.is_whitespace() { 0 }
    else if seps.contains(ch) { 2 }
    else if ch.is_alphanumeric() || ch == '_' { 1 }
    else { 2 }
}

/// Move cursor to start of next word (w key in vi copy mode).
pub fn move_word_forward(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let seps = app.word_separators.clone();
    let (text, cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let mut col = c as usize;
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);

    // Phase 1: skip current word class
    if col < bytes.len() {
        let cls = char_class(bytes[col], &seps);
        while col < bytes.len() && char_class(bytes[col], &seps) == cls { col += 1; }
    }
    // Phase 2: skip whitespace
    while col < bytes.len() && bytes[col].is_whitespace() { col += 1; }

    if col < cols as usize {
        app.copy_pos = Some((r, col as u16));
    } else {
        // Wrap to next line
        let nr = (r + 1).min(rows.saturating_sub(1));
        if nr != r {
            if let Some((next_text, _)) = read_row_text(app, nr) {
                let next_bytes: Vec<char> = next_text.chars().collect();
                let mut nc = 0usize;
                while nc < next_bytes.len() && next_bytes[nc].is_whitespace() { nc += 1; }
                app.copy_pos = Some((nr, nc as u16));
            } else {
                app.copy_pos = Some((nr, 0));
            }
        }
    }
}

/// Move cursor to start of previous word (b key in vi copy mode).
pub fn move_word_backward(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let seps = app.word_separators.clone();
    let (text, _) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let mut col = c as usize;

    if col == 0 {
        // Wrap to previous line
        if r > 0 {
            let nr = r - 1;
            if let Some((prev_text, prev_cols)) = read_row_text(app, nr) {
                let prev_bytes: Vec<char> = prev_text.chars().collect();
                let mut nc = (prev_cols as usize).min(prev_bytes.len()).saturating_sub(1);
                while nc > 0 && prev_bytes[nc].is_whitespace() { nc -= 1; }
                let cls = char_class(prev_bytes[nc], &seps);
                while nc > 0 && char_class(prev_bytes[nc - 1], &seps) == cls { nc -= 1; }
                app.copy_pos = Some((nr, nc as u16));
            } else {
                app.copy_pos = Some((r - 1, 0));
            }
        }
        return;
    }

    // Phase 1: move left past whitespace
    while col > 0 && bytes[col - 1].is_whitespace() { col -= 1; }
    // Phase 2: move left past current word class
    if col > 0 {
        let cls = char_class(bytes[col - 1], &seps);
        while col > 0 && char_class(bytes[col - 1], &seps) == cls { col -= 1; }
    }
    app.copy_pos = Some((r, col as u16));
}

/// Move cursor to end of current word (e key in vi copy mode).
pub fn move_word_end(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let seps = app.word_separators.clone();
    let (text, cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let mut col = (c as usize) + 1; // start one past current position
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);

    // Skip whitespace
    while col < bytes.len() && bytes[col].is_whitespace() { col += 1; }
    // Find end of word class
    if col < bytes.len() {
        let cls = char_class(bytes[col], &seps);
        while col + 1 < bytes.len() && char_class(bytes[col + 1], &seps) == cls { col += 1; }
    }

    if col < cols as usize {
        app.copy_pos = Some((r, col as u16));
    } else {
        let nr = (r + 1).min(rows.saturating_sub(1));
        if nr != r {
            if let Some((next_text, _)) = read_row_text(app, nr) {
                let next_bytes: Vec<char> = next_text.chars().collect();
                let mut nc = 0usize;
                while nc < next_bytes.len() && next_bytes[nc].is_whitespace() { nc += 1; }
                let cls = if nc < next_bytes.len() { char_class(next_bytes[nc], &seps) } else { 0 };
                while nc + 1 < next_bytes.len() && char_class(next_bytes[nc + 1], &seps) == cls { nc += 1; }
                app.copy_pos = Some((nr, nc as u16));
            } else {
                app.copy_pos = Some((nr, 0));
            }
        }
    }
}

/// Scroll the active pane's scrollback buffer without entering copy mode.
/// Used when scroll-enter-copy-mode is off (#193, credit: @jun2077681).
pub fn scroll_pane_scrollback(app: &mut AppState, lines: usize, up: bool) {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let mut parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    let current = parser.screen().scrollback();
    let new_offset = if up { current.saturating_add(lines) } else { current.saturating_sub(lines) };
    parser.screen_mut().set_scrollback(new_offset);
}

pub fn scroll_copy_up(app: &mut AppState, lines: usize) {
    scroll_pane_scrollback(app, lines, true);
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    app.copy_scroll_offset = parser.screen().scrollback();
}

pub fn scroll_copy_down(app: &mut AppState, lines: usize) {
    scroll_pane_scrollback(app, lines, false);
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    app.copy_scroll_offset = parser.screen().scrollback();
}

pub fn scroll_to_top(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let mut parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    parser.screen_mut().set_scrollback(usize::MAX);
    app.copy_scroll_offset = parser.screen().scrollback();
}

pub fn scroll_to_bottom(app: &mut AppState) {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let mut parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    parser.screen_mut().set_scrollback(0);
    app.copy_scroll_offset = 0;
}

pub fn yank_selection(app: &mut AppState) -> io::Result<()> {
    let (anchor, pos) = match (app.copy_anchor, app.copy_pos) { (Some(a), Some(p)) => (a,p), _ => return Ok(()) };
    let sel_mode = app.copy_selection_mode;
    let anchor_scroll = app.copy_anchor_scroll_offset;
    let current_scroll = app.copy_scroll_offset;
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(()) };
    let mut parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(()) };
    let rows = p.last_rows;
    let cols = p.last_cols;

    // Compute absolute line positions (relative to an arbitrary reference).
    // abs = screen_row - scrollback_at_that_time
    // Higher abs = further down in the terminal buffer (more recent).
    let anchor_abs = anchor.0 as i64 - anchor_scroll as i64;
    let cursor_abs = pos.0 as i64 - current_scroll as i64;
    let sel_top_abs = anchor_abs.min(cursor_abs);
    let sel_bot_abs = anchor_abs.max(cursor_abs);
    let total_lines = (sel_bot_abs - sel_top_abs + 1) as usize;

    // For character mode: determine which endpoint is the "top" (first) line
    let (top_col, bot_col) = if anchor_abs <= cursor_abs {
        (anchor.1, pos.1)
    } else {
        (pos.1, anchor.1)
    };

    // Read all selected rows by adjusting scrollback as needed.
    // At scrollback S, row R shows absolute line (R - S).
    // To read absolute line L: row = L + S, needs 0 <= L + S < rows.
    let mut text = String::new();
    let mut abs_idx: usize = 0; // running index within selection
    let mut next_abs = sel_top_abs;
    while next_abs <= sel_bot_abs {
        // Set scrollback so next_abs maps to row 0 (or as close as possible)
        let target_sb = (-next_abs).max(0) as usize;
        parser.screen_mut().set_scrollback(target_sb);
        let actual_sb = parser.screen().scrollback() as i64;
        let vis_start_abs = -actual_sb;
        let vis_end_abs   = -actual_sb + rows as i64 - 1;
        let read_start = next_abs.max(vis_start_abs);
        let read_end   = sel_bot_abs.min(vis_end_abs);
        if read_start > read_end { break; }

        for aline in read_start..=read_end {
            let r = (aline + actual_sb) as u16;
            let is_first = abs_idx == 0;
            let is_last  = abs_idx + 1 == total_lines;
            match sel_mode {
                crate::types::SelectionMode::Rect => {
                    let c0 = anchor.1.min(pos.1); let c1 = anchor.1.max(pos.1);
                    let mut line = String::new();
                    for c in c0..=c1 {
                        if let Some(cell) = parser.screen().cell(r, c) { line.push_str(&cell.contents().to_string()); } else { line.push(' '); }
                    }
                    text.push_str(line.trim_end());
                    if !is_last { text.push('\n'); }
                }
                crate::types::SelectionMode::Line => {
                    let mut line = String::new();
                    for c in 0..cols {
                        if let Some(cell) = parser.screen().cell(r, c) { line.push_str(&cell.contents().to_string()); } else { line.push(' '); }
                    }
                    text.push_str(line.trim_end());
                    text.push('\n');
                }
                crate::types::SelectionMode::Char => {
                    if total_lines == 1 {
                        let c0 = anchor.1.min(pos.1); let c1 = anchor.1.max(pos.1);
                        for c in c0..=c1 {
                            if let Some(cell) = parser.screen().cell(r, c) { text.push_str(&cell.contents().to_string()); } else { text.push(' '); }
                        }
                    } else {
                        let line_start = if is_first { top_col } else { 0 };
                        let line_end   = if is_last  { bot_col } else { cols.saturating_sub(1) };
                        let mut line = String::new();
                        for c in line_start..=line_end {
                            if let Some(cell) = parser.screen().cell(r, c) { line.push_str(&cell.contents().to_string()); } else { line.push(' '); }
                        }
                        text.push_str(line.trim_end());
                        if !is_last { text.push('\n'); }
                    }
                }
            }
            abs_idx += 1;
        }
        next_abs = read_end + 1;
    }
    // Restore original scrollback
    parser.screen_mut().set_scrollback(current_scroll);
    // Store in named register if one was selected
    if let Some(reg) = app.copy_register.take() {
        app.named_registers.insert(reg, text.clone());
    }
    app.paste_buffers.insert(0, text.clone());
    if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
    copy_to_system_clipboard(&text);
    // Stage text for OSC 52 delivery to the client (works over SSH)
    if app.set_clipboard != "off" {
        app.clipboard_osc52 = Some(text.clone());
    }
    // Pipe to copy-command if configured
    if !app.copy_command.is_empty() {
        let cmd = app.copy_command.clone();
        pipe_text_to_command(&text, &cmd);
    }
    Ok(())
}

/// Pipe text to a shell command's stdin.
fn pipe_text_to_command(text: &str, cmd: &str) {
    let shell = if cfg!(windows) { "pwsh" } else { "sh" };
    let args: Vec<&str> = if cfg!(windows) {
        vec!["-NoProfile", "-Command", cmd]
    } else {
        vec!["-c", cmd]
    };
    if let Ok(mut child) = {
        let mut cmd = std::process::Command::new(shell);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        { use crate::platform::HideWindowCommandExt; cmd.hide_window(); }
        cmd.spawn()
    }
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn paste_latest(app: &mut AppState) -> io::Result<()> {
    // If a named register was selected, paste from it
    if let Some(reg) = app.copy_register.take() {
        if let Some(text) = app.named_registers.get(&reg).cloned() {
            let win = &mut app.windows[app.active_idx];
            if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(p.writer, "{}", text); }
        }
        return Ok(());
    }
    if let Some(buf) = app.paste_buffers.first() {
        let win = &mut app.windows[app.active_idx];
        if let Some(p) = active_pane_mut(&mut win.root, &win.active_path) { let _ = write!(p.writer, "{}", buf); }
    }
    Ok(())
}

pub fn capture_active_pane(app: &mut AppState) -> io::Result<()> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(()) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(()) };
    let screen = parser.screen();
    let mut text = String::new();
    for r in 0..p.last_rows {
        let mut row = String::new();
        for c in 0..p.last_cols { if let Some(cell) = screen.cell(r, c) { row.push_str(&cell.contents().to_string()); } else { row.push(' '); } }
        text.push_str(row.trim_end());
        text.push('\n');
    }
    app.paste_buffers.insert(0, text);
    if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
    Ok(())
}

pub fn capture_active_pane_text(app: &mut AppState) -> io::Result<Option<String>> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(None) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(None) };
    let screen = parser.screen();
    let mut text = String::new();
    for r in 0..p.last_rows {
        let mut row = String::new();
        for c in 0..p.last_cols { if let Some(cell) = screen.cell(r, c) { row.push_str(&cell.contents().to_string()); } else { row.push(' '); } }
        text.push_str(row.trim_end());
        text.push('\n');
    }
    // Trim trailing all-empty lines so iTerm2 doesn't advance its cursor
    // past the actual content on initial attach.
    while text.ends_with("\n\n") { text.pop(); }
    if text == "\n" { text.clear(); }
    Ok(Some(text))
}

pub fn save_latest_buffer(app: &mut AppState, file: &str) -> io::Result<()> {
    if let Some(buf) = app.paste_buffers.first() { std::fs::write(file, buf)?; }
    Ok(())
}

/// Search the active pane's screen content for a query string.
/// Populates `app.copy_search_matches` with (row, col_start, col_end) tuples.
/// If forward is true, sorts matches top-to-bottom; otherwise bottom-to-top.
pub fn search_copy_mode(app: &mut AppState, query: &str, forward: bool) {
    app.copy_search_matches.clear();
    app.copy_search_idx = 0;
    if query.is_empty() { return; }

    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    let screen = parser.screen();
    let query_lower = query.to_lowercase();
    let qlen = query_lower.len() as u16;

    // Scan all visible rows
    for r in 0..p.last_rows {
        // Build the row text
        let mut row_text = String::with_capacity(p.last_cols as usize);
        for c in 0..p.last_cols {
            if let Some(cell) = screen.cell(r, c) {
                let t = cell.contents();
                if t.is_empty() { row_text.push(' '); } else { row_text.push_str(t); }
            } else {
                row_text.push(' ');
            }
        }
        // Case-insensitive search
        let row_lower = row_text.to_lowercase();
        let mut start = 0;
        while let Some(pos) = row_lower[start..].find(&query_lower) {
            let col_start = (start + pos) as u16;
            let col_end = col_start + qlen;
            app.copy_search_matches.push((r, col_start, col_end));
            start += pos + 1;
        }
    }

    if !forward {
        app.copy_search_matches.reverse();
    }
}

/// Jump to the next search match in copy mode.
pub fn search_next(app: &mut AppState) {
    if app.copy_search_matches.is_empty() { return; }
    let wrap = app.user_options.get("wrap-search").map(|v| v.as_str()) != Some("off");
    let next = app.copy_search_idx + 1;
    if next >= app.copy_search_matches.len() {
        if !wrap { return; }
        app.copy_search_idx = 0;
    } else {
        app.copy_search_idx = next;
    }
    let (r, c, _) = app.copy_search_matches[app.copy_search_idx];
    app.copy_pos = Some((r, c));
}

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

/// Move to top of visible screen — H key
pub fn move_to_screen_top(app: &mut AppState) {
    app.copy_pos = Some((0, 0));
}

/// Move to middle of visible screen — M key
pub fn move_to_screen_middle(app: &mut AppState) {
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);
    app.copy_pos = Some((rows / 2, 0));
}

/// Move to bottom of visible screen — L key
pub fn move_to_screen_bottom(app: &mut AppState) {
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);
    app.copy_pos = Some((rows.saturating_sub(1), 0));
}

/// Find character forward on current line — f key
pub fn find_char_forward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (c as usize + 1)..bytes.len() {
            if bytes[i] == ch { app.copy_pos = Some((r, i as u16)); return; }
        }
    }
}

/// Find character backward on current line — F key
pub fn find_char_backward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (0..(c as usize)).rev() {
            if bytes[i] == ch { app.copy_pos = Some((r, i as u16)); return; }
        }
    }
}

/// Find char up to (not including) forward — t key
pub fn find_char_to_forward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (c as usize + 1)..bytes.len() {
            if bytes[i] == ch { app.copy_pos = Some((r, (i as u16).saturating_sub(1))); return; }
        }
    }
}

/// Find char up to (not including) backward — T key
pub fn find_char_to_backward(app: &mut AppState, ch: char) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    if let Some((text, _)) = read_row_text(app, r) {
        let bytes: Vec<char> = text.chars().collect();
        for i in (0..(c as usize)).rev() {
            if bytes[i] == ch { app.copy_pos = Some((r, (i as u16) + 1)); return; }
        }
    }
}

/// Yank from cursor to end of line — D key
pub fn copy_end_of_line(app: &mut AppState) -> io::Result<()> {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return Ok(()) };
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(()) };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(()) };
    let screen = parser.screen();
    let cols = p.last_cols;
    let mut text = String::new();
    for col in c..cols {
        if let Some(cell) = screen.cell(r, col) { text.push_str(&cell.contents().to_string()); } else { text.push(' '); }
    }
    let text = text.trim_end().to_string();
    app.paste_buffers.insert(0, text.clone());
    if app.paste_buffers.len() > 10 { app.paste_buffers.pop(); }
    copy_to_system_clipboard(&text);
    Ok(())
}

/// Jump to the previous search match in copy mode.
pub fn search_prev(app: &mut AppState) {
    if app.copy_search_matches.is_empty() { return; }
    let wrap = app.user_options.get("wrap-search").map(|v| v.as_str()) != Some("off");
    if app.copy_search_idx == 0 {
        if !wrap { return; }
        app.copy_search_idx = app.copy_search_matches.len() - 1;
    } else {
        app.copy_search_idx -= 1;
    }
    let (r, c, _) = app.copy_search_matches[app.copy_search_idx];
    app.copy_pos = Some((r, c));
}

/// Compute the (start, end) row range for capture-pane given optional -S/-E
/// values and the last visible row index.
///
/// Tmux semantics (from cmd-capture-pane.c):
///   Negative -S means "N scrollback lines above visible". Since psmux only
///   exposes visible rows here, any negative start clamps to 0 (top of visible),
///   matching tmux behavior when no scrollback history is available.
///   Negative -E likewise clamps to 0.
pub fn compute_capture_range(s: Option<i32>, e: Option<i32>, last_row: u16) -> (u16, u16) {
    let start = match s {
        Some(v) if v < 0 => 0u16,
        Some(v) => (v as u16).min(last_row),
        None => 0,
    };
    let end = match e {
        Some(v) if v < 0 => 0u16,
        Some(v) => (v as u16).min(last_row),
        None => last_row,
    };
    (start, end)
}

pub fn capture_active_pane_range(app: &mut AppState, s: Option<i32>, e: Option<i32>) -> io::Result<Option<String>> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(None) };
    let mut parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(None) };
    let rows = p.last_rows;
    let cols = p.last_cols;
    let last_row = rows.saturating_sub(1) as i32;

    // If all args are non-negative (or None), use the fast visible-only path
    let needs_scrollback = matches!(s, Some(v) if v < 0);
    if !needs_scrollback {
        let (start, end) = compute_capture_range(s, e, last_row as u16);
        let screen = parser.screen();
        let mut text = String::new();
        for r in start..=end {
            let mut row = String::new();
            for c in 0..cols { if let Some(cell) = screen.cell(r, c) { row.push_str(&cell.contents().to_string()); } else { row.push(' '); } }
            text.push_str(row.trim_end());
            text.push('\n');
        }
        // Trim trailing all-empty lines: prevents iTerm2 from advancing its
        // cursor past the actual content on initial attach, which would
        // otherwise place the first prompt arriving via %output at the
        // bottom of the window instead of the top.
        while text.ends_with("\n\n") { text.pop(); }
        if text == "\n" { text.clear(); }
        return Ok(Some(text));
    }

    // Scrollback-aware capture path.
    // Absolute line numbering: 0 = top of visible (at scrollback 0),
    // negative = lines above visible top (into scrollback history).
    // Determine actual retained scrollback depth.
    let saved_sb = parser.screen().scrollback();
    parser.screen_mut().set_scrollback(usize::MAX);
    let max_sb = parser.screen().scrollback() as i64;
    parser.screen_mut().set_scrollback(saved_sb);

    // Resolve start: i32::MIN means "all history", other negatives are offsets
    let start_abs: i64 = match s {
        Some(v) if v == i32::MIN => -max_sb,
        Some(v) => (v as i64).max(-max_sb),
        None => 0,
    };
    // Resolve end: negative means N lines above visible top, None means last visible row
    let end_abs: i64 = match e {
        Some(v) if v < 0 => (v as i64).max(-max_sb),
        Some(v) => (v as i64).min(last_row as i64),
        None => last_row as i64,
    };

    if start_abs > end_abs { parser.screen_mut().set_scrollback(saved_sb); return Ok(Some(String::new())); }

    // Walk scrollback in batches (same pattern as yank_selection).
    // At scrollback offset S, screen row R shows absolute line (R - S).
    // To read absolute line L, set scrollback so L maps to a visible row.
    let mut text = String::new();
    let mut next_abs = start_abs;
    while next_abs <= end_abs {
        let target_sb = (-next_abs).max(0) as usize;
        parser.screen_mut().set_scrollback(target_sb);
        let actual_sb = parser.screen().scrollback() as i64;
        let vis_start_abs = -actual_sb;
        let vis_end_abs = -actual_sb + rows as i64 - 1;
        let read_start = next_abs.max(vis_start_abs);
        let read_end = end_abs.min(vis_end_abs);
        if read_start > read_end { break; }

        for aline in read_start..=read_end {
            let r = (aline + actual_sb) as u16;
            let mut row = String::new();
            for c in 0..cols {
                if let Some(cell) = parser.screen().cell(r, c) { row.push_str(&cell.contents().to_string()); } else { row.push(' '); }
            }
            text.push_str(row.trim_end());
            text.push('\n');
        }
        next_abs = read_end + 1;
    }

    // Restore original scrollback offset (no side effects on user view)
    parser.screen_mut().set_scrollback(saved_sb);
    // Trim trailing all-empty lines: prevents iTerm2 from advancing its
    // cursor past the actual content on initial attach, which would
    // otherwise place the first prompt arriving via %output at the
    // bottom of the window instead of the top.
    while text.ends_with("\n\n") { text.pop(); }
    if text == "\n" { text.clear(); }
    Ok(Some(text))
}

/// Capture the active pane's screen content with ANSI escape sequences preserved.
/// This is the `-e` flag for capture-pane.  Supports optional start/end range.
/// Negative -S values read from scrollback history; i32::MIN means all retained history.
pub fn capture_active_pane_styled(app: &mut AppState, s: Option<i32>, e: Option<i32>) -> io::Result<Option<String>> {
    let win = &mut app.windows[app.active_idx];
    let p = match active_pane_mut(&mut win.root, &win.active_path) { Some(p) => p, None => return Ok(None) };
    let mut parser = match p.term.lock() { Ok(g) => g, Err(_) => return Ok(None) };
    let rows = p.last_rows;
    let cols = p.last_cols;
    let last_row = rows.saturating_sub(1) as i32;

    // SGR delta tracker state (persists across scrollback batches)
    let mut prev_fg: Option<vt100::Color> = None;
    let mut prev_bg: Option<vt100::Color> = None;
    let mut prev_bold = false;
    let mut prev_dim = false;
    let mut prev_italic = false;
    let mut prev_underline = false;
    let mut prev_blink = false;
    let mut prev_inverse = false;
    let mut prev_hidden = false;
    let mut prev_strikethrough = false;

    // Helper closure: render one screen row with SGR tracking
    let mut render_styled_row = |screen: &vt100::Screen, r: u16, text: &mut String| {
        let mut row_chars: Vec<String> = Vec::new();
        let mut row_sgr: Vec<Option<String>> = Vec::new();
        let mut any_style_active = false;
        for c in 0..cols {
            if let Some(cell) = screen.cell(r, c) {
                let fg = cell.fgcolor();
                let bg = cell.bgcolor();
                let bold = cell.bold();
                let dim = cell.dim();
                let italic = cell.italic();
                let underline = cell.underline();
                let blink = cell.blink();
                let inverse = cell.inverse();
                let hidden = cell.hidden();
                let strikethrough = cell.strikethrough();

                let style_changed = Some(fg) != prev_fg || Some(bg) != prev_bg
                    || bold != prev_bold || dim != prev_dim
                    || italic != prev_italic
                    || underline != prev_underline || blink != prev_blink
                    || inverse != prev_inverse || hidden != prev_hidden
                    || strikethrough != prev_strikethrough;

                let sgr = if style_changed {
                    let mut params = Vec::new();
                    params.push("0".to_string());
                    if bold { params.push("1".to_string()); }
                    if dim { params.push("2".to_string()); }
                    if italic { params.push("3".to_string()); }
                    if underline { params.push("4".to_string()); }
                    if blink { params.push("5".to_string()); }
                    if inverse { params.push("7".to_string()); }
                    if hidden { params.push("8".to_string()); }
                    if strikethrough { params.push("9".to_string()); }
                    match fg {
                        vt100::Color::Default => {}
                        vt100::Color::Idx(n) => {
                            if n < 8 { params.push(format!("{}", 30 + n)); }
                            else if n < 16 { params.push(format!("{}", 90 + n - 8)); }
                            else { params.push(format!("38;5;{}", n)); }
                        }
                        vt100::Color::Rgb(r, g, b) => { params.push(format!("38;2;{};{};{}", r, g, b)); }
                    }
                    match bg {
                        vt100::Color::Default => {}
                        vt100::Color::Idx(n) => {
                            if n < 8 { params.push(format!("{}", 40 + n)); }
                            else if n < 16 { params.push(format!("{}", 100 + n - 8)); }
                            else { params.push(format!("48;5;{}", n)); }
                        }
                        vt100::Color::Rgb(r, g, b) => { params.push(format!("48;2;{};{};{}", r, g, b)); }
                    }
                    prev_fg = Some(fg);
                    prev_bg = Some(bg);
                    prev_bold = bold;
                    prev_dim = dim;
                    prev_italic = italic;
                    prev_underline = underline;
                    prev_blink = blink;
                    prev_inverse = inverse;
                    prev_hidden = hidden;
                    prev_strikethrough = strikethrough;
                    any_style_active = true;
                    Some(format!("\x1b[{}m", params.join(";")))
                } else {
                    None
                };
                row_sgr.push(sgr);
                row_chars.push(cell.contents().to_string());
            } else {
                row_sgr.push(None);
                row_chars.push(" ".to_string());
            }
        }
        let last_non_ws = row_chars.iter().rposition(|s| !s.is_empty() && s.trim() != "");
        let trim_end = match last_non_ws { Some(pos) => pos + 1, None => 0 };
        for c in 0..trim_end {
            if let Some(ref sgr) = row_sgr[c] { text.push_str(sgr); }
            text.push_str(&row_chars[c]);
        }
        if any_style_active {
            text.push_str("\x1b[0m");
            prev_fg = None;
            prev_bg = None;
            prev_bold = false;
            prev_dim = false;
            prev_italic = false;
            prev_underline = false;
            prev_blink = false;
            prev_inverse = false;
            prev_hidden = false;
        }
        text.push('\n');
    };

    // Fast path: no scrollback needed
    let needs_scrollback = matches!(s, Some(v) if v < 0);
    if !needs_scrollback {
        let (start_row, end_row) = compute_capture_range(s, e, last_row as u16);
        let mut text = String::new();
        for r in start_row..=end_row {
            let screen = parser.screen();
            render_styled_row(screen, r, &mut text);
        }
        // Trim trailing all-empty rows — match the behaviour the plain
        // (non-styled) capture path has had for a long time (`while
        // text.ends_with("\n\n") { text.pop(); }` + the iTerm2 comment in
        // `capture_active_pane_text` and `capture_active_pane_range`).
        // The styled path needs its own helper because empty rows that
        // follow a styled row carry an `\x1b[0m` SGR reset between the
        // newlines, so the plain ends_with("\n\n") test misses them.
        // Without the trim, a downstream consumer that writes the snapshot
        // into a terminal (xterm.js for a screen-mirror UI, fresh xterm
        // window, …) leaves the cursor under the visible content — by as
        // many rows as the pane has trailing blanks. Aiball #531 + POC
        // (delta_y/x measured between display-message and xterm.js after
        // term.write) reported the same shift on tmux Linux even though
        // there the wrap at column 80 made it visually less obvious.
        trim_trailing_empty_styled_lines(&mut text);
        return Ok(Some(text));
    }

    // Scrollback-aware styled capture
    let saved_sb = parser.screen().scrollback();
    parser.screen_mut().set_scrollback(usize::MAX);
    let max_sb = parser.screen().scrollback() as i64;
    parser.screen_mut().set_scrollback(saved_sb);

    let start_abs: i64 = match s {
        Some(v) if v == i32::MIN => -max_sb,
        Some(v) => (v as i64).max(-max_sb),
        None => 0,
    };
    let end_abs: i64 = match e {
        Some(v) if v < 0 => (v as i64).max(-max_sb),
        Some(v) => (v as i64).min(last_row as i64),
        None => last_row as i64,
    };

    if start_abs > end_abs { parser.screen_mut().set_scrollback(saved_sb); return Ok(Some(String::new())); }

    let mut text = String::new();
    let mut next_abs = start_abs;
    while next_abs <= end_abs {
        let target_sb = (-next_abs).max(0) as usize;
        parser.screen_mut().set_scrollback(target_sb);
        let actual_sb = parser.screen().scrollback() as i64;
        let vis_start_abs = -actual_sb;
        let vis_end_abs = -actual_sb + rows as i64 - 1;
        let read_start = next_abs.max(vis_start_abs);
        let read_end = end_abs.min(vis_end_abs);
        if read_start > read_end { break; }

        for aline in read_start..=read_end {
            let r = (aline + actual_sb) as u16;
            let screen = parser.screen();
            render_styled_row(screen, r, &mut text);
        }
        next_abs = read_end + 1;
    }

    parser.screen_mut().set_scrollback(saved_sb);
    trim_trailing_empty_styled_lines(&mut text);
    Ok(Some(text))
}

/// Strip trailing all-empty rows from a styled (`-e`) capture buffer.
///
/// A row in the styled output is one "line" terminated by `\n`. An empty
/// row carries either `\n` (no style ever active) or `\x1b[0m\n` (the
/// previous row left a style active and this row reset it). After all
/// trailing empty rows are gone, the buffer ends with the last
/// content-bearing row + its newline (or is empty if there was no
/// content at all). The plain-capture siblings already do this via
/// `while text.ends_with("\n\n") { text.pop(); }` — the styled path
/// needs its own helper because of the SGR resets between newlines.
fn trim_trailing_empty_styled_lines(text: &mut String) {
    loop {
        if !text.ends_with('\n') {
            return;
        }
        let trailing_nl_at = text.len() - 1;
        let line_start = text[..trailing_nl_at].rfind('\n').map_or(0, |i| i + 1);
        let last_line = &text[line_start..trailing_nl_at];
        // The line is "empty" when stripping any leading `\x1b[0m` SGR
        // resets leaves nothing.
        let mut remaining = last_line;
        while let Some(rest) = remaining.strip_prefix("\x1b[0m") {
            remaining = rest;
        }
        if !remaining.is_empty() {
            return; // last line has content — done.
        }
        text.truncate(line_start);
    }
}

#[cfg(test)]
mod trim_trailing_empty_styled_lines_tests {
    use super::trim_trailing_empty_styled_lines;

    fn run(input: &str) -> String {
        let mut s = String::from(input);
        trim_trailing_empty_styled_lines(&mut s);
        s
    }

    #[test]
    fn empty_input_stays_empty() {
        assert_eq!(run(""), "");
    }

    #[test]
    fn no_trailing_newline_untouched() {
        assert_eq!(run("hello"), "hello");
        assert_eq!(run("\x1b[31mred"), "\x1b[31mred");
    }

    #[test]
    fn single_trailing_newline_after_content_kept() {
        assert_eq!(run("hello\n"), "hello\n");
        assert_eq!(run("first\nsecond\n"), "first\nsecond\n");
    }

    #[test]
    fn trailing_blank_lines_collapsed() {
        assert_eq!(run("hello\n\n\n\n"), "hello\n");
    }

    #[test]
    fn trailing_sgr_reset_lines_collapsed() {
        // Each trailing empty row carries an SGR reset before its \n.
        assert_eq!(run("hello\n\x1b[0m\n\x1b[0m\n"), "hello\n");
    }

    #[test]
    fn mixed_trailing_blank_and_reset_lines_collapsed() {
        assert_eq!(run("hello\n\n\x1b[0m\n\n\x1b[0m\n"), "hello\n");
    }

    #[test]
    fn fully_empty_input_truncates() {
        // Only resets + newlines, no content anywhere.
        assert_eq!(run("\n\n\n"), "");
        assert_eq!(run("\x1b[0m\n\x1b[0m\n"), "");
    }

    #[test]
    fn content_with_inline_resets_kept() {
        // SGR reset in the middle of a content row is part of that row, not a
        // trailing-empty marker — must NOT be confused.
        assert_eq!(
            run("\x1b[31mred\x1b[0m text\n\n"),
            "\x1b[31mred\x1b[0m text\n",
        );
    }

    #[test]
    fn multiple_sgr_resets_on_an_empty_line_still_empty() {
        // Pathological but legal: a row that resets and then resets again
        // (some renderers may emit double resets defensively).
        assert_eq!(run("hello\n\x1b[0m\x1b[0m\n"), "hello\n");
    }
}

/// Move to next empty line (paragraph boundary) — } key
pub fn move_next_paragraph(app: &mut AppState) {
    let (r, _) = match get_copy_pos(app) { Some(p) => p, None => return };
    let rows = app.windows.get(app.active_idx)
        .and_then(|w| active_pane(&w.root, &w.active_path))
        .map(|p| p.last_rows).unwrap_or(24);
    // Skip current non-blank lines, then find next blank line
    let mut row = r + 1;
    // Skip non-blank
    while row < rows {
        if let Some((text, _)) = read_row_text(app, row) {
            if text.trim().is_empty() { break; }
        } else { break; }
        row += 1;
    }
    // Skip blank lines to find start of next paragraph
    while row < rows {
        if let Some((text, _)) = read_row_text(app, row) {
            if !text.trim().is_empty() { break; }
        } else { break; }
        row += 1;
    }
    app.copy_pos = Some((row.min(rows.saturating_sub(1)), 0));
}

/// Move to previous empty line (paragraph boundary) — { key
pub fn move_prev_paragraph(app: &mut AppState) {
    let (r, _) = match get_copy_pos(app) { Some(p) => p, None => return };
    if r == 0 { return; }
    let mut row = r.saturating_sub(1);
    // Skip non-blank
    loop {
        if let Some((text, _)) = read_row_text(app, row) {
            if text.trim().is_empty() { break; }
        } else { break; }
        if row == 0 { app.copy_pos = Some((0, 0)); return; }
        row -= 1;
    }
    // Skip blank lines
    loop {
        if let Some((text, _)) = read_row_text(app, row) {
            if !text.trim().is_empty() { break; }
        } else { break; }
        if row == 0 { app.copy_pos = Some((0, 0)); return; }
        row -= 1;
    }
    app.copy_pos = Some((row, 0));
}

/// Move to matching bracket — % key
pub fn move_matching_bracket(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let win = match app.windows.get(app.active_idx) { Some(w) => w, None => return };
    let p = match active_pane(&win.root, &win.active_path) { Some(p) => p, None => return };
    let parser = match p.term.lock() { Ok(g) => g, Err(_) => return };
    let screen = parser.screen();
    
    // Get char at cursor
    let ch = screen.cell(r, c).map(|cell| {
        let t = cell.contents();
        t.chars().next().unwrap_or(' ')
    }).unwrap_or(' ');
    
    let (open, close, forward) = match ch {
        '(' => ('(', ')', true),
        ')' => ('(', ')', false),
        '[' => ('[', ']', true),
        ']' => ('[', ']', false),
        '{' => ('{', '}', true),
        '}' => ('{', '}', false),
        '<' => ('<', '>', true),
        '>' => ('<', '>', false),
        _ => return,
    };
    
    let rows = p.last_rows;
    let cols = p.last_cols;
    let mut depth = 1i32;
    let mut cr = r;
    let mut cc = c;
    
    loop {
        if forward {
            cc += 1;
            if cc >= cols { cc = 0; cr += 1; }
            if cr >= rows { return; }
        } else {
            if cc == 0 {
                if cr == 0 { return; }
                cr -= 1;
                cc = cols.saturating_sub(1);
            } else { cc -= 1; }
        }
        
        let cell_ch = screen.cell(cr, cc).map(|cell| {
            cell.contents().chars().next().unwrap_or(' ')
        }).unwrap_or(' ');
        
        if cell_ch == open { depth += if forward { 1 } else { -1 }; }
        if cell_ch == close { depth += if forward { -1 } else { 1 }; }
        if depth == 0 {
            app.copy_pos = Some((cr, cc));
            return;
        }
    }
}

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

/// Select "inner WORD" (iW) — whitespace-delimited token without surrounding whitespace.
pub fn select_inner_word_big(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let (text, _cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let col = c as usize;
    if col >= bytes.len() { return; }
    if bytes[col].is_whitespace() {
        // Cursor on whitespace — select contiguous whitespace
        let mut start = col;
        while start > 0 && bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    } else {
        // Cursor on non-whitespace — select contiguous non-whitespace
        let mut start = col;
        while start > 0 && !bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && !bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    }
    app.copy_selection_mode = crate::types::SelectionMode::Char;
}

/// Select "a WORD" (aW) — whitespace-delimited token plus trailing whitespace.
pub fn select_a_word_big(app: &mut AppState) {
    let (r, c) = match get_copy_pos(app) { Some(p) => p, None => return };
    let (text, _cols) = match read_row_text(app, r) { Some(t) => t, None => return };
    let bytes: Vec<char> = text.chars().collect();
    let col = c as usize;
    if col >= bytes.len() { return; }
    if bytes[col].is_whitespace() {
        // Cursor on whitespace — select contiguous whitespace
        let mut start = col;
        while start > 0 && bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    } else {
        // Cursor on non-whitespace — select contiguous non-whitespace
        let mut start = col;
        while start > 0 && !bytes[start - 1].is_whitespace() { start -= 1; }
        let mut end = col;
        while end + 1 < bytes.len() && !bytes[end + 1].is_whitespace() { end += 1; }
        // Include trailing whitespace
        while end + 1 < bytes.len() && bytes[end + 1].is_whitespace() { end += 1; }
        app.copy_anchor = Some((r, start as u16));
        app.copy_anchor_scroll_offset = app.copy_scroll_offset;
        app.copy_pos = Some((r, end as u16));
    }
    app.copy_selection_mode = crate::types::SelectionMode::Char;
}
