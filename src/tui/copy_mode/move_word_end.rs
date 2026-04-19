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
pub(crate) fn pipe_text_to_command(text: &str, cmd: &str) {
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
