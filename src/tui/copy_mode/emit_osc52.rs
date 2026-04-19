#[allow(unused_imports)]
use std::io::{self, Write};
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL};
#[cfg(windows)]
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData};
#[cfg(windows)]
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

use crate::types::{AppState, Mode, CopyModeState};
use crate::tree::{active_pane, active_pane_mut};

/// Emit an OSC 52 escape sequence to set the terminal clipboard.
/// This works over SSH because the sequence travels through the SSH pipe
/// to the local terminal emulator (e.g. Windows Terminal, iTerm2).
/// The `writer` should be the client's stdout (not the server's).
use super::*;

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

#[cfg(windows)]
pub fn copy_to_system_clipboard(text: &str) {
    const CF_UNICODETEXT: u32 = 13;

    // Clipboard can be momentarily locked by other processes; retry briefly.
    for _ in 0..5 {
        let opened = unsafe { OpenClipboard(std::ptr::null_mut()) };
        if opened == 0 {
            thread::sleep(Duration::from_millis(2));
            continue;
        }

        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        utf16.push(0); // null terminator required by CF_UNICODETEXT
        let size_bytes = utf16.len() * std::mem::size_of::<u16>();
        let mut hmem: HGLOBAL = std::ptr::null_mut();

        unsafe {
            if EmptyClipboard() != 0 {
                hmem = GlobalAlloc(GMEM_MOVEABLE, size_bytes);
                if !hmem.is_null() {
                    let dst = GlobalLock(hmem) as *mut u16;
                    if !dst.is_null() {
                        std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
                        GlobalUnlock(hmem);
                        if !SetClipboardData(CF_UNICODETEXT, hmem).is_null() {
                            // Ownership transferred to the OS on success.
                            hmem = std::ptr::null_mut();
                        }
                    }
                }
            }

            if !hmem.is_null() {
                let _ = GlobalFree(hmem);
            }
            let _ = CloseClipboard();
        }
        break;
    }
}

#[cfg(not(windows))]
pub fn copy_to_system_clipboard(_text: &str) {}

/// Read text from the Windows system clipboard.
#[cfg(windows)]
pub fn read_from_system_clipboard() -> Option<String> {
    const CF_UNICODETEXT: u32 = 13;
    for _ in 0..5 {
        let opened = unsafe { OpenClipboard(std::ptr::null_mut()) };
        if opened == 0 {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let result = unsafe {
            let hmem = GetClipboardData(CF_UNICODETEXT);
            if hmem.is_null() {
                let _ = CloseClipboard();
                return None;
            }
            let ptr = GlobalLock(hmem) as *const u16;
            if ptr.is_null() {
                let _ = CloseClipboard();
                return None;
            }
            // Find null terminator
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
                if len > 1_000_000 { break; } // safety limit
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let text = String::from_utf16_lossy(slice);
            GlobalUnlock(hmem);
            let _ = CloseClipboard();
            // Normalize Windows CRLF to LF — ConPTY expands LF to CRLF on
            // output, so keeping \r\n produces double-spaced text.
            let text = text.replace("\r\n", "\n");
            Some(text)
        };
        return result;
    }
    None
}

#[cfg(not(windows))]
pub fn read_from_system_clipboard() -> Option<String> { None }

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
pub(crate) fn read_row_text(app: &mut AppState, row: u16) -> Option<(String, u16)> {
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
